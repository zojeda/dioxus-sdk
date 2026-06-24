//! Android backend using Firebase Cloud Messaging (FCM).
//!
//! Token retrieval and message delivery require a small Kotlin shim
//! (`DioxusFirebaseMessagingService.kt`, shipped under `android_shim/`) declared in the
//! app's `AndroidManifest.xml`, plus `google-services.json` and the
//! `firebase-messaging` gradle dependency. See the crate README for setup.
//!
//! The shim's `external` methods bind to the `#[unsafe(no_mangle)]` JNI exports below;
//! they run on Firebase/Binder threads and simply forward into [`bridge::emit`].

use crate::core::{
    DevicePushToken, Error, NotificationContent, PermissionStatus, Provider, PushConfig, PushEvent,
    RemoteMessage,
};
use crate::platform::bridge;
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jboolean;

pub use bridge::listen;

pub struct PushManager;

impl PushManager {
    pub fn new(_config: PushConfig) -> Result<Self, Error> {
        Ok(Self)
    }
}

/// Request the Android 13+ `POST_NOTIFICATIONS` runtime permission. The result is
/// delivered asynchronously as [`PushEvent::PermissionChanged`].
pub async fn request_permission(_manager: &PushManager) -> Result<PermissionStatus, Error> {
    call_shim_static("requestNotificationPermission");
    Ok(PermissionStatus::Undetermined)
}

/// Begin FCM registration. The token arrives asynchronously as
/// [`PushEvent::TokenRegistered`] via [`Java_dev_dioxus_push_DioxusPush_nativeOnToken`].
pub fn register(_manager: &PushManager) -> Result<(), Error> {
    call_shim_static("register");
    Ok(())
}

/// Dispatch a call to a `DioxusPush.<name>(Context)` static method on the UI thread.
fn call_shim_static(method: &'static str) {
    dioxus::mobile::wry::prelude::dispatch(
        move |env: &mut JNIEnv, activity: &JObject, _webview| {
            let Ok(class) = env.find_class("dev/dioxus/push/DioxusPush") else {
                bridge::emit(PushEvent::TokenError(
                    "DioxusPush shim class not found (is the Kotlin shim included?)".into(),
                ));
                return;
            };
            let _ = env.call_static_method(
                class,
                method,
                "(Landroid/content/Context;)V",
                &[JValue::Object(activity)],
            );
        },
    );
}

/// Read a (possibly null) Java string into an `Option<String>`.
fn read_string(env: &mut JNIEnv, value: &JString) -> Option<String> {
    if value.is_null() {
        return None;
    }
    env.get_string(value).ok().map(|s| s.into())
}

/// `DioxusPush.nativeOnToken(String)` — a fresh or refreshed FCM token.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnToken(
    mut env: JNIEnv,
    _class: JClass,
    token: JString,
) {
    if let Some(token) = read_string(&mut env, &token) {
        bridge::emit(PushEvent::TokenRegistered(DevicePushToken {
            provider: Provider::Fcm,
            token,
        }));
    }
}

/// `DioxusPush.nativeOnTokenError(String)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnTokenError(
    mut env: JNIEnv,
    _class: JClass,
    error: JString,
) {
    let message = read_string(&mut env, &error).unwrap_or_else(|| "unknown error".into());
    bridge::emit(PushEvent::TokenError(message));
}

/// `DioxusPush.nativeOnMessage(String?, String?, String, String?)` —
/// (title, body, dataJson, messageId).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnMessage(
    mut env: JNIEnv,
    _class: JClass,
    title: JString,
    body: JString,
    data_json: JString,
    message_id: JString,
) {
    let title = read_string(&mut env, &title);
    let body = read_string(&mut env, &body);
    let data = read_string(&mut env, &data_json)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    let message_id = read_string(&mut env, &message_id);

    let notification = if title.is_some() || body.is_some() {
        Some(NotificationContent { title, body })
    } else {
        None
    };

    bridge::emit(PushEvent::MessageReceived(RemoteMessage {
        notification,
        data,
        message_id,
    }));
}

/// `DioxusPush.nativeOnPermission(boolean)`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_dioxus_push_DioxusPush_nativeOnPermission(
    _env: JNIEnv,
    _class: JClass,
    granted: jboolean,
) {
    let status = if granted != 0 {
        PermissionStatus::Granted
    } else {
        PermissionStatus::Denied
    };
    bridge::emit(PushEvent::PermissionChanged(status));
}
