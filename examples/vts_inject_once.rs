//! One-shot vtubestudio-rs client test (prints full error debug).
use vtubestudio::data::{
    InjectParameterDataMode, InjectParameterDataRequest, ParameterValue,
};
use vtubestudio::Client;

#[tokio::main]
async fn main() {
    let url = std::env::var("VTS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8001".into());
    let (mut client, _) = Client::builder()
        .url(&url)
        .authentication("live-ascii test", "Dev", None)
        .build_tungstenite();

    let result = client
        .send(&InjectParameterDataRequest {
            face_found: true,
            mode: Some(InjectParameterDataMode::Set.into()),
            parameter_values: vec![ParameterValue {
                id: "FaceAngleX".into(),
                value: 15.0,
                weight: None,
            }],
        })
        .await;

    match result {
        Ok(r) => println!("OK: {:?}", r),
        Err(e) => println!("ERR: {e:?}"),
    }
}
