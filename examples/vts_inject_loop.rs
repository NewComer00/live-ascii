//! Demo client for live-ascii's VTS server: tracking inject, hotkeys, expressions.
//!
//! Terminal 1 (restart after rebuilding live-ascii):
//!   cargo run -- models/mao_en/runtime/mao_pro.model3.json --vts
//!   # or: cargo run --example vts_server_only
//!
//! Terminal 2:
//!   cargo run --example vts_inject_loop
//!
//! Optional env:
//!   VTS_URL=ws://127.0.0.1:8001
//!   VTS_AUTH_TOKEN=<saved token from prior run>

use std::f64::consts::PI;
use std::time::{Duration, Instant};

use vtubestudio::data::{
    ApiStateRequest, CurrentModelRequest, ExpressionActivationRequest, ExpressionStateRequest,
    HotkeyTriggerRequest, HotkeysInCurrentModelRequest, InjectParameterDataMode,
    InjectParameterDataRequest, ParameterValue,
};
use vtubestudio::{Client, ClientEvent};

const FRAME_MS: u64 = 33;
const MOTION_INTERVAL: u64 = 240; // ~8 s at 30 fps
const EXPRESSION_INTERVAL: u64 = 360; // ~12 s at 30 fps

struct DemoAssets {
    motion_hotkeys: Vec<(String, String)>,
    expressions: Vec<(String, String)>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("VTS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8001".into());
    let stored_token = std::env::var("VTS_AUTH_TOKEN").ok();

    let mut builder = Client::builder()
        .url(&url)
        .authentication("live-ascii test", "Dev", None);
    if let Some(token) = stored_token {
        builder = builder.auth_token(Some(token));
    }

    let (mut client, mut events) = builder.build_tungstenite();

    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                ClientEvent::Connected => eprintln!("event: connected"),
                ClientEvent::Disconnected => eprintln!("event: disconnected"),
                ClientEvent::NewAuthToken(token) => {
                    eprintln!("Save for next run: VTS_AUTH_TOKEN={token}");
                }
                other => eprintln!("event: {other:?}"),
            }
        }
    });

    eprintln!("Connecting to {url} ...");

    match client.send(&ApiStateRequest {}).await {
        Ok(state) => eprintln!(
            "API ok  active={}  vts=\"{}\"  authenticated={}",
            state.active, state.vtubestudio_version, state.current_session_authenticated
        ),
        Err(e) => {
            eprintln!("Cannot reach VTS server at {url}: {e:?}");
            eprintln!();
            eprintln!("Checklist:");
            eprintln!("  1. Start the server in another terminal (--vts or vts_server_only)");
            eprintln!("  2. Stop any old vts_server_only.exe still bound to port 8001");
            eprintln!("  3. Rebuild and restart the server after pulling live-ascii changes");
            return Err(e.into());
        }
    }

    if let Ok(model) = client.send(&CurrentModelRequest {}).await {
        eprintln!(
            "Model  loaded={}  name=\"{}\"  params={}",
            model.model_loaded, model.model_name, model.number_of_live2d_parameters
        );
    }

    let assets = discover_assets(&mut client).await;
    eprintln!(
        "Discovered  motions={}  expressions={}",
        assets.motion_hotkeys.len(),
        assets.expressions.len()
    );
    for (id, name) in &assets.motion_hotkeys {
        eprintln!("  motion hotkey  {id}  ({name})");
    }
    for (file, name) in &assets.expressions {
        eprintln!("  expression  {file}  ({name})");
    }

    let start = Instant::now();
    let mut frame = 0u64;
    let mut backoff = Duration::from_millis(FRAME_MS);
    let mut motion_idx = 0usize;
    let mut expression_idx = 0usize;
    let mut expression_on = false;

    loop {
        let t = start.elapsed().as_secs_f64();

        if frame > 0 && frame % MOTION_INTERVAL == 0 && !assets.motion_hotkeys.is_empty() {
            let (hotkey_id, name) = &assets.motion_hotkeys[motion_idx % assets.motion_hotkeys.len()];
            motion_idx += 1;
            match client
                .send(&HotkeyTriggerRequest {
                    hotkey_id: hotkey_id.clone(),
                    item_instance_id: None,
                })
                .await
            {
                Ok(_) => println!("hotkey ok  {name} ({hotkey_id})"),
                Err(e) => eprintln!("hotkey error ({hotkey_id}): {e:?}"),
            }
        }

        if frame > 0 && frame % EXPRESSION_INTERVAL == 0 && !assets.expressions.is_empty() {
            let (file, name) = &assets.expressions[expression_idx % assets.expressions.len()];
            expression_on = !expression_on;
            if !expression_on {
                expression_idx += 1;
            }
            match client
                .send(&ExpressionActivationRequest {
                    expression_file: file.clone(),
                    active: expression_on,
                })
                .await
            {
                Ok(_) => println!(
                    "expression ok  {}  {name} ({file})",
                    if expression_on { "on" } else { "off" }
                ),
                Err(e) => eprintln!("expression error ({file}): {e:?}"),
            }
        }

        let values = animated_params(t);
        let head_x = values
            .iter()
            .find(|p| p.id == "FaceAngleX")
            .map(|p| p.value)
            .unwrap_or(0.0);
        let mouth = values
            .iter()
            .find(|p| p.id == "MouthOpen")
            .map(|p| p.value)
            .unwrap_or(0.0);

        match client
            .send(&InjectParameterDataRequest {
                face_found: true,
                mode: Some(InjectParameterDataMode::Set.into()),
                parameter_values: values,
            })
            .await
        {
            Ok(_) => {
                backoff = Duration::from_millis(FRAME_MS);
                if frame % 30 == 0 {
                    println!("inject ok  head={head_x:.1}°  mouth={mouth:.2}");
                }
            }
            Err(e) => {
                eprintln!("inject error: {e:?}");
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        }

        frame += 1;
        tokio::time::sleep(backoff).await;
    }
}

async fn discover_assets(client: &mut Client) -> DemoAssets {
    let mut motion_hotkeys = Vec::new();
    let mut expressions = Vec::new();

    if let Ok(hotkeys) = client
        .send(&HotkeysInCurrentModelRequest {
            model_id: None,
            live2d_item_file_name: None,
        })
        .await
    {
        for hk in hotkeys.available_hotkeys {
            let kind = hk.type_.as_str();
            if kind == "TriggerAnimation" {
                motion_hotkeys.push((hk.hotkey_id, hk.name));
            } else if kind == "ToggleExpression" && hk.file.ends_with(".exp3.json") {
                expressions.push((hk.file, hk.name));
            }
        }
    }

    if expressions.is_empty() {
        if let Ok(state) = client
            .send(&ExpressionStateRequest {
                details: false,
                expression_file: None,
            })
            .await
        {
            for exp in state.expressions {
                expressions.push((exp.file, exp.name));
            }
        }
    }

    DemoAssets {
        motion_hotkeys,
        expressions,
    }
}

fn animated_params(t: f64) -> Vec<ParameterValue> {
    let head_x = (t * 0.8 * PI).sin() * 18.0;
    let head_y = (t * 0.5 * PI).sin() * 12.0;
    let head_z = (t * 0.35 * PI).cos() * 8.0;
    let mouth = ((t * 6.0).sin().abs() * 0.55).clamp(0.0, 1.0);
    let smile = (t * 0.25 * PI).sin() * 0.35;
    let eye_x = (t * 0.45 * PI).sin() * 0.6;
    let eye_y = (t * 0.3 * PI).cos() * 0.35;
    let blink = if (t * 1.2).sin() > 0.92 { 0.05 } else { 1.0 };
    let brow = (t * 0.2 * PI).sin() * 0.4;

    vec![
        param("FaceAngleX", head_x),
        param("FaceAngleY", head_y),
        param("FaceAngleZ", head_z),
        param("MouthOpen", mouth),
        param("MouthSmile", smile),
        param("EyeLeftX", eye_x),
        param("EyeLeftY", eye_y),
        param("EyeRightX", eye_x),
        param("EyeRightY", eye_y),
        param("EyeOpenLeft", blink),
        param("EyeOpenRight", blink),
        param("BrowLeftY", brow),
        param("BrowRightY", brow),
    ]
}

fn param(id: &str, value: f64) -> ParameterValue {
    ParameterValue {
        id: id.into(),
        value,
        weight: None,
    }
}
