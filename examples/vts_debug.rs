//! Raw WebSocket debug: auth + inject, print every response JSON.
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("VTS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8001".into());
    let (ws, _) = connect_async(&url).await?;
    let (mut write, mut read) = ws.split();
    eprintln!("connected to {url}");

    write
        .send(Message::Text(
            r#"{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"t1","messageType":"AuthenticationTokenRequest","data":{"pluginName":"live-ascii test","pluginDeveloper":"Dev"}}"#.into(),
        ))
        .await?;
    let token_resp = match read.next().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        other => format!("unexpected: {other:?}"),
    };
    println!("TOKEN: {token_resp}");

    let token: String = serde_json::from_str::<serde_json::Value>(&token_resp)?
        .get("data")
        .and_then(|d| d.get("authenticationToken"))
        .and_then(|t| t.as_str())
        .unwrap()
        .to_string();

    write
        .send(Message::Text(
            format!(
                r#"{{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"a1","messageType":"AuthenticationRequest","data":{{"pluginName":"live-ascii test","pluginDeveloper":"Dev","authenticationToken":"{token}"}}}}"#
            )
            .into(),
        ))
        .await?;
    let auth_resp = match read.next().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        other => format!("unexpected: {other:?}"),
    };
    println!("AUTH: {auth_resp}");

    write
        .send(Message::Text(
            r#"{"apiName":"VTubeStudioPublicAPI","apiVersion":"1.0","requestID":"i1","messageType":"InjectParameterDataRequest","data":{"faceFound":true,"mode":"set","parameterValues":[{"id":"FaceAngleX","value":15.0}]}}"#.into(),
        ))
        .await?;
    let inject_resp = match read.next().await {
        Some(Ok(Message::Text(t))) => t.to_string(),
        other => format!("unexpected: {other:?}"),
    };
    println!("INJECT: {inject_resp}");

    Ok(())
}
