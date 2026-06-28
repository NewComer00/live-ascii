mod auth;
mod mapping;
mod protocol;
mod server;
mod state;

pub use auth::TokenStore;
pub use mapping::{default_param_by_name, DEFAULT_PARAMS};
pub use state::{new_shared_state, SharedVtsState, VtsMainCommand, VtsSharedState};

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

/// Handle to the background VTS WebSocket server.
#[derive(Debug, Clone)]
pub struct VtsServer {
    pub state: SharedVtsState,
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
        let token_store_thread = Arc::clone(&token_store);
        let state_thread = Arc::clone(&state);
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
            ));
        });

        Self {
            state,
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
                        if let Some((group, index)) = vts
                            .state
                            .lock()
                            .unwrap()
                            .motion_hotkeys
                            .get(&hotkey_id)
                            .cloned()
                        {
                            trigger_motion(context, mm, model_setting, &group, index);
                        } else if !hk.file.is_empty() {
                            trigger_motion_file(context, mm, &hk.file);
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
) {
    if let Some(file) = model_setting.get_motion_file_name(group, index) {
        trigger_motion_file(context, mm, file);
    }
}

fn trigger_motion_file(context: &Context, mm: &mut MotionManager, file: &str) {
    if let Ok(motion_data) = MotionData::from_path(&context.base_dir, file) {
        let motion = CubismMotion::new(motion_data);
        mm.start_motion_priority(motion, true, 2);
    }
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
