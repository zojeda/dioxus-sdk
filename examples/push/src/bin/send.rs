//! Demonstrates the direct provider clients (requires real credentials to actually send).
//!
//! Run with: `cargo run -p push-example --features server --bin send`

#[cfg(not(feature = "server"))]
fn main() {
    eprintln!("re-run with `--features server` to build the send demo");
}

#[cfg(feature = "server")]
fn main() {
    println!("see the source for FCM / APNs / Web Push usage (fill in your credentials)");
}

#[cfg(feature = "server")]
#[allow(dead_code)]
mod usage {
    use dioxus_sdk_push::server::{
        ApnsClient, ApnsPayload, Endpoint, FcmClient, Message, ServerError, WebPushClient,
    };
    use std::collections::HashMap;

    // --- FCM (HTTP v1) ---
    async fn send_fcm() -> Result<(), ServerError> {
        let fcm = FcmClient::from_service_account_file("service-account.json").await?;
        fcm.send(
            &Message::to_token("<fcm-token>")
                .notification("Title", "Body")
                .data("key", "value"),
        )
        .await?;
        Ok(())
    }

    // --- APNs (.p8 token auth) ---
    async fn send_apns(p8_pem: &[u8]) -> Result<(), ServerError> {
        let apns = ApnsClient::new_token(p8_pem, "<key-id>", "<team-id>", Endpoint::Production)?;
        apns.send(
            "<device-token>",
            "com.example.app",
            ApnsPayload {
                title: Some("Title".into()),
                body: Some("Body".into()),
                custom: HashMap::new(),
            },
        )
        .await?;
        Ok(())
    }

    // --- Web Push (VAPID) ---
    async fn send_web_push(
        vapid_private_pem: Vec<u8>,
        subscription_json: &str,
    ) -> Result<(), ServerError> {
        let web = WebPushClient::new(vapid_private_pem);
        web.send(subscription_json, None, &HashMap::new()).await?;
        Ok(())
    }
}
