//! Subscribe to `ModelAnimationEvent` and chain motion hotkeys on End (no timers).
//!
//! `isIdleAnimation` (official VTS API):
//! - On each `ModelAnimationEvent`: `ev.is_idle_animation` (`true` = idle loop; End repeats each cycle).
//! - This example sets `ignore_idle_animations: false` so idle Start/End are received.
//! - Chains on the first matching `End` per trigger (one loop cycle for idle motions).
//!
//! Terminal 1 (restart after rebuilding live-ascii):
//!   cargo run --release -- models/mao_en/runtime/mao_pro.model3.json --vts
//!
//! Terminal 2:
//!   cargo run --example vts_animation_events
//!
//! Optional env:
//!   VTS_URL=ws://127.0.0.1:8001
//!   VTS_AUTH_TOKEN=<saved token from prior run>

use std::time::Duration;

use tokio::sync::mpsc;
use vtubestudio::data::{
    AnimationEventType, ApiStateRequest, Event, EventSubscriptionRequest,
    HotkeyTriggerRequest, HotkeysInCurrentModelRequest, ModelAnimationEvent,
    ModelAnimationEventConfig,
};
use vtubestudio::{Client, ClientEvent};

struct MotionHotkey {
    id: String,
    name: String,
    file: String,
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
    let (anim_tx, mut anim_rx) = mpsc::unbounded_channel::<ModelAnimationEvent>();

    tokio::spawn(async move {
        while let Some(event) = events.next().await {
            match event {
                ClientEvent::Connected => eprintln!("event: connected"),
                ClientEvent::Disconnected => eprintln!("event: disconnected"),
                ClientEvent::NewAuthToken(token) => {
                    eprintln!("Save for next run: VTS_AUTH_TOKEN={token}");
                }
                ClientEvent::Api(Event::ModelAnimation(ev)) => {
                    let _ = anim_tx.send(ev);
                }
                other => eprintln!("event: {other:?}"),
            }
        }
    });

    eprintln!("Connecting to {url} ...");

    let state = client.send(&ApiStateRequest {}).await.map_err(|e| {
        eprintln!("Cannot reach VTS server at {url}: {e:?}");
        eprintln!();
        eprintln!("Start live-ascii with --vts in another terminal first.");
        e
    })?;
    eprintln!(
        "API ok  active={}  vts=\"{}\"",
        state.active, state.vtubestudio_version
    );

    let sub = client
        .send(
            &EventSubscriptionRequest::subscribe(&ModelAnimationEventConfig {
                ignore_live2d_items: true,
                ignore_idle_animations: false,
            })?,
        )
        .await?;
    eprintln!(
        "Subscribed to {} event(s): {:?}",
        sub.subscribed_event_count, sub.subscribed_events
    );

    let motions = discover_motion_hotkeys(&mut client).await;
    if motions.is_empty() {
        eprintln!("No TriggerAnimation hotkeys found on the loaded model.");
        return Ok(());
    }

    eprintln!("Motion hotkeys ({}):", motions.len());
    for m in &motions {
        eprintln!("  {}  {}  ({})", m.id, m.name, m.file);
    }

    let mut idx = 0usize;
    loop {
        let motion = &motions[idx % motions.len()];
        idx += 1;

        client
            .send(&HotkeyTriggerRequest {
                hotkey_id: motion.id.clone(),
                item_instance_id: None,
            })
            .await?;
        eprintln!("triggered  {}  ({})", motion.name, motion.file);

        match wait_for_motion(&mut anim_rx, &motion.file, Duration::from_secs(30)).await {
            Ok((length, idle)) => {
                eprintln!(
                    "End  {}  ({length:.2}s)  idle={idle}",
                    motion.file
                )
            }
            Err(e) => eprintln!("wait error: {e}"),
        }

        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn discover_motion_hotkeys(client: &mut Client) -> Vec<MotionHotkey> {
    let mut motions = Vec::new();
    if let Ok(hotkeys) = client
        .send(&HotkeysInCurrentModelRequest {
            model_id: None,
            live2d_item_file_name: None,
        })
        .await
    {
        for hk in hotkeys.available_hotkeys {
            if hk.type_.as_str() == "TriggerAnimation" && !hk.file.is_empty() {
                motions.push(MotionHotkey {
                    id: hk.hotkey_id,
                    name: hk.name,
                    file: hk.file,
                });
            }
        }
    }
    motions
}

async fn wait_for_motion(
    rx: &mut mpsc::UnboundedReceiver<ModelAnimationEvent>,
    expected_file: &str,
    timeout: Duration,
) -> Result<(f64, bool), Box<dyn std::error::Error + Send + Sync>> {
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        tokio::select! {
            msg = rx.recv() => {
                let Some(ev) = msg else {
                    return Err("event stream closed".into());
                };
                if ev.animation_event_type == AnimationEventType::Start
                    && animation_matches(&ev.animation_name, expected_file)
                {
                    eprintln!(
                        "  Start  {}  ({:.2}s)  idle={}",
                        ev.animation_name, ev.animation_length, ev.is_idle_animation
                    );
                } else if ev.animation_event_type == AnimationEventType::End
                    && animation_matches(&ev.animation_name, expected_file)
                {
                    return Ok((ev.animation_length, ev.is_idle_animation));
                } else if ev.animation_event_type == AnimationEventType::Custom {
                    eprintln!(
                        "  Custom  {} @ {:.2}s  {:?}",
                        ev.animation_name, ev.animation_event_time, ev.animation_event_data
                    );
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(format!("timeout waiting for End of {expected_file}").into());
            }
        }
    }
}

fn animation_matches(animation_name: &str, expected_file: &str) -> bool {
    animation_name == expected_file
        || animation_name.ends_with(expected_file)
        || expected_file.ends_with(animation_name)
}
