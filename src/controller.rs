use std::collections::HashMap;

use crate::model::Model;
use crate::tracker::Packet;
use glam::{EulerRot, Quat}; 

fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

fn clamp(v: f32, min: f32, max: f32) -> f32 {
    v.max(min).min(max)
}

fn clamp01(v: f32) -> f32 {
    clamp(v, 0.0, 1.0)
}

pub struct FaceController {
    current_values: HashMap<String, f32>,
    smooth_factor: f32,
}

impl FaceController {
    pub fn new(smooth_factor: f32) -> Self {
        Self {
            current_values: HashMap::new(),
            smooth_factor: clamp(smooth_factor, 0.0, 1.0),
        }
    }

    fn set_param_smoothed(&mut self, model: &mut Model, param_id: &str, target_value: f32) {
        let current = self
            .current_values
            .entry(param_id.to_string())
            .or_insert(target_value);

        *current = lerp(*current, target_value, self.smooth_factor);
        model.set_parameter_value_by_id(param_id, *current, 1.0);
    }

    pub fn update_parameters(&mut self, model: &mut Model, packet: &Packet) {
        self.update_parameters_except(model, packet, &std::collections::HashSet::new());
    }

    pub fn update_parameters_except(
        &mut self,
        model: &mut Model,
        packet: &Packet,
        skip_params: &std::collections::HashSet<String>,
    ) {
        const HEAD_X_GAIN: f32 = 40.0; 
        const HEAD_Y_GAIN: f32 = 40.0; 
        const HEAD_Z_GAIN: f32 = 40.0; 

        const BODY_X_GAIN: f32 = 10.0;
        const BODY_Y_GAIN: f32 = 10.0;
        const BODY_Z_GAIN: f32 = 10.0;

        const EYE_BALL_GAIN: f32 = 0.60;
        const MOUTH_OPEN_GAIN: f32 = 1.50;
        const MOUTH_FORM_GAIN: f32 = 2.00;

        let raw_quat = Quat::from_xyzw(
            packet.quaternion[0],
            -packet.quaternion[1],
            -packet.quaternion[2],
            packet.quaternion[3],
        );

        let correction = Quat::from_euler(
            EulerRot::XYZ,
            std::f32::consts::PI,
            0.0,
            std::f32::consts::PI / 2.0,
        );

        let final_quat = raw_quat.mul_quat(correction);

        let (pitch, yaw, roll) = final_quat.to_euler(EulerRot::XYZ);

        let _left_eye = packet.lms[36];
        let _right_eye = packet.lms[45];

        self.set_param_smoothed_if(model, skip_params, "ParamAngleX", yaw * HEAD_X_GAIN);
        self.set_param_smoothed_if(model, skip_params, "ParamAngleY", pitch * HEAD_Y_GAIN);
        self.set_param_smoothed_if(model, skip_params, "ParamAngleZ", roll * HEAD_Z_GAIN);

        self.set_param_smoothed_if(model, skip_params, "ParamBodyAngleX", yaw * BODY_X_GAIN);
        self.set_param_smoothed_if(model, skip_params, "ParamBodyAngleY", pitch * BODY_Y_GAIN);
        self.set_param_smoothed_if(model, skip_params, "ParamBodyAngleZ", roll * BODY_Z_GAIN);

        // Eye
        self.set_param_smoothed_if(model, skip_params, "ParamEyeLOpen", clamp01(packet.eye_blink_left));
        self.set_param_smoothed_if(model, skip_params, "ParamEyeROpen", clamp01(packet.eye_blink_right));

        // EyeBall
        self.set_param_smoothed_if(model, skip_params, "ParamEyeBallX", yaw * EYE_BALL_GAIN);
        self.set_param_smoothed_if(model, skip_params, "ParamEyeBallY", pitch * EYE_BALL_GAIN);

        // Mouth
        let mouth_open_y = clamp(packet.mouth_open * MOUTH_OPEN_GAIN, 0.0, 1.2);
        let mouth_form = clamp(packet.mouth_wide * MOUTH_FORM_GAIN - 1.0, -1.0, 1.0);

        self.set_param_smoothed_if(model, skip_params, "ParamMouthOpenY", mouth_open_y);
        self.set_param_smoothed_if(model, skip_params, "ParamMouthForm", mouth_form);

        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamMouthCornerLeft",
            clamp(packet.mouth_corner_updown_left * 1.5, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamMouthCornerRight",
            clamp(packet.mouth_corner_updown_right * 1.5, -1.0, 1.0),
        );

        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamMouthCornerInOutLeft",
            clamp(packet.mouth_corner_inout_left * 1.5, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamMouthCornerInOutRight",
            clamp(packet.mouth_corner_inout_right * 1.5, -1.0, 1.0),
        );

        // Eye Details
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeSteepnessLeft",
            clamp(packet.eye_steepness_left, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeUpDownLeft",
            clamp(packet.eye_up_down_left, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeQuirkLeft",
            clamp(packet.eye_quirk_left, -1.0, 1.0),
        );

        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeSteepnessRight",
            clamp(packet.eye_steepness_right, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeUpDownRight",
            clamp(packet.eye_up_down_right, -1.0, 1.0),
        );
        self.set_param_smoothed_if(
            model,
            skip_params,
            "ParamEyeQuirkRight",
            clamp(packet.eye_quirk_right, -1.0, 1.0),
        );

        self.set_param_smoothed_if(model, skip_params, "ParamEyeL", clamp01(packet.eye_left));
        self.set_param_smoothed_if(model, skip_params, "ParamEyeR", clamp01(packet.eye_right));
    }

    fn set_param_smoothed_if(
        &mut self,
        model: &mut Model,
        skip_params: &std::collections::HashSet<String>,
        param_id: &str,
        target_value: f32,
    ) {
        if skip_params.contains(param_id) {
            return;
        }
        self.set_param_smoothed(model, param_id, target_value);
    }
}
