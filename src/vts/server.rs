use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use super::auth::{validate_plugin_identity, TokenStore};
use super::mapping::default_param_by_name;
use super::protocol::{
    empty_data, request_id_or_uuid, validate_api, ResponseEnvelope, RequestEnvelope,
    ERR_EXPRESSION_NOT_FOUND, ERR_HOTKEY_NOT_FOUND, ERR_MODEL_NOT_LOADED,
    ERR_PARAM_NOT_FOUND, ERR_REQUIRES_AUTH, ERR_TOKEN_DENIED, ERR_UNSUPPORTED, VTS_VERSION,
};
use super::state::{InjectionMode, SharedVtsState, VtsMainCommand};

#[derive(Debug)]
struct Session {
    authenticated: bool,
    plugin_key: Option<String>,
}

impl Session {
    fn new() -> Self {
        Self {
            authenticated: false,
            plugin_key: None,
        }
    }
}

pub async fn run_server(
    port: u16,
    auto_approve: bool,
    state: SharedVtsState,
    token_store: Arc<Mutex<TokenStore>>,
) {
    let addr = format!("127.0.0.1:{port}");
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("VTS server: failed to bind {addr}: {e}");
            return;
        }
    };
    eprintln!("VTS API server listening on ws://{addr}");

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        let token_store = Arc::clone(&token_store);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, auto_approve, state, token_store).await {
                eprintln!("VTS connection error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    auto_approve: bool,
    state: SharedVtsState,
    token_store: Arc<Mutex<TokenStore>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    let (mut write, mut read) = ws.split();
    let mut session = Session::new();

    {
        let shared = state.lock().unwrap();
        shared.client_connected();
    }

    let mut disconnected = false;
    let mut disconnect = || {
        if disconnected {
            return;
        }
        disconnected = true;
        if let Ok(shared) = state.lock() {
            shared.client_disconnected();
        }
    };

    while let Some(msg) = read.next().await {
        let msg = msg?;
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => String::from_utf8(b.to_vec()).unwrap_or_default(),
            Message::Close(_) => {
                disconnect();
                break;
            }
            Message::Ping(data) => {
                write.send(Message::Pong(data)).await?;
                continue;
            }
            Message::Pong(_) | Message::Frame(_) => continue,
        };

        if text.is_empty() {
            continue;
        }

        let response = dispatch_message(&text, &mut session, auto_approve, &state, &token_store);
        write.send(Message::Text(response.into())).await?;
    }

    disconnect();
    Ok(())
}

fn dispatch_message(
    text: &str,
    session: &mut Session,
    auto_approve: bool,
    state: &SharedVtsState,
    token_store: &Arc<Mutex<TokenStore>>,
) -> String {
    let envelope: RequestEnvelope = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            return ResponseEnvelope::error(None, 100, format!("Invalid JSON: {e}")).to_json();
        }
    };

    if let Err(resp) = validate_api(&envelope) {
        return resp.to_json();
    }

    let request_id = envelope.request_id.clone();
    let msg_type = envelope.message_type.as_str();

    match msg_type {
        "APIStateRequest" => api_state_response(request_id, session),
        "AuthenticationTokenRequest" => auth_token_request(
            request_id,
            &envelope.data,
            auto_approve,
            token_store,
        ),
        "AuthenticationRequest" => auth_request(request_id, &envelope.data, session, token_store),
        "StatisticsRequest" => statistics_request(request_id, state),
        "CurrentModelRequest" => current_model_request(request_id, state),
        "AvailableModelsRequest" => available_models_request(request_id, state),
        "InputParameterListRequest" => input_parameter_list_request(request_id, state),
        "Live2DParameterListRequest" => live2d_parameter_list_request(request_id, state),
        "ParameterValueRequest" => parameter_value_request(request_id, &envelope.data, state),
        "InjectParameterDataRequest" => {
            inject_parameter_data_request(request_id, &envelope.data, session, state)
        }
        "HotkeysInCurrentModelRequest" => hotkeys_request(request_id, state),
        "HotkeyTriggerRequest" => hotkey_trigger_request(request_id, &envelope.data, session, state),
        "ExpressionStateRequest" => expression_state_request(request_id, state),
        "ExpressionActivationRequest" => {
            expression_activation_request(request_id, &envelope.data, session, state)
        }
        _ => ResponseEnvelope::error(
            request_id,
            ERR_UNSUPPORTED,
            format!("Unsupported message type: {msg_type}"),
        )
        .to_json(),
    }
}

fn api_state_response(request_id: Option<String>, session: &Session) -> String {
    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "APIStateResponse",
        json!({
            "active": true,
            "vTubeStudioVersion": VTS_VERSION,
            "currentSessionAuthenticated": session.authenticated,
        }),
    )
    .to_json()
}

fn auth_token_request(
    request_id: Option<String>,
    data: &Value,
    auto_approve: bool,
    token_store: &Arc<Mutex<TokenStore>>,
) -> String {
    let plugin_name = data.get("pluginName").and_then(|v| v.as_str()).unwrap_or("");
    let plugin_developer = data
        .get("pluginDeveloper")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Err(msg) = validate_plugin_identity(plugin_name, plugin_developer) {
        return ResponseEnvelope::error(request_id, 100, msg).to_json();
    }

    if !auto_approve {
        return ResponseEnvelope::error(
            request_id,
            ERR_TOKEN_DENIED,
            "Token request denied (auto-approve disabled)",
        )
        .to_json();
    }

    eprintln!(
        "VTS: auto-approved plugin \"{plugin_name}\" by \"{plugin_developer}\""
    );

    let token = token_store
        .lock()
        .unwrap()
        .issue_token(plugin_name, plugin_developer);

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "AuthenticationTokenResponse",
        json!({ "authenticationToken": token }),
    )
    .to_json()
}

fn auth_request(
    request_id: Option<String>,
    data: &Value,
    session: &mut Session,
    token_store: &Arc<Mutex<TokenStore>>,
) -> String {
    let plugin_name = data.get("pluginName").and_then(|v| v.as_str()).unwrap_or("");
    let plugin_developer = data
        .get("pluginDeveloper")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let token = data
        .get("authenticationToken")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Err(msg) = validate_plugin_identity(plugin_name, plugin_developer) {
        return ResponseEnvelope::error(request_id, 100, msg).to_json();
    }

    let valid = token_store
        .lock()
        .unwrap()
        .validate_token(plugin_name, plugin_developer, token);

    if !valid {
        return ResponseEnvelope::error(request_id, ERR_REQUIRES_AUTH, "Invalid authentication token")
            .to_json();
    }

    session.authenticated = true;
    session.plugin_key = Some(super::auth::plugin_key(plugin_name, plugin_developer));

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "AuthenticationResponse",
        json!({
            "authenticated": true,
            "reason": "Authentication successful",
        }),
    )
    .to_json()
}

fn require_auth(request_id: Option<String>, session: &Session) -> Result<(), String> {
    if session.authenticated {
        Ok(())
    } else {
        Err(
            ResponseEnvelope::error(
                request_id,
                ERR_REQUIRES_AUTH,
                "Authentication required",
            )
            .to_json(),
        )
    }
}

fn statistics_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "StatisticsResponse",
        json!({
            "uptime": shared.uptime_ms(),
            "framerate": 60,
            "vTubeStudioVersion": VTS_VERSION,
            "connectedPlugins": shared.connected_clients.load(std::sync::atomic::Ordering::Relaxed),
            "startedWithSteam": false,
            "vtubeStudioVersionNumber": 100,
            "vtubeStudioVersionString": VTS_VERSION,
        }),
    )
    .to_json()
}

fn current_model_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    let snap = shared.snapshot();
    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "CurrentModelResponse",
        json!({
            "modelLoaded": shared.model_loaded,
            "modelName": shared.model_name,
            "modelID": shared.model_id,
            "vtsModelName": "",
            "vtsModelIconName": "",
            "live2DModelName": format!("{}.model3.json", shared.model_name),
            "modelLoadTime": 0,
            "timeSinceModelLoaded": shared.uptime_ms(),
            "numberOfLive2DParameters": snap.live2d_params.len(),
            "numberOfLive2DArtmeshes": 0,
            "hasPhysicsFile": false,
            "numberOfTextures": 0,
            "textureResolution": 0,
            "modelPosition": {
                "positionX": 0.0,
                "positionY": 0.0,
                "rotation": 0.0,
                "size": 0.0
            }
        }),
    )
    .to_json()
}

fn available_models_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "AvailableModelsResponse",
        json!({
            "numberOfLoadedModels": if shared.model_loaded { 1 } else { 0 },
            "totalNumberOfModels": 1,
            "availableModels": [{
                "modelID": shared.model_id,
                "modelName": shared.model_name,
                "modelLoadTime": 0,
                "timeSinceModelLoaded": shared.uptime_ms(),
            }]
        }),
    )
    .to_json()
}

fn input_parameter_list_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    let snap = shared.snapshot();
    let default_parameters: Vec<Value> = super::mapping::DEFAULT_PARAMS
        .iter()
        .map(|spec| {
            let value = snap
                .vts_default_values
                .get(spec.name)
                .copied()
                .unwrap_or(spec.default_value);
            json!({
                "name": spec.name,
                "addedBy": "VTube Studio",
                "value": value,
                "min": spec.min,
                "max": spec.max,
                "defaultValue": spec.default_value,
            })
        })
        .collect();

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "InputParameterListResponse",
        json!({
            "modelLoaded": shared.model_loaded,
            "modelName": shared.model_name,
            "modelID": shared.model_id,
            "customParameters": [],
            "defaultParameters": default_parameters,
        }),
    )
    .to_json()
}

fn live2d_parameter_list_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    if !shared.model_loaded {
        return ResponseEnvelope::error(request_id, ERR_MODEL_NOT_LOADED, "No model loaded").to_json();
    }
    let snap = shared.snapshot();
    let parameters: Vec<Value> = snap
        .live2d_params
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "value": p.value,
                "min": p.min,
                "max": p.max,
                "defaultValue": p.default_value,
            })
        })
        .collect();

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "Live2DParameterListResponse",
        json!({
            "modelLoaded": true,
            "modelName": shared.model_name,
            "modelID": shared.model_id,
            "parameters": parameters,
        }),
    )
    .to_json()
}

fn parameter_value_request(
    request_id: Option<String>,
    data: &Value,
    state: &SharedVtsState,
) -> String {
    let name = data.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let shared = state.lock().unwrap();
    let snap = shared.snapshot();

    if let Some(spec) = default_param_by_name(name) {
        let value = snap
            .vts_default_values
            .get(name)
            .copied()
            .unwrap_or(spec.default_value);
        return ResponseEnvelope::new(
            Some(request_id_or_uuid(request_id)),
            "ParameterValueResponse",
            json!({
                "name": name,
                "addedBy": "VTube Studio",
                "value": value,
                "min": spec.min,
                "max": spec.max,
                "defaultValue": spec.default_value,
            }),
        )
        .to_json();
    }

    if let Some(p) = snap.live2d_params.iter().find(|p| p.name == name) {
        return ResponseEnvelope::new(
            Some(request_id_or_uuid(request_id)),
            "ParameterValueResponse",
            json!({
                "name": name,
                "addedBy": "Live2D",
                "value": p.value,
                "min": p.min,
                "max": p.max,
                "defaultValue": p.default_value,
            }),
        )
        .to_json();
    }

    ResponseEnvelope::error(request_id, ERR_PARAM_NOT_FOUND, format!("Parameter not found: {name}"))
        .to_json()
}

fn inject_parameter_data_request(
    request_id: Option<String>,
    data: &Value,
    session: &Session,
    state: &SharedVtsState,
) -> String {
    if let Err(resp) = require_auth(request_id.clone(), session) {
        return resp;
    }

    let face_found = data.get("faceFound").and_then(|v| v.as_bool()).unwrap_or(false);
    let mode_str = data.get("mode").and_then(|v| v.as_str()).unwrap_or("set");
    let mode = if mode_str.eq_ignore_ascii_case("add") {
        InjectionMode::Add
    } else {
        InjectionMode::Set
    };

    let mut values = Vec::new();
    if let Some(arr) = data.get("parameterValues").and_then(|v| v.as_array()) {
        for item in arr {
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let value = item.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
            let weight = item
                .get("weight")
                .and_then(|v| v.as_f64())
                .map(|w| w as f32)
                .unwrap_or(1.0);
            values.push((id.to_string(), value, weight.clamp(0.0, 1.0)));
        }
    }

    let mut shared = state.lock().unwrap();
    if let Err((code, msg)) =
        shared.inject_parameters(session.plugin_key.as_deref(), face_found, mode, &values)
    {
        return ResponseEnvelope::error(request_id, code, msg).to_json();
    }

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "InjectParameterDataResponse",
        empty_data(),
    )
    .to_json()
}

fn hotkeys_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    let available: Vec<Value> = shared
        .hotkeys
        .iter()
        .map(|h| {
            json!({
                "name": h.name,
                "type": h.hotkey_type,
                "description": h.description,
                "file": h.file,
                "hotkeyID": h.hotkey_id,
                "keyCombination": [],
                "onScreenButtonID": -1,
            })
        })
        .collect();

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "HotkeysInCurrentModelResponse",
        json!({
            "modelLoaded": shared.model_loaded,
            "modelName": shared.model_name,
            "modelID": shared.model_id,
            "availableHotkeys": available,
        }),
    )
    .to_json()
}

fn hotkey_trigger_request(
    request_id: Option<String>,
    data: &Value,
    session: &Session,
    state: &SharedVtsState,
) -> String {
    if let Err(resp) = require_auth(request_id.clone(), session) {
        return resp;
    }

    let hotkey_id = data.get("hotkeyID").and_then(|v| v.as_str()).unwrap_or("");
    let mut shared = state.lock().unwrap();
    if shared.find_hotkey(hotkey_id).is_none() {
        return ResponseEnvelope::error(
            request_id,
            ERR_HOTKEY_NOT_FOUND,
            format!("Hotkey not found: {hotkey_id}"),
        )
        .to_json();
    }

    shared.enqueue_command(VtsMainCommand::TriggerHotkey {
        hotkey_id: hotkey_id.to_string(),
    });

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "HotkeyTriggerResponse",
        json!({ "hotkeyID": hotkey_id }),
    )
    .to_json()
}

fn expression_state_request(request_id: Option<String>, state: &SharedVtsState) -> String {
    let shared = state.lock().unwrap();
    let snap = shared.snapshot();
    let expressions: Vec<Value> = shared
        .expression_files
        .keys()
        .map(|file| {
            let active = snap.active_expressions.contains(file);
            json!({
                "name": shared.expression_files.get(file).cloned().unwrap_or_default(),
                "file": file,
                "active": active,
                "deactivateWhenKeyIsLetGo": false,
                "autoDeactivateAfterSeconds": false,
                "secondsRemaining": 0.0,
                "usedInHotkeys": [],
                "parameters": [],
            })
        })
        .collect();

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "ExpressionStateResponse",
        json!({
            "modelLoaded": shared.model_loaded,
            "modelName": shared.model_name,
            "modelID": shared.model_id,
            "expressions": expressions,
        }),
    )
    .to_json()
}

fn expression_activation_request(
    request_id: Option<String>,
    data: &Value,
    session: &Session,
    state: &SharedVtsState,
) -> String {
    if let Err(resp) = require_auth(request_id.clone(), session) {
        return resp;
    }

    let file = data
        .get("expressionFile")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let active = data.get("active").and_then(|v| v.as_bool()).unwrap_or(true);

    if !file.ends_with(".exp3.json") {
        return ResponseEnvelope::error(
            request_id,
            ERR_EXPRESSION_NOT_FOUND,
            "Invalid expression file name",
        )
        .to_json();
    }

    let mut shared = state.lock().unwrap();
    if shared.find_expression_by_file(file).is_none() {
        return ResponseEnvelope::error(
            request_id,
            ERR_EXPRESSION_NOT_FOUND,
            format!("Expression not found: {file}"),
        )
        .to_json();
    }

    shared.enqueue_command(VtsMainCommand::SetExpression {
        file: file.to_string(),
        active,
    });

    ResponseEnvelope::new(
        Some(request_id_or_uuid(request_id)),
        "ExpressionActivationResponse",
        empty_data(),
    )
    .to_json()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::model_setting::ModelSetting;
    use crate::vts::auth::TokenStore;
    use crate::vts::state::VtsSharedState;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::connect_async;

    #[test]
    fn api_state_roundtrip() {
        let session = Session::new();
        let json = api_state_response(Some("test".into()), &session);
        assert!(json.contains("APIStateResponse"));
        assert!(json.contains("live-ascii"));
    }

    #[tokio::test]
    async fn websocket_auth_and_inject() {
        let port = 18765u16;
        let setting = ModelSetting {
            version: 3,
            file_references: Default::default(),
            groups: vec![],
            hit_areas: vec![],
            layout: None,
        };
        let state = Arc::new(Mutex::new(VtsSharedState::from_model_setting(
            "test",
            &setting,
            None,
        )));
        let token_store = Arc::new(Mutex::new(TokenStore::load_default()));

        tokio::spawn(run_server(port, true, Arc::clone(&state), Arc::clone(&token_store)));
        tokio::time::sleep(Duration::from_millis(50)).await;

        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _) = connect_async(&url).await.expect("connect");
        let (mut write, mut read) = ws.split();

        write
            .send(Message::Text(
                r#"{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"t1","messageType":"AuthenticationTokenRequest","data":{"pluginName":"live-ascii test","pluginDeveloper":"Dev"}}"#.into(),
            ))
            .await
            .unwrap();
        let token_resp = read.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(token_resp.contains("AuthenticationTokenResponse"));
        assert!(token_resp.contains("\"requestID\":\"t1\""));

        let token: String = serde_json::from_str::<serde_json::Value>(&token_resp)
            .unwrap()
            .pointer("/data/authenticationToken")
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();

        write
            .send(Message::Text(format!(
                r#"{{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"a1","messageType":"AuthenticationRequest","data":{{"pluginName":"live-ascii test","pluginDeveloper":"Dev","authenticationToken":"{token}"}}}}"#
            ).into()))
            .await
            .unwrap();
        let auth_resp = read.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(auth_resp.contains("AuthenticationResponse"));
        assert!(auth_resp.contains("\"reason\""));
        assert!(auth_resp.contains("\"authenticated\":true"));

        write
            .send(Message::Text(
                r#"{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"i1","messageType":"InjectParameterDataRequest","data":{"faceFound":true,"mode":"set","parameterValues":[{"id":"FaceAngleX","value":15.0}]}}"#.into(),
            ))
            .await
            .unwrap();
        let inject_resp = read.next().await.unwrap().unwrap().into_text().unwrap();
        assert!(inject_resp.contains("InjectParameterDataResponse"));
        assert!(inject_resp.contains("\"requestID\":\"i1\""));
    }
}
