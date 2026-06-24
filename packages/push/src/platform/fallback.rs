//! Windows / Linux fallback backend.
//!
//! There is no OS-level push service on these platforms, so this maintains an
//! app-managed WebSocket connection to the developer's backend and surfaces pushed
//! messages as local desktop notifications (via [`dioxus_sdk_notification`]). This is
//! **not** a wake-from-closed push service: it only delivers while the app is running
//! and connected.

use crate::core::{
    DevicePushToken, Error, NotificationContent, PermissionStatus, Provider, PushConfig, PushEvent,
    RemoteMessage,
};
use crate::platform::bridge;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;

pub use bridge::listen;

pub struct PushManager {
    config: PushConfig,
    device_id: String,
}

impl PushManager {
    pub fn new(config: PushConfig) -> Result<Self, Error> {
        let device_id = config
            .fallback_device_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        Ok(Self { config, device_id })
    }
}

/// Desktop OSes do not gate showing notifications behind a runtime permission.
pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    Ok(PermissionStatus::Granted)
}

pub fn register(manager: &PushManager) -> Result<(), Error> {
    let device_id = manager.device_id.clone();
    let endpoint = manager.config.fallback_endpoint.clone();

    // The device id *is* the SelfHosted token; surface it immediately.
    bridge::emit(PushEvent::TokenRegistered(DevicePushToken {
        provider: Provider::SelfHosted,
        token: device_id.clone(),
    }));

    let Some(endpoint) = endpoint else {
        return Err(Error::RegistrationFailed(
            "PushConfig.fallback_endpoint is required on this platform".into(),
        ));
    };

    // Run the connection on a dedicated thread with its own runtime so we don't
    // depend on an ambient async runtime at the call site.
    std::thread::Builder::new()
        .name("dioxus-push-fallback".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    bridge::emit(PushEvent::TokenError(format!("runtime build failed: {e}")));
                    return;
                }
            };
            rt.block_on(connection_loop(endpoint, device_id));
        })
        .map_err(|e| Error::DeviceError(format!("failed to spawn connection thread: {e}")))?;

    Ok(())
}

/// A frame received from the backend over the live connection.
#[derive(Debug, Deserialize)]
struct IncomingFrame {
    #[serde(default)]
    notification: Option<NotificationContent>,
    #[serde(default)]
    data: std::collections::HashMap<String, String>,
    #[serde(default)]
    message_id: Option<String>,
}

async fn connection_loop(endpoint: String, device_id: String) {
    let mut backoff = Duration::from_secs(1);
    let max_backoff = Duration::from_secs(30);

    loop {
        match run_once(&endpoint, &device_id).await {
            Ok(()) => backoff = Duration::from_secs(1),
            Err(e) => bridge::emit(PushEvent::TokenError(format!("connection error: {e}"))),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(max_backoff);
    }
}

async fn run_once(endpoint: &str, device_id: &str) -> Result<(), String> {
    let (mut ws, _) = tokio_tungstenite::connect_async(endpoint)
        .await
        .map_err(|e| e.to_string())?;

    // Identify ourselves so the backend can route messages to this device.
    let register = serde_json::json!({ "type": "register", "device_id": device_id });
    ws.send(WsMessage::Text(register.to_string()))
        .await
        .map_err(|e| e.to_string())?;

    while let Some(msg) = ws.next().await {
        match msg.map_err(|e| e.to_string())? {
            WsMessage::Text(text) => handle_payload(text.as_bytes()),
            WsMessage::Binary(bytes) => handle_payload(&bytes),
            WsMessage::Ping(_) | WsMessage::Pong(_) | WsMessage::Frame(_) => {}
            WsMessage::Close(_) => break,
        }
    }

    Ok(())
}

fn handle_payload(bytes: &[u8]) {
    let Ok(frame) = serde_json::from_slice::<IncomingFrame>(bytes) else {
        return;
    };

    // Raise a local toast for the display portion.
    if let Some(content) = &frame.notification {
        let mut notification = dioxus_sdk_notification::Notification::new();
        if let Some(title) = &content.title {
            notification.summary(title);
        }
        if let Some(body) = &content.body {
            notification.body(body);
        }
        let _ = notification.show();
    }

    bridge::emit(PushEvent::MessageReceived(RemoteMessage {
        notification: frame.notification,
        data: frame.data,
        message_id: frame.message_id,
    }));
}
