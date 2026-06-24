//! Web (wasm) backend using the W3C Web Push API and a service worker.

use crate::core::{
    DevicePushToken, Error, NotificationContent, NotificationResponse, PermissionStatus, Provider,
    PushConfig, PushEvent, RemoteMessage,
};
use crate::platform::bridge;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};

pub use bridge::listen;

const DEFAULT_SW_URL: &str = "/dioxus-push-sw.js";

pub struct PushManager {
    config: PushConfig,
}

impl PushManager {
    pub fn new(config: PushConfig) -> Result<Self, Error> {
        Ok(Self { config })
    }
}

/// Request browser notification permission.
pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    let promise =
        web_sys::Notification::request_permission().map_err(|e| Error::DeviceError(js_err(&e)))?;
    let result = JsFuture::from(promise)
        .await
        .map_err(|e| Error::DeviceError(js_err(&e)))?;
    let status = match result.as_string().as_deref() {
        Some("granted") => PermissionStatus::Granted,
        Some("denied") => PermissionStatus::Denied,
        _ => PermissionStatus::Undetermined,
    };
    bridge::emit(PushEvent::PermissionChanged(status));
    Ok(status)
}

pub fn register(manager: &PushManager) -> Result<(), Error> {
    let sw_url = manager
        .config
        .web_service_worker_url
        .clone()
        .unwrap_or_else(|| DEFAULT_SW_URL.to_string());
    let vapid_key = manager.config.web_vapid_public_key.clone().ok_or_else(|| {
        Error::RegistrationFailed("PushConfig.web_vapid_public_key is required on web".into())
    })?;

    spawn_local(async move {
        if let Err(e) = subscribe(sw_url, vapid_key).await {
            bridge::emit(PushEvent::TokenError(e));
        }
    });

    Ok(())
}

async fn subscribe(sw_url: String, vapid_key: String) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let container = window.navigator().service_worker();

    // Register (or reuse) the service worker.
    let registration = JsFuture::from(container.register(&sw_url))
        .await
        .map_err(|e| js_err(&e))?
        .dyn_into::<web_sys::ServiceWorkerRegistration>()
        .map_err(|_| "unexpected registration type".to_string())?;

    // Subscribe to push with the VAPID application server key.
    let push_manager = registration.push_manager().map_err(|e| js_err(&e))?;
    let options = web_sys::PushSubscriptionOptionsInit::new();
    options.set_user_visible_only(true);
    options.set_application_server_key(&JsValue::from_str(&vapid_key));

    let subscribe_promise = push_manager
        .subscribe_with_options(&options)
        .map_err(|e| js_err(&e))?;
    let subscription = JsFuture::from(subscribe_promise)
        .await
        .map_err(|e| js_err(&e))?
        .dyn_into::<web_sys::PushSubscription>()
        .map_err(|_| "unexpected subscription type".to_string())?;

    // The whole subscription JSON is the addressing token.
    let json = js_sys::JSON::stringify(&subscription)
        .map_err(|e| js_err(&e))?
        .as_string()
        .ok_or("subscription is not serializable")?;

    bridge::emit(PushEvent::TokenRegistered(DevicePushToken {
        provider: Provider::WebPush,
        token: json,
    }));

    install_message_listener(&container);
    Ok(())
}

/// Shape posted by the service worker to in-page clients.
#[derive(Debug, Deserialize)]
struct SwMessage {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    notification: Option<NotificationContent>,
    #[serde(default)]
    data: std::collections::HashMap<String, String>,
    #[serde(default)]
    message_id: Option<String>,
}

fn install_message_listener(container: &web_sys::ServiceWorkerContainer) {
    let closure =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            let Ok(json) = js_sys::JSON::stringify(&event.data()) else {
                return;
            };
            let Some(json) = json.as_string() else {
                return;
            };
            let Ok(message) = serde_json::from_str::<SwMessage>(&json) else {
                return;
            };

            let remote = RemoteMessage {
                notification: message.notification,
                data: message.data,
                message_id: message.message_id,
            };

            if message.kind == "notificationclick" {
                bridge::emit(PushEvent::NotificationTapped(NotificationResponse {
                    message: remote,
                    action_id: None,
                }));
            } else {
                bridge::emit(PushEvent::MessageReceived(remote));
            }
        });

    container.set_onmessage(Some(closure.as_ref().unchecked_ref()));
    // Keep the closure alive for the lifetime of the page.
    closure.forget();
}

fn js_err(value: &JsValue) -> String {
    value
        .as_string()
        .or_else(|| {
            js_sys::JSON::stringify(value)
                .ok()
                .and_then(|s| s.as_string())
        })
        .unwrap_or_else(|| "unknown JS error".to_string())
}
