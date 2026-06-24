//! Platform-agnostic types and the [`PushManager`] facade for remote push notifications.

use super::platform;
use core::fmt;
use dioxus::prelude::Coroutine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Identifies which push service issued a [`DevicePushToken`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    /// Firebase Cloud Messaging (Android).
    Fcm,
    /// Apple Push Notification service (iOS / macOS).
    Apns,
    /// W3C Web Push (browsers / wasm).
    WebPush,
    /// App-managed live connection (Windows / Linux fallback).
    SelfHosted,
}

/// The device's push registration handle.
///
/// `token` carries whatever addressing information the provider issued, ready to be
/// forwarded verbatim to your backend:
/// - [`Provider::Fcm`] — the FCM registration token.
/// - [`Provider::Apns`] — the APNs device token, hex-encoded.
/// - [`Provider::WebPush`] — the JSON-serialized `PushSubscription` (endpoint + keys).
/// - [`Provider::SelfHosted`] — a stable device id used to route over the live connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePushToken {
    pub provider: Provider,
    pub token: String,
}

/// The display portion of a push, when present.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// A received remote message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteMessage {
    /// The display block, if the push carried one.
    pub notification: Option<NotificationContent>,
    /// Arbitrary key/value data payload.
    pub data: HashMap<String, String>,
    /// Provider message id, when available.
    pub message_id: Option<String>,
}

/// Delivered when the user taps a notification.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationResponse {
    pub message: RemoteMessage,
    /// Action identifier, or `None` for a default tap.
    pub action_id: Option<String>,
}

/// Runtime permission state for receiving notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionStatus {
    /// Not yet requested.
    Undetermined,
    Granted,
    Denied,
    /// Provisional / quiet authorization (iOS only).
    Provisional,
}

/// An event delivered from a native (or service worker) callback into the hook coroutine.
#[derive(Debug, Clone)]
pub enum PushEvent {
    /// A device token was obtained or refreshed.
    TokenRegistered(DevicePushToken),
    /// Token registration failed.
    TokenError(String),
    /// A message arrived (foreground, or a data message).
    MessageReceived(RemoteMessage),
    /// The user tapped a notification.
    NotificationTapped(NotificationResponse),
    /// The permission state changed.
    PermissionChanged(PermissionStatus),
}

/// The reactive snapshot exposed by [`use_push_notifications`](crate::use_push_notifications).
#[derive(Debug, Clone, Default)]
pub struct PushState {
    pub token: Option<DevicePushToken>,
    pub permission: Option<PermissionStatus>,
    pub last_message: Option<RemoteMessage>,
    pub last_tap: Option<NotificationResponse>,
    pub error: Option<Error>,
}

/// Per-platform configuration supplied by the app at initialization.
///
/// Only the fields relevant to the running target are read; the rest are ignored.
#[derive(Debug, Clone, Default)]
pub struct PushConfig {
    /// Web: the VAPID public key used as `applicationServerKey`.
    pub web_vapid_public_key: Option<String>,
    /// Web: URL the app hosts the service worker at (defaults to `/dioxus-push-sw.js`).
    pub web_service_worker_url: Option<String>,
    /// Windows / Linux: the `ws://` / `wss://` backend endpoint for the live connection.
    pub fallback_endpoint: Option<String>,
    /// Windows / Linux: a stable device id (one is generated if omitted).
    pub fallback_device_id: Option<String>,
}

/// The push notifications abstraction.
pub struct PushManager {
    inner: platform::PushManager,
}

impl PushManager {
    /// Create a new push manager for the current platform.
    pub fn new(config: PushConfig) -> Result<Self, Error> {
        Ok(Self {
            inner: platform::PushManager::new(config)?,
        })
    }

    /// Request OS notification permission, resolving once the user responds.
    pub async fn request_permission(&self) -> Result<PermissionStatus, Error> {
        platform::request_permission(&self.inner).await
    }

    /// Begin registration with the platform push service. The resulting token arrives
    /// via the listener as [`PushEvent::TokenRegistered`].
    pub fn register(&self) -> Result<(), Error> {
        platform::register(&self.inner)
    }

    /// Subscribe a coroutine to push events.
    pub fn listen(&self, listener: Coroutine<PushEvent>) -> Result<(), Error> {
        let tx = listener.tx();
        platform::listen(Arc::new(move |event: PushEvent| {
            tx.unbounded_send(event).ok();
        }))
    }
}

/// Errors that may occur when using the push notifications abstraction.
#[derive(Debug, Clone)]
pub enum Error {
    /// The push manager was not initialized (call `init_push_manager` first).
    NotInitialized,
    /// The user denied notification permission.
    PermissionDenied,
    /// The platform push service rejected registration.
    RegistrationFailed(String),
    /// An underlying platform / JNI / objc / connection error.
    DeviceError(String),
    /// The current platform is not supported.
    Unsupported,
}

impl std::error::Error for Error {}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::NotInitialized => write!(f, "push manager not initialized"),
            Error::PermissionDenied => write!(f, "notification permission denied"),
            Error::RegistrationFailed(e) => write!(f, "push registration failed: {e}"),
            Error::DeviceError(e) => write!(f, "a device error occurred: {e}"),
            Error::Unsupported => write!(f, "the current platform is not supported"),
        }
    }
}
