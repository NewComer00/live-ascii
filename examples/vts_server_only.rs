//! Headless VTS API server for testing (no model / no TUI).
//!
//!   cargo run --example vts_server_only
//!   cargo run --example vts_inject_loop   # in another terminal

use live_ascii::model_setting::ModelSetting;
use live_ascii::vts::{VtsConfig, VtsServer};

fn main() {
    let setting = ModelSetting {
        version: 3,
        file_references: Default::default(),
        groups: vec![],
        hit_areas: vec![],
        layout: None,
    };

    let port = std::env::var("VTS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8001u16);

    let _server = VtsServer::start(
        VtsConfig {
            port,
            auto_approve: true,
            model_name: "test".into(),
        },
        &setting,
        None,
    );

    eprintln!("VTS API server listening on ws://127.0.0.1:{port} (Ctrl+C to stop)");
    eprintln!("Rebuild this example after changing src/vts/ — stop any older copy on the same port.");
    std::thread::park();
}
