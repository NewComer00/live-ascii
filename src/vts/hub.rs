use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::events::{model_animation_event_json, Subscriptions};

/// Routes server-push VTS events to subscribed WebSocket clients.
#[derive(Debug, Default)]
pub struct EventHub {
    next_id: AtomicU64,
    clients: Mutex<HashMap<u64, ClientHandle>>,
}

#[derive(Debug)]
struct ClientHandle {
    subscriptions: Arc<Mutex<Subscriptions>>,
    tx: mpsc::UnboundedSender<String>,
}

impl EventHub {
    pub fn register(
        &self,
        subscriptions: Arc<Mutex<Subscriptions>>,
        tx: mpsc::UnboundedSender<String>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.clients.lock().unwrap().insert(
            id,
            ClientHandle {
                subscriptions,
                tx,
            },
        );
        id
    }

    pub fn unregister(&self, id: u64) {
        self.clients.lock().unwrap().remove(&id);
    }

    pub fn broadcast_model_animation(
        &self,
        model_id: &str,
        model_name: &str,
        event_type: &str,
        animation_name: &str,
        animation_length: f64,
        is_idle_animation: bool,
    ) {
        let payload = model_animation_event_json(
            event_type,
            animation_name,
            animation_length,
            is_idle_animation,
            model_id,
            model_name,
        );
        let clients = self.clients.lock().unwrap();
        for client in clients.values() {
            let subs = client.subscriptions.lock().unwrap();
            let Some(cfg) = &subs.model_animation else {
                continue;
            };
            if cfg.ignore_idle_animations && is_idle_animation {
                continue;
            }
            // live-ascii never emits Live2D-item animation events.
            let _ = client.tx.send(payload.clone());
        }
    }
}
