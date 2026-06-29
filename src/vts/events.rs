use serde_json::{json, Value};

use super::protocol::{request_id_or_uuid, ResponseEnvelope};

/// Config for `ModelAnimationEvent` subscriptions.
#[derive(Debug, Clone, Default)]
pub struct ModelAnimationConfig {
    pub ignore_live2d_items: bool,
    pub ignore_idle_animations: bool,
}

/// Per-connection event subscriptions.
#[derive(Debug, Clone, Default)]
pub struct Subscriptions {
    pub model_animation: Option<ModelAnimationConfig>,
}

impl Subscriptions {
    pub fn subscribed_event_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.model_animation.is_some() {
            names.push("ModelAnimationEvent");
        }
        names
    }
}

pub fn parse_model_animation_config(data: &Value) -> ModelAnimationConfig {
    let config = data.get("config").unwrap_or(data);
    ModelAnimationConfig {
        ignore_live2d_items: config
            .get("ignoreLive2DItems")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        ignore_idle_animations: config
            .get("ignoreIdleAnimations")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    }
}

pub fn model_animation_event_json(
    event_type: &str,
    animation_name: &str,
    animation_length: f64,
    is_idle_animation: bool,
    model_id: &str,
    model_name: &str,
) -> String {
    ResponseEnvelope::new(
        None,
        "ModelAnimationEvent",
        json!({
            "animationEventType": event_type,
            "animationEventTime": 0.0,
            "animationEventData": "",
            "animationName": animation_name,
            "animationLength": animation_length,
            "isIdleAnimation": is_idle_animation,
            "modelID": model_id,
            "modelName": model_name,
            "isLive2DItem": false,
        }),
    )
    .to_json()
}

pub fn event_subscription_response(request_id: Option<String>, subs: &Subscriptions) -> String {
    let names: Vec<&str> = subs.subscribed_event_names();
    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "EventSubscriptionResponse",
        json!({
            "subscribedEventCount": names.len(),
            "subscribedEvents": names,
        }),
    )
    .to_json()
}
