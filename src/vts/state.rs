use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::live::json::Live;
use crate::model::Model;
use crate::model_setting::ModelSetting;

use super::mapping::{default_param_by_name, vts_value_from_model};

pub const INJECTION_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub struct HotkeyInfo {
    pub hotkey_id: String,
    pub name: String,
    pub hotkey_type: String,
    pub description: String,
    pub file: String,
}

#[derive(Debug, Clone)]
pub enum VtsMainCommand {
    TriggerHotkey { hotkey_id: String },
    SetExpression { file: String, active: bool },
}

/// Motion tracked for `ModelAnimationEvent` push to subscribed clients.
#[derive(Debug, Clone)]
pub struct TrackedAnimation {
    pub queue_id: usize,
    pub animation_name: String,
    pub animation_length: f64,
    pub is_idle_animation: bool,
    pub start_emitted: bool,
    pub end_emitted: bool,
    /// `start_time_seconds` at the current loop cycle (idle motions only).
    pub last_cycle_start: Option<f32>,
}

#[derive(Debug, Clone)]
struct ActiveInjection {
    live2d_id: String,
    value: f32,
    weight: f32,
    mode: InjectionMode,
    owner: Option<String>,
    last_update: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionMode {
    Set,
    Add,
}

#[derive(Debug)]
pub struct ModelSnapshot {
    pub model_name: String,
    pub model_id: String,
    pub live2d_params: Vec<Live2dParamSnapshot>,
    pub vts_default_values: HashMap<String, f32>,
    pub active_expressions: HashSet<String>,
}

#[derive(Debug, Clone)]
pub struct Live2dParamSnapshot {
    pub name: String,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub default_value: f32,
}

#[derive(Debug)]
pub struct VtsSharedState {
    pub model_loaded: bool,
    pub model_name: String,
    pub model_id: String,
    pub hotkeys: Vec<HotkeyInfo>,
    pub expression_files: HashMap<String, String>,
    pub motion_hotkeys: HashMap<String, (String, usize)>,
    pub started_at: Instant,
    pub connected_clients: AtomicUsize,
    pub face_found: bool,
    injections: HashMap<String, ActiveInjection>,
    set_owners: HashMap<String, String>,
    pending_commands: VecDeque<VtsMainCommand>,
    tracked_animations: Vec<TrackedAnimation>,
    snapshot: ModelSnapshot,
}

impl VtsSharedState {
    pub fn from_model_setting(model_name: &str, model_setting: &ModelSetting, live: Option<&Live>) -> Self {
        let model_id = model_name.to_string();
        let mut hotkeys = Vec::new();
        let mut expression_files = HashMap::new();
        let mut motion_hotkeys = HashMap::new();

        for exp in &model_setting.file_references.expressions {
            let id = exp.name.clone();
            expression_files.insert(exp.file.clone(), exp.name.clone());
            hotkeys.push(HotkeyInfo {
                hotkey_id: id.clone(),
                name: exp.name.clone(),
                hotkey_type: "ToggleExpression".into(),
                description: "Toggles an expression".into(),
                file: exp.file.clone(),
            });
        }

        for (group, motions) in &model_setting.file_references.motions {
            for (index, motion) in motions.iter().enumerate() {
                let id = format!("{group}/{index}");
                motion_hotkeys.insert(id.clone(), (group.clone(), index));
                hotkeys.push(HotkeyInfo {
                    hotkey_id: id,
                    name: format!("{group} #{index}"),
                    hotkey_type: "TriggerAnimation".into(),
                    description: "Triggers an animation".into(),
                    file: motion.file.clone(),
                });
            }
        }

        if let Some(live) = live {
            for (i, hk) in live.hotkeys.iter().enumerate() {
                hotkeys.push(HotkeyInfo {
                    hotkey_id: format!("livejson_{i}"),
                    name: format!("{:?}", hk.action),
                    hotkey_type: "Unset".into(),
                    description: "live.json hotkey".into(),
                    file: hk.file.clone(),
                });
            }
        }

        Self {
            model_loaded: true,
            model_name: model_name.to_string(),
            model_id,
            hotkeys,
            expression_files,
            motion_hotkeys,
            started_at: Instant::now(),
            connected_clients: AtomicUsize::new(0),
            face_found: false,
            injections: HashMap::new(),
            set_owners: HashMap::new(),
            pending_commands: VecDeque::new(),
            tracked_animations: Vec::new(),
            snapshot: ModelSnapshot {
                model_name: model_name.to_string(),
                model_id: model_name.to_string(),
                live2d_params: Vec::new(),
                vts_default_values: HashMap::new(),
                active_expressions: HashSet::new(),
            },
        }
    }

    pub fn client_connected(&self) {
        self.connected_clients.fetch_add(1, Ordering::Relaxed);
    }

    pub fn client_disconnected(&self) {
        self.connected_clients.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn uptime_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn inject_parameters(
        &mut self,
        plugin_key: Option<&str>,
        face_found: bool,
        mode: InjectionMode,
        values: &[(String, f32, f32)],
    ) -> Result<(), (i32, String)> {
        self.face_found = face_found;
        let known_live2d: HashSet<String> = self
            .snapshot
            .live2d_params
            .iter()
            .map(|p| p.name.clone())
            .collect();
        for (id, value, weight) in values {
            let Some(live2d_id) = resolve_live2d_param_id_for_inject(id, &known_live2d) else {
                return Err((super::protocol::ERR_PARAM_NOT_FOUND, format!("Parameter not found: {id}")));
            };
            let owner_key = plugin_key.map(str::to_string);
            if mode == InjectionMode::Set {
                if let Some(existing) = self.set_owners.get(&live2d_id) {
                    if owner_key.as_deref() != Some(existing.as_str()) {
                        return Err((
                            super::protocol::ERR_PARAM_OWNED,
                            format!("Parameter {id} is controlled by another plugin"),
                        ));
                    }
                } else if let Some(ref owner) = owner_key {
                    self.set_owners.insert(live2d_id.clone(), owner.clone());
                }
            }

            let entry = self.injections.entry(live2d_id.clone()).or_insert_with(|| {
                ActiveInjection {
                    live2d_id: live2d_id.clone(),
                    value: 0.0,
                    weight: 1.0,
                    mode,
                    owner: owner_key.clone(),
                    last_update: Instant::now(),
                }
            });

            entry.mode = mode;
            entry.last_update = Instant::now();
            entry.weight = *weight;
            match mode {
                InjectionMode::Set => entry.value = *value,
                InjectionMode::Add => entry.value += *value * *weight,
            }
            if mode == InjectionMode::Set {
                entry.owner = owner_key.clone();
            }
        }
        Ok(())
    }

    pub fn expire_stale_injections(&mut self) {
        let now = Instant::now();
        let stale: Vec<String> = self
            .injections
            .iter()
            .filter(|(_, inj)| {
                inj.mode == InjectionMode::Set && now.duration_since(inj.last_update) > INJECTION_TIMEOUT
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in stale {
            self.injections.remove(&key);
            self.set_owners.remove(&key);
        }
    }

    pub fn owned_live2d_params(&self) -> HashSet<String> {
        self.set_owners.keys().cloned().collect()
    }

    pub fn apply_to_model(&self, model: &mut Model) {
        for inj in self.injections.values() {
            model.set_parameter_value_by_id(&inj.live2d_id, inj.value, inj.weight);
        }
    }

    pub fn sync_from_model(&mut self, model: &mut Model, active_expressions: &HashSet<String>) {
        let mut live2d_params = Vec::with_capacity(model.param_count);
        for (i, id) in model.param_ids.iter().enumerate() {
            let (min, max, default) = unsafe {
                (
                    *model.param_min_vs.add(i),
                    *model.param_max_vs.add(i),
                    *model.param_default_vs.add(i),
                )
            };
            live2d_params.push(Live2dParamSnapshot {
                name: id.clone(),
                value: model.get_parameter_value(i),
                min,
                max,
                default_value: default,
            });
        }

        let mut vts_default_values = HashMap::new();
        for spec in super::mapping::DEFAULT_PARAMS {
            if let Some(v) = vts_value_from_model(model, spec.name) {
                vts_default_values.insert(spec.name.to_string(), v);
            }
        }

        self.snapshot = ModelSnapshot {
            model_name: self.model_name.clone(),
            model_id: self.model_id.clone(),
            live2d_params,
            vts_default_values,
            active_expressions: active_expressions.clone(),
        };
    }

    pub fn snapshot(&self) -> &ModelSnapshot {
        &self.snapshot
    }

    pub fn enqueue_command(&mut self, cmd: VtsMainCommand) {
        self.pending_commands.push_back(cmd);
    }

    pub fn drain_commands(&mut self) -> Vec<VtsMainCommand> {
        self.pending_commands.drain(..).collect()
    }

    pub fn track_animation(
        &mut self,
        queue_id: usize,
        animation_name: &str,
        animation_length: f32,
        is_idle_animation: bool,
    ) {
        self.tracked_animations.retain(|t| !t.end_emitted);
        self.tracked_animations.push(TrackedAnimation {
            queue_id,
            animation_name: animation_name.to_string(),
            animation_length: animation_length as f64,
            is_idle_animation,
            start_emitted: false,
            end_emitted: false,
            last_cycle_start: None,
        });
    }

    pub fn tracked_animations_mut(&mut self) -> &mut Vec<TrackedAnimation> {
        &mut self.tracked_animations
    }

    pub fn find_hotkey(&self, hotkey_id: &str) -> Option<&HotkeyInfo> {
        self.hotkeys
            .iter()
            .find(|h| h.hotkey_id.eq_ignore_ascii_case(hotkey_id))
    }

    pub fn find_expression_by_file(&self, file: &str) -> Option<&str> {
        self.expression_files.get(file).map(|s| s.as_str())
    }

    #[cfg(test)]
    fn test_age_injection(&mut self, live2d_id: &str, age: Duration) {
        if let Some(inj) = self.injections.get_mut(live2d_id) {
            inj.last_update = Instant::now() - age;
        }
    }
}

fn resolve_live2d_param_id_for_inject(id: &str, known_live2d: &HashSet<String>) -> Option<String> {
    if let Some(spec) = default_param_by_name(id) {
        return Some(spec.live2d_target.to_string());
    }
    if known_live2d.contains(id) {
        return Some(id.to_string());
    }
    if super::mapping::DEFAULT_PARAMS
        .iter()
        .any(|p| p.live2d_target == id)
    {
        return Some(id.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_setting::ModelSetting;

    #[test]
    fn set_mode_ownership_conflict() {
        let setting = ModelSetting {
            version: 3,
            file_references: Default::default(),
            groups: vec![],
            hit_areas: vec![],
            layout: None,
        };
        let mut state = VtsSharedState::from_model_setting("test", &setting, None);
        state
            .inject_parameters(
                Some("plugin_a"),
                false,
                InjectionMode::Set,
                &[("FaceAngleX".into(), 1.0, 1.0)],
            )
            .unwrap();
        let err = state
            .inject_parameters(
                Some("plugin_b"),
                false,
                InjectionMode::Set,
                &[("FaceAngleX".into(), 2.0, 1.0)],
            )
            .unwrap_err();
        assert_eq!(err.0, super::super::protocol::ERR_PARAM_OWNED);
    }

    #[test]
    fn reject_unknown_param() {
        let setting = ModelSetting {
            version: 3,
            file_references: Default::default(),
            groups: vec![],
            hit_areas: vec![],
            layout: None,
        };
        let mut state = VtsSharedState::from_model_setting("test", &setting, None);
        let err = state
            .inject_parameters(
                Some("plugin_a"),
                false,
                InjectionMode::Set,
                &[("NotARealParam".into(), 1.0, 1.0)],
            )
            .unwrap_err();
        assert_eq!(err.0, super::super::protocol::ERR_PARAM_NOT_FOUND);
    }

    #[test]
    fn stale_injection_expires() {
        let setting = ModelSetting {
            version: 3,
            file_references: Default::default(),
            groups: vec![],
            hit_areas: vec![],
            layout: None,
        };
        let mut state = VtsSharedState::from_model_setting("test", &setting, None);
        state
            .inject_parameters(
                Some("plugin_a"),
                false,
                InjectionMode::Set,
                &[("FaceAngleX".into(), 1.0, 1.0)],
            )
            .unwrap();
        assert!(!state.owned_live2d_params().is_empty());
        state.test_age_injection("ParamAngleX", INJECTION_TIMEOUT + Duration::from_millis(1));
        state.expire_stale_injections();
        assert!(state.owned_live2d_params().is_empty());
    }
}

pub type SharedVtsState = Arc<Mutex<VtsSharedState>>;

pub fn new_shared_state(
    model_name: &str,
    model_setting: &ModelSetting,
    live: Option<&Live>,
) -> SharedVtsState {
    Arc::new(Mutex::new(VtsSharedState::from_model_setting(
        model_name,
        model_setting,
        live,
    )))
}
