//! Input handling — keyboard dispatch (panel navigation, live hotkeys) and
//! mouse handling (drag-to-pan, scroll-to-zoom).  Returns `true` from
//! [`handle_key`] when the render loop should quit.
use crossterm::event::{KeyCode, KeyModifiers, MouseEventKind};

use crate::context::*;
use crate::effect::pose::*;
use crate::model_setting::ModelSetting;
use crate::motion::amotion::*;
use crate::motion::json::*;
use crate::motion::manager::*;
use crate::model::Model;
use crate::utils::*;

/// Drag state for mouse pan: (grab_col, grab_row, offset_x_at_grab, offset_y_at_grab).
pub type DragState = Option<(u16, u16, f32, f32)>;

/// Handle a key press. Returns `true` if the render loop should break (quit).
pub fn handle_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    context: &mut Context,
    mm: &mut MotionManager,
    model_setting: &mut ModelSetting,
    pose: &mut Option<Pose>,
    model: &mut Model,
    offset_x: &mut f32,
    offset_y: &mut f32,
    scale: &mut f32,
) -> bool {
    match context.current_panel {
        Panel::None => match code {
            KeyCode::Char('q') => return true,
            KeyCode::Up => *offset_y -= 0.1,
            KeyCode::Down => *offset_y += 0.1,
            KeyCode::Left => *offset_x -= 0.1,
            KeyCode::Right => *offset_x += 0.1,
            KeyCode::Char('=') | KeyCode::Char('+') => {
                *scale = (*scale + 0.5).min(10.0);
            }
            KeyCode::Char('-') => {
                *scale = (*scale - 0.5).max(0.5);
            }
            _ => {}
        },
        Panel::Op => match context.current_op_panel {
            OpPanel::Motions => match code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    context.current_panel = Panel::None;
                    context.current_op_panel = OpPanel::None;
                }
                KeyCode::Up => context.motion_list_state.select_previous(),
                KeyCode::Down => context.motion_list_state.select_next(),
                KeyCode::Enter => {
                    if let Some(idx) = context.motion_list_state.selected() {
                        let file = model_setting.get_all_motion_names()[idx];
                        if let Ok(motion_data) = MotionData::from_path(&context.base_dir, file) {
                            let motion = CubismMotion::new(motion_data);
                            mm.start_motion_priority(motion, true, 0);
                        } else {
                            context
                                .popups
                                .push_err(&format!("Failed to parse: {}", file));
                        }
                    }
                    if let Some(p) = pose {
                        p.reset(model);
                    }
                }
                _ => {}
            },
            OpPanel::None => {}
        },
        Panel::Debug => match context.current_debug_panel {
            DebugPanel::None => {}
            _ => match code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    context.current_panel = Panel::None;
                    context.current_debug_panel = DebugPanel::None;
                }
                KeyCode::Char('1') => context.current_debug_panel = DebugPanel::Parameters,
                KeyCode::Char('2') => context.current_debug_panel = DebugPanel::PartOpacities,
                KeyCode::Char('3') => context.current_debug_panel = DebugPanel::AppliedExp,
                KeyCode::Char('4') => context.current_debug_panel = DebugPanel::ActionQueue,
                KeyCode::Char('5') => context.current_debug_panel = DebugPanel::Camera,
                KeyCode::Char('6') => context.current_debug_panel = DebugPanel::Manager,
                KeyCode::Up => match context.current_debug_panel {
                    DebugPanel::Camera => {
                        context.camera_offset = context.camera_offset.saturating_sub(1);
                    }
                    DebugPanel::Manager => {
                        context.context_offset = context.context_offset.saturating_sub(1);
                    }
                    _ => context.param_list_state.select_previous(),
                },
                KeyCode::Down => match context.current_debug_panel {
                    DebugPanel::Camera => {
                        context.camera_offset = context.camera_offset.saturating_add(1);
                    }
                    DebugPanel::Manager => {
                        context.context_offset = context.context_offset.saturating_add(1);
                    }
                    _ => context.param_list_state.select_next(),
                },
                _ => {}
            },
        },
    }

    // Live hotkeys always run regardless of panel
    let key_str = key_code_to_str(code);
    let mods = modifiers_to_vec(modifiers);
    if let Some(live) = &context.live_setting {
        live.handle_hotkeys(key_str, mods, &mut context.action_queue);
    }

    false
}

/// Handle a mouse event. Mutates drag state, offset, and scale in place.
pub fn handle_mouse(
    kind: MouseEventKind,
    column: u16,
    row: u16,
    drag: &mut DragState,
    offset_x: &mut f32,
    offset_y: &mut f32,
    scale: &mut f32,
    width: u16,
    height: u16,
) {
    match kind {
        MouseEventKind::Down(_) => {
            *drag = Some((column, row, *offset_x, *offset_y));
        }
        MouseEventKind::Drag(_) => {
            if let Some((gx, gy, init_ox, init_oy)) = *drag {
                let dx = column as i32 - gx as i32;
                let dy = row as i32 - gy as i32;
                *offset_x = init_ox + dx as f32 / width as f32;
                *offset_y = init_oy + dy as f32 / height as f32;
            }
        }
        MouseEventKind::Up(_) => {
            *drag = None;
        }
        MouseEventKind::ScrollUp => {
            *scale = (*scale + 0.5).min(10.0);
        }
        MouseEventKind::ScrollDown => {
            *scale = (*scale - 0.5).max(0.5);
        }
        _ => {}
    }
}
