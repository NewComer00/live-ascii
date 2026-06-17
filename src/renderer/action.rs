//! Action dispatch — drain the per-frame action queue and execute every
//! `ActionKind` variant (expression toggles, panel open/close, shader
//! switching, receiver control, etc.).
use std::time::Duration;

use ratatui::style::Color;

use crate::context::*;
use crate::effect::pose::*;
use crate::expression::exp::*;
use crate::expression::manager::*;
use crate::live::json::ActionKind;
use crate::model::*;
use crate::motion::manager::*;
use crate::receiver::*;
use crate::shader::*;
use crate::ui::popup::*;

/// Drain and dispatch all queued actions for the current frame.
/// Returns updated (shader, text_chars) since shader changes are driven by actions.
pub fn dispatch(
    context: &mut Context,
    em: &mut ExpressionManager,
    _mm: &mut MotionManager,
    _pose: &mut Option<Pose>,
    _model: &mut Model,
    shader_manager: &mut ShaderManager,
    shader: &mut Shader,
    text_chars: &mut Option<Vec<char>>,
) {
    // Take ownership to avoid borrow conflicts during iteration.
    let actions: Vec<_> = context.action_queue.drain(..).collect();
    for action in &actions {
        match &action.kind {
            ActionKind::SetUnsetExpression(file) => {
                if let Some(&saved_id) = context.active_expressions.get(file) {
                    if let Some(entry) = em.qm.motions.iter_mut().find(|e| e.id == saved_id) {
                        let fade = entry.motion.base().fade_out_seconds;
                        let user_time = em.qm.user_time_seconds;
                        entry.start_fade_out(fade, user_time);
                    }
                    context.active_expressions.remove(file);
                } else {
                    if let Ok(exp) = ExpMotion::from_path(&context.base_dir, file) {
                        let new_id = em.qm.start_motion(exp, false);
                        context.active_expressions.insert(file.clone(), new_id);
                    } else {
                        context
                            .popups
                            .push_err(&format!("Failed to parse: {}", file));
                    }
                }
            }

            ActionKind::OpenCloseMotionPanel => match context.current_panel {
                Panel::Op => {
                    if let OpPanel::Motions = context.current_op_panel {
                        context.current_op_panel = OpPanel::None;
                        context.current_panel = Panel::None;
                    }
                }
                _ => {
                    context.current_op_panel = OpPanel::Motions;
                    context.current_panel = Panel::Op;
                }
            },

            ActionKind::OpenCloseDebugPanel => match context.current_panel {
                Panel::Debug => {
                    context.current_debug_panel = DebugPanel::None;
                    context.current_panel = Panel::None;
                }
                _ => {
                    context.current_debug_panel = DebugPanel::Parameters;
                    context.current_panel = Panel::Debug;
                }
            },

            ActionKind::EnableDisablePhysics => {
                context.use_physics = !context.use_physics;
                if action.show_log {
                    let (text, color) = if context.use_physics {
                        ("Enable physical effects", Color::Rgb(118, 232, 165))
                    } else {
                        ("Disable physical effects", Color::Rgb(235, 129, 129))
                    };
                    context.popups.push(Popup::new(
                        text,
                        Duration::from_secs(3),
                        (text.len() + 3, 3),
                        color,
                    ));
                }
            }

            ActionKind::OpenCloseCamera => {
                context.camera = !context.camera;
                if action.show_log {
                    let (text, color) = if context.camera {
                        ("Start facetracking", Color::Rgb(118, 232, 165))
                    } else {
                        ("Stop facetracking", Color::Rgb(235, 129, 129))
                    };
                    context.popups.push(Popup::new(
                        text,
                        Duration::from_secs(3),
                        (text.len() + 3, 3),
                        color,
                    ));
                    if context.camera {
                        context.tracker.run().unwrap_or_else(|_| {
                            context.popups.push_err("Failed to run tracker.")
                        });
                    }
                }
            }

            ActionKind::NextShader => {
                shader_manager.next();
                update_shader(shader_manager, shader, text_chars);
                if action.show_log {
                    push_shader_popup(context, "Switch to next shader");
                }
            }

            ActionKind::PrevShader => {
                shader_manager.prev();
                update_shader(shader_manager, shader, text_chars);
                if action.show_log {
                    push_shader_popup(context, "Switch to prev shader");
                }
            }

            ActionKind::OpenCloseReceiver(port) => {
                if let Some(r) = &context.receiver {
                    r.stop();
                    context.receiver = None;
                    if action.show_log {
                        let text = "Disable receiver";
                        context.popups.push(Popup::new(
                            text,
                            Duration::from_secs(3),
                            (text.len() + 3, 3),
                            Color::Rgb(235, 129, 129),
                        ));
                    }
                } else if let Some(p) = port {
                    let new_receiver = MsgReceiver::new(*p, context.msg_chan.0.clone());
                    match new_receiver.run() {
                        Ok(_) => {
                            context.receiver = Some(new_receiver);
                            if action.show_log {
                                let text = "Enable receiver";
                                context.popups.push(Popup::new(
                                    text,
                                    Duration::from_secs(3),
                                    (text.len() + 3, 3),
                                    Color::Rgb(118, 232, 165),
                                ));
                            }
                        }
                        Err(_) => context.popups.push_err("Failed to run receiver."),
                    }
                } else if action.show_log {
                    let text = "Failed to get port";
                    context.popups.push(Popup::new(
                        text,
                        Duration::from_secs(3),
                        (text.len() + 3, 3),
                        Color::Rgb(235, 129, 129),
                    ));
                }
            }
        }
    }

    // Done — queue was already drained at the top via drain()
}

fn update_shader(
    shader_manager: &ShaderManager,
    shader: &mut Shader,
    text_chars: &mut Option<Vec<char>>,
) {
    let s = shader_manager.current_shader();
    *shader = s.clone();
    *text_chars = if let Shader::Text(t) = s {
        Some(t.chars().collect())
    } else {
        None
    };
}

fn push_shader_popup(context: &mut Context, text: &'static str) {
    context.popups.push(Popup::new(
        text,
        Duration::from_secs(3),
        (text.len() + 3, 3),
        Color::Rgb(144, 220, 222),
    ));
}
