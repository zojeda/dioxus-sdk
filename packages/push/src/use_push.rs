//! Initialization and consumption hooks for push notifications.

use super::core::{Error, PermissionStatus, PushConfig, PushEvent, PushManager, PushState};
use dioxus::{
    prelude::{
        Coroutine, ReadSignal, Signal, UnboundedReceiver, provide_context, spawn,
        try_consume_context, use_context, use_coroutine, use_hook, use_signal,
    },
    signals::{ReadableExt, WritableExt},
};
use futures_util::stream::StreamExt;
use std::sync::Once;

static INIT: Once = Once::new();

/// Must be called once, before any use of the push abstraction (e.g. near the app root).
///
/// ```no_run
/// use dioxus::prelude::*;
/// use dioxus_sdk_push::{init_push_manager, PushConfig};
///
/// fn app() -> Element {
///     init_push_manager(PushConfig {
///         fallback_endpoint: Some("wss://example.com/push".into()),
///         ..Default::default()
///     });
///     rsx! { "ready" }
/// }
/// ```
pub fn init_push_manager(config: PushConfig) -> Signal<Result<PushManager, Error>> {
    use_hook(|| provide_context(Signal::new(PushManager::new(config))))
}

/// Reactive push state: device token, permission, last received message, last tap, error.
///
/// On unsupported platforms the returned state carries [`Error::Unsupported`] so the UI
/// can degrade gracefully.
pub fn use_push_notifications() -> ReadSignal<PushState> {
    let mut state: Signal<PushState> = use_signal(PushState::default);

    // Drain native events into the reactive state.
    let listener: Coroutine<PushEvent> =
        use_coroutine(move |mut rx: UnboundedReceiver<PushEvent>| async move {
            while let Some(event) = rx.next().await {
                let mut s = state.write();
                match event {
                    PushEvent::TokenRegistered(token) => {
                        s.token = Some(token);
                        s.error = None;
                    }
                    PushEvent::TokenError(e) => {
                        s.error = Some(Error::RegistrationFailed(e));
                    }
                    PushEvent::MessageReceived(message) => {
                        s.last_message = Some(message);
                    }
                    PushEvent::NotificationTapped(response) => {
                        s.last_tap = Some(response);
                    }
                    PushEvent::PermissionChanged(permission) => {
                        s.permission = Some(permission);
                    }
                }
            }
        });

    // Wire the listener to the manager exactly once.
    match try_consume_context::<Signal<Result<PushManager, Error>>>() {
        Some(manager) => {
            let manager = manager.read();
            match manager.as_ref() {
                Ok(manager) => {
                    INIT.call_once(|| {
                        manager.listen(listener).ok();
                        manager.register().ok();
                    });
                }
                Err(e) => state.set(PushState {
                    error: Some(e.clone()),
                    ..Default::default()
                }),
            }
        }
        None => {
            state.write().error = Some(Error::NotInitialized);
        }
    }

    use_hook(|| ReadSignal::new(state))
}

/// Returns a callback that requests OS notification permission when invoked.
///
/// The resulting [`PermissionStatus`] is also delivered through the listener as
/// [`PushEvent::PermissionChanged`], so [`use_push_notifications`] stays in sync.
pub fn use_request_permission() -> impl FnMut() + Clone {
    let manager = use_context::<Signal<Result<PushManager, Error>>>();
    move || {
        spawn(async move {
            let guard = manager.read();
            if let Ok(manager) = guard.as_ref() {
                let _ = manager.request_permission().await;
            }
        });
    }
}

/// Imperatively await the current permission request result.
pub async fn request_permission(
    manager: &Signal<Result<PushManager, Error>>,
) -> Result<PermissionStatus, Error> {
    let manager = manager.read();
    match manager.as_ref() {
        Ok(manager) => manager.request_permission().await,
        Err(e) => Err(e.clone()),
    }
}
