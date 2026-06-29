mod auth;
mod events;
mod hub;
mod mapping;
mod protocol;
mod server;
mod state;

pub use auth::TokenStore;
pub use mapping::{default_param_by_name, DEFAULT_PARAMS};
pub use state::{
    new_shared_state, SharedVtsState, TrackedAnimation, VtsMainCommand, VtsSharedState,
};

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;

use crate::context::Context;
use crate::controller::FaceController;
use crate::expression::exp::ExpMotion;
use crate::expression::manager::ExpressionManager;
use crate::model::Model;
use crate::model_setting::ModelSetting;
use crate::motion::amotion::CubismMotion;
use crate::motion::json::MotionData;
use crate::motion::manager::MotionManager;

use self::auth::TokenStore as AuthTokenStore;
pub use self::hub::EventHub;

/// Handle to the background VTS WebSocket server.
#[derive(Debug, Clone)]
pub struct VtsServer {
    pub state: SharedVtsState,
    event_hub: Arc<EventHub>,
    _token_store: Arc<Mutex<AuthTokenStore>>,
}

#[derive(Debug, Clone)]
pub struct VtsConfig {
    pub port: u16,
    pub auto_approve: bool,
    pub model_name: String,
}

impl VtsServer {
    pub fn start(
        config: VtsConfig,
        model_setting: &ModelSetting,
        live: Option<&crate::live::json::Live>,
    ) -> Self {
        let state = new_shared_state(&config.model_name, model_setting, live);
        let token_store = Arc::new(Mutex::new(AuthTokenStore::load_default()));
        let event_hub = Arc::new(EventHub::default());
        let token_store_thread = Arc::clone(&token_store);
        let state_thread = Arc::clone(&state);
        let hub_thread = Arc::clone(&event_hub);
        let port = config.port;
        let auto_approve = config.auto_approve;

        thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("VTS tokio runtime");
            rt.block_on(server::run_server(
                port,
                auto_approve,
                state_thread,
                token_store_thread,
                hub_thread,
            ));
        });

        Self {
            state,
            event_hub,
            _token_store: token_store,
        }
    }

    /// Expire stale injections, apply active values, and sync parameter snapshots.
    pub fn tick(&self, model: &mut Model, active_expressions: &HashSet<String>) {
        let mut shared = self.state.lock().unwrap();
        shared.expire_stale_injections();
        shared.apply_to_model(model);
        shared.sync_from_model(model, active_expressions);
    }

    pub fn owned_live2d_params(&self) -> HashSet<String> {
        self.state.lock().unwrap().owned_live2d_params()
    }

    /// Emit `ModelAnimationEvent` Start/End for tracked motions (call after `mm.update_motion`).
    pub fn poll_animation_events(&self, mm: &MotionManager) {
        let mut emissions: Vec<(String, String, f64, bool)> = Vec::new();
        let (model_id, model_name) = {
            let mut shared = self.state.lock().unwrap();
            let active_queue_ids: HashSet<usize> =
                mm.qm.motions.iter().map(|e| e.id).collect();

            for tracked in shared.tracked_animations_mut().iter_mut() {
                if !active_queue_ids.contains(&tracked.queue_id) {
                    finalize_tracked_animation(tracked, &mut emissions);
                    continue;
                }

                let Some(e) = mm.qm.motions.iter().find(|e| e.id == tracked.queue_id) else {
                    continue;
                };

                if tracked.is_idle_animation {
                    if !e.started {
                        continue;
                    }
                    let cycle_start = e.start_time_seconds;
                    let len = tracked.animation_length;
                    let name = tracked.animation_name.clone();
                    let idle = tracked.is_idle_animation;

                    if !tracked.start_emitted {
                        tracked.start_emitted = true;
                        tracked.last_cycle_start = Some(cycle_start);
                        emissions.push(("Start".into(), name.clone(), len, idle));
                    } else if tracked
                        .last_cycle_start
                        .is_some_and(|prev| (cycle_start - prev).abs() > 0.0001)
                    {
                        emissions.push(("End".into(), name.clone(), len, idle));
                        emissions.push(("Start".into(), name, len, idle));
                        tracked.last_cycle_start = Some(cycle_start);
                    }
                } else {
                    if e.started && !tracked.start_emitted {
                        tracked.start_emitted = true;
                        emissions.push((
                            "Start".to_string(),
                            tracked.animation_name.clone(),
                            tracked.animation_length,
                            tracked.is_idle_animation,
                        ));
                    }
                    let elapsed = if e.started && e.start_time_seconds >= 0.0 {
                        mm.qm.user_time_seconds - e.start_time_seconds
                    } else {
                        0.0
                    };
                    let completed =
                        e.finished || elapsed >= tracked.animation_length as f32;
                    if completed && !tracked.end_emitted {
                        tracked.end_emitted = true;
                        emissions.push((
                            "End".to_string(),
                            tracked.animation_name.clone(),
                            tracked.animation_length,
                            tracked.is_idle_animation,
                        ));
                    }
                }
            }
            shared.tracked_animations_mut().retain(|t| !t.end_emitted);
            (
                shared.model_id.clone(),
                shared.model_name.clone(),
            )
        };

        for (event_type, animation_name, animation_length, is_idle) in emissions {
            self.event_hub.broadcast_model_animation(
                &model_id,
                &model_name,
                &event_type,
                &animation_name,
                animation_length,
                is_idle,
            );
        }
    }
}

/// Mark tracking complete and emit a final `End` when the motion left the queue.
fn finalize_tracked_animation(
    tracked: &mut TrackedAnimation,
    emissions: &mut Vec<(String, String, f64, bool)>,
) {
    if tracked.end_emitted {
        return;
    }
    tracked.end_emitted = true;
    if tracked.start_emitted {
        emissions.push((
            "End".to_string(),
            tracked.animation_name.clone(),
            tracked.animation_length,
            tracked.is_idle_animation,
        ));
    }
}

/// Drain VTS hotkey/expression commands and execute on the main thread.
pub fn process_commands(
    vts: &VtsServer,
    context: &mut Context,
    em: &mut ExpressionManager,
    mm: &mut MotionManager,
    model_setting: &ModelSetting,
) {
    let commands = {
        let mut shared = vts.state.lock().unwrap();
        shared.drain_commands()
    };

    for cmd in commands {
        match cmd {
            VtsMainCommand::TriggerHotkey { hotkey_id } => {
                let hotkey = {
                    let shared = vts.state.lock().unwrap();
                    shared.find_hotkey(&hotkey_id).cloned()
                };
                let Some(hk) = hotkey else { continue };

                match hk.hotkey_type.as_str() {
                    "ToggleExpression" => {
                        toggle_expression(context, em, &hk.file);
                    }
                    "TriggerAnimation" => {
                        let tracked = if let Some((group, index)) = vts
                            .state
                            .lock()
                            .unwrap()
                            .motion_hotkeys
                            .get(&hotkey_id)
                            .cloned()
                        {
                            trigger_motion(context, mm, model_setting, &group, index)
                        } else if !hk.file.is_empty() {
                            trigger_motion_file(context, mm, &hk.file, false)
                        } else {
                            None
                        };
                        if let Some((queue_id, duration, file, is_idle)) = tracked {
                            vts.state.lock().unwrap().track_animation(
                                queue_id,
                                &file,
                                duration,
                                is_idle,
                            );
                        }
                    }
                    _ => {}
                }
            }
            VtsMainCommand::SetExpression { file, active } => {
                if active {
                    activate_expression(context, em, &file);
                } else {
                    deactivate_expression(context, em, &file);
                }
            }
        }
    }
}

fn toggle_expression(context: &mut Context, em: &mut ExpressionManager, file: &str) {
    if context.active_expressions.contains_key(file) {
        deactivate_expression(context, em, file);
    } else {
        activate_expression(context, em, file);
    }
}

fn activate_expression(context: &mut Context, em: &mut ExpressionManager, file: &str) {
    if context.active_expressions.contains_key(file) {
        return;
    }
    if let Ok(exp) = ExpMotion::from_path(&context.base_dir, file) {
        let new_id = em.qm.start_motion(exp, false);
        context.active_expressions.insert(file.to_string(), new_id);
    }
}

fn deactivate_expression(context: &mut Context, em: &mut ExpressionManager, file: &str) {
    if let Some(&saved_id) = context.active_expressions.get(file) {
        if let Some(entry) = em.qm.motions.iter_mut().find(|e| e.id == saved_id) {
            let fade = entry.motion.base().fade_out_seconds;
            let user_time = em.qm.user_time_seconds;
            entry.start_fade_out(fade, user_time);
        }
        context.active_expressions.remove(file);
    }
}

fn trigger_motion(
    context: &Context,
    mm: &mut MotionManager,
    model_setting: &ModelSetting,
    group: &str,
    index: usize,
) -> Option<(usize, f32, String, bool)> {
    model_setting
        .get_motion_file_name(group, index)
        .and_then(|file| trigger_motion_file(context, mm, file, group == "Idle"))
}

fn trigger_motion_file(
    context: &Context,
    mm: &mut MotionManager,
    file: &str,
    is_idle: bool,
) -> Option<(usize, f32, String, bool)> {
    let motion_data = MotionData::from_path(&context.base_dir, file).ok()?;
    let duration = motion_data.duration;
    let mut motion = CubismMotion::new(motion_data);
    if !is_idle {
        // Non-idle hotkeys are one-shot in VTS even when the .motion3 has Loop: true.
        // Leaving is_loop true would restart the motion forever and never emit ModelAnimationEvent End.
        motion.base.is_loop = false;
    }
    let queue_id = mm.start_motion_priority(motion, true, 2);
    Some((queue_id, duration, file.to_string(), is_idle))
}

/// Update face tracking while skipping parameters owned by VTS.
pub fn update_face_tracking(
    face_controller: &mut FaceController,
    model: &mut Model,
    packet: &crate::tracker::Packet,
    skip_params: &HashSet<String>,
) {
    face_controller.update_parameters_except(model, packet, skip_params);
}

#[cfg(test)]
mod poll_tests {
    use super::*;
    use crate::model_setting::ModelSetting;
    use crate::motion::json::MotionData;

    fn test_server() -> VtsServer {
        let setting = ModelSetting {
            version: 3,
            file_references: Default::default(),
            groups: vec![],
            hit_areas: vec![],
            layout: None,
        };
        VtsServer {
            state: new_shared_state("test", &setting, None),
            event_hub: Arc::new(EventHub::default()),
            _token_store: Arc::new(Mutex::new(AuthTokenStore::load_default())),
        }
    }

    fn idle_motion_data() -> MotionData {
        MotionData {
            duration: 1.0,
            loop_: true,
            fps: 30.0,
            curves: vec![],
            segments: vec![],
            points: vec![],
            events: vec![],
        }
    }

    #[test]
    fn idle_tracked_animation_removed_when_motion_leaves_queue() {
        let vts = test_server();
        vts.state
            .lock()
            .unwrap()
            .track_animation(42, "idle.motion3.json", 1.0, true);
        vts.state.lock().unwrap().tracked_animations_mut()[0].start_emitted = true;

        let mm = MotionManager::new();
        vts.poll_animation_events(&mm);
        assert!(
            vts.state.lock().unwrap().tracked_animations_mut().is_empty(),
            "stale idle tracker should be removed when queue entry is gone"
        );
    }

    #[test]
    fn many_stale_idle_trackers_do_not_accumulate() {
        let vts = test_server();
        for id in 0..20 {
            vts.state
                .lock()
                .unwrap()
                .track_animation(id, "idle.motion3.json", 1.0, true);
        }

        let mm = MotionManager::new();
        vts.poll_animation_events(&mm);
        assert_eq!(vts.state.lock().unwrap().tracked_animations_mut().len(), 0);
    }

    #[test]
    fn active_idle_motion_stays_tracked() {
        let vts = test_server();
        let mut mm = MotionManager::new();
        let queue_id =
            mm.start_motion_priority(CubismMotion::new(idle_motion_data()), true, 2);
        vts.state.lock().unwrap().track_animation(
            queue_id,
            "idle.motion3.json",
            1.0,
            true,
        );
        mm.qm.motions[0].started = true;
        mm.qm.motions[0].start_time_seconds = 0.0;

        vts.poll_animation_events(&mm);
        assert_eq!(vts.state.lock().unwrap().tracked_animations_mut().len(), 1);
    }
}
