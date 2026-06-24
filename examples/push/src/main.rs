use dioxus::prelude::*;
use dioxus_sdk_push::{
    init_push_manager, use_push_notifications, use_request_permission, PushConfig,
};

fn main() {
    launch(App);
}

#[component]
fn App() -> Element {
    // Configure per-platform setup. Only the fields relevant to the running target
    // are used; the example degrades gracefully where push is unavailable.
    init_push_manager(PushConfig {
        web_vapid_public_key: Some("<your-vapid-public-key>".into()),
        fallback_endpoint: Some("ws://127.0.0.1:8080/push".into()),
        ..Default::default()
    });

    let state = use_push_notifications();
    let mut request_permission = use_request_permission();

    rsx!(
        div {
            style: "text-align: center; font-family: sans-serif;",
            h1 { "🔔 Dioxus Push Notifications Example" }

            button { onclick: move |_| request_permission(), "Request permission" }

            h3 { "Permission" }
            p { "{state().permission:?}" }

            h3 { "Device token" }
            match state().token {
                Some(token) => rsx!( p { "{token.provider:?}: {token.token}" } ),
                None => rsx!( p { "Registering…" } ),
            }

            h3 { "Last message" }
            match state().last_message {
                Some(message) => rsx!( pre { "{message:#?}" } ),
                None => rsx!( p { "None yet" } ),
            }

            if let Some(error) = state().error {
                p { style: "color: #b00;", "Error: {error}" }
            }
        }
    )
}
