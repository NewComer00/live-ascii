use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const API_NAME: &str = "VTubeStudioPublicAPI";
pub const API_VERSION: &str = "1.0";
pub const VTS_VERSION: &str = "live-ascii 0.1.0";

pub const ERR_REQUIRES_AUTH: i32 = 8;
pub const ERR_INVALID_REQUEST: i32 = 100;
pub const ERR_TOKEN_DENIED: i32 = 50;
pub const ERR_PARAM_NOT_FOUND: i32 = 353;
pub const ERR_PARAM_OWNED: i32 = 454;
pub const ERR_HOTKEY_NOT_FOUND: i32 = 552;
pub const ERR_EXPRESSION_NOT_FOUND: i32 = 402;
pub const ERR_MODEL_NOT_LOADED: i32 = 501;
pub const ERR_UNSUPPORTED: i32 = 1000;

#[derive(Debug, Clone, Deserialize)]
pub struct RequestEnvelope {
    #[serde(rename = "apiName", default)]
    pub api_name: String,
    #[serde(rename = "apiVersion", default)]
    pub api_version: String,
    #[serde(rename = "requestID", default)]
    pub request_id: Option<String>,
    #[serde(rename = "messageType", default)]
    pub message_type: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResponseEnvelope {
    #[serde(rename = "apiName")]
    pub api_name: String,
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub timestamp: u64,
    #[serde(rename = "requestID")]
    pub request_id: String,
    #[serde(rename = "messageType")]
    pub message_type: String,
    pub data: Value,
}

impl ResponseEnvelope {
    pub fn new(request_id: Option<String>, message_type: &str, data: Value) -> Self {
        Self {
            api_name: API_NAME.to_string(),
            api_version: API_VERSION.to_string(),
            timestamp: now_ms(),
            request_id: request_id_or_uuid(request_id),
            message_type: message_type.to_string(),
            data,
        }
    }

    pub fn error(request_id: Option<String>, error_id: i32, message: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert("errorID".into(), Value::from(error_id));
        data.insert("message".into(), Value::String(message.into()));
        Self::new(request_id, "APIError", Value::Object(data))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"messageType":"APIError","data":{"errorID":100,"message":"serialization failed"}}"#
                .to_string()
        })
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn request_id_or_uuid(request_id: Option<String>) -> String {
    request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

pub fn validate_api(envelope: &RequestEnvelope) -> Result<(), ResponseEnvelope> {
    if envelope.api_name != API_NAME {
        return Err(ResponseEnvelope::error(
            envelope.request_id.clone(),
            ERR_INVALID_REQUEST,
            format!("Invalid apiName: expected {}", API_NAME),
        ));
    }
    if envelope.api_version != API_VERSION {
        return Err(ResponseEnvelope::error(
            envelope.request_id.clone(),
            ERR_INVALID_REQUEST,
            format!("Invalid apiVersion: expected {}", API_VERSION),
        ));
    }
    Ok(())
}

pub fn empty_data() -> Value {
    Value::Object(Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_api_state_request() {
        let json = r#"{
            "apiName": "VTubeStudioPublicAPI",
            "apiVersion": "1.0",
            "requestID": "test-id",
            "messageType": "APIStateRequest"
        }"#;
        let env: RequestEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.api_name, API_NAME);
        assert_eq!(env.message_type, "APIStateRequest");
        assert_eq!(env.request_id.as_deref(), Some("test-id"));
    }

    #[test]
    fn response_uses_camel_case() {
        let resp = ResponseEnvelope::new(
            Some("id".into()),
            "APIStateResponse",
            serde_json::json!({"active": true}),
        );
        let json = resp.to_json();
        assert!(json.contains("\"apiName\""));
        assert!(json.contains("\"requestID\""));
        assert!(json.contains("\"messageType\""));
    }

    #[test]
    fn api_error_shape() {
        let resp = ResponseEnvelope::error(Some("x".into()), 8, "auth required");
        let json = resp.to_json();
        assert!(json.contains("\"errorID\":8"));
        assert!(json.contains("\"requestID\""));
        assert!(json.contains("APIError"));
    }

    #[test]
    fn auth_response_has_reason() {
        let resp = ResponseEnvelope::new(
            Some("a1".into()),
            "AuthenticationResponse",
            serde_json::json!({
                "authenticated": true,
                "reason": "Authentication successful",
            }),
        );
        let json = resp.to_json();
        assert!(json.contains("\"reason\""));
        assert!(json.contains("\"requestID\":\"a1\""));
    }
}
