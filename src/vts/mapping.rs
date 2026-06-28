use std::collections::HashMap;

use crate::model::Model;

/// VTS default tracking parameter metadata.
#[derive(Debug, Clone, Copy)]
pub struct DefaultParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default_value: f32,
    pub live2d_target: &'static str,
}

pub const DEFAULT_PARAMS: &[DefaultParamSpec] = &[
    spec("FaceAngleX", -30.0, 30.0, "ParamAngleX"),
    spec("FaceAngleY", -30.0, 30.0, "ParamAngleY"),
    spec("FaceAngleZ", -30.0, 30.0, "ParamAngleZ"),
    spec("FacePositionX", -10.0, 10.0, "ParamBodyAngleX"),
    spec("FacePositionY", -10.0, 10.0, "ParamBodyAngleY"),
    spec("FacePositionZ", -10.0, 10.0, "ParamBodyAngleZ"),
    spec("MouthOpen", 0.0, 1.0, "ParamMouthOpenY"),
    spec("MouthSmile", -1.0, 1.0, "ParamMouthForm"),
    spec("MouthForm", -1.0, 1.0, "ParamMouthForm"),
    spec("EyeOpenLeft", 0.0, 1.0, "ParamEyeLOpen"),
    spec("EyeOpenRight", 0.0, 1.0, "ParamEyeROpen"),
    spec("EyeLeftX", -1.0, 1.0, "ParamEyeBallX"),
    spec("EyeLeftY", -1.0, 1.0, "ParamEyeBallY"),
    spec("EyeRightX", -1.0, 1.0, "ParamEyeBallX"),
    spec("EyeRightY", -1.0, 1.0, "ParamEyeBallY"),
    spec("BrowLeftY", -1.0, 1.0, "ParamBrowLY"),
    spec("BrowRightY", -1.0, 1.0, "ParamBrowRY"),
    spec("CheekPuff", 0.0, 1.0, "ParamCheekPuff"),
    spec("TongueOut", 0.0, 1.0, "ParamTongueOut"),
];

const fn spec(name: &'static str, min: f32, max: f32, live2d: &'static str) -> DefaultParamSpec {
    DefaultParamSpec {
        name,
        min,
        max,
        default_value: 0.0,
        live2d_target: live2d,
    }
}

#[allow(dead_code)]
pub fn vts_to_live2d_map() -> HashMap<&'static str, &'static str> {
    DEFAULT_PARAMS
        .iter()
        .map(|p| (p.name, p.live2d_target))
        .collect()
}

#[allow(dead_code)]
pub fn live2d_to_vts_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    for p in DEFAULT_PARAMS {
        map.entry(p.live2d_target).or_insert(p.name);
    }
    map
}

pub fn default_param_by_name(name: &str) -> Option<&'static DefaultParamSpec> {
    DEFAULT_PARAMS.iter().find(|p| p.name == name)
}

/// Resolve an injection/query id to the Live2D parameter id used on the model.
#[allow(dead_code)]
pub fn resolve_live2d_param_id(model: &Model, id: &str) -> Option<String> {
    if model.param_id_to_index.contains_key(id) {
        return Some(id.to_string());
    }
    if let Some(spec) = default_param_by_name(id) {
        if model.param_id_to_index.contains_key(spec.live2d_target) {
            return Some(spec.live2d_target.to_string());
        }
        // Fall back to mapped name even if not in model (virtual param slot).
        return Some(spec.live2d_target.to_string());
    }
    None
}

pub fn vts_value_from_model(model: &mut Model, vts_name: &str) -> Option<f32> {
    if let Some(spec) = default_param_by_name(vts_name) {
        Some(model.get_parameter_value_by_id(spec.live2d_target))
    } else if model.param_id_to_index.contains_key(vts_name) {
        Some(model.get_parameter_value_by_id(vts_name))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_have_unique_names() {
        let mut seen = std::collections::HashSet::new();
        for p in DEFAULT_PARAMS {
            assert!(seen.insert(p.name), "duplicate {}", p.name);
        }
    }

    #[test]
    fn lookup_face_angle_x() {
        let spec = default_param_by_name("FaceAngleX").unwrap();
        assert_eq!(spec.live2d_target, "ParamAngleX");
    }
}
