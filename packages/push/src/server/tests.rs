use super::*;
use crate::core::{DevicePushToken, Provider};
use std::sync::Arc;

fn self_hosted(id: &str) -> RegisteredDevice {
    RegisteredDevice::new(
        id,
        DevicePushToken {
            provider: Provider::SelfHosted,
            token: id.to_string(),
        },
    )
}

#[test]
fn fcm_message_serializes_to_v1_shape() {
    let message = Message::to_token("tok")
        .notification("Hello", "World")
        .data("k", "v");
    let value = serde_json::to_value(&message).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "token": "tok",
            "notification": { "title": "Hello", "body": "World" },
            "data": { "k": "v" }
        })
    );
}

#[test]
fn fcm_message_omits_empty_fields() {
    let value = serde_json::to_value(Message::to_token("tok")).unwrap();
    assert_eq!(value, serde_json::json!({ "token": "tok" }));
}

#[test]
fn token_invalid_detection() {
    assert!(
        ServerError::FcmRejected {
            status: 404,
            body: String::new()
        }
        .is_token_invalid()
    );
    assert!(
        ServerError::FcmRejected {
            status: 400,
            body: "messaging/UNREGISTERED".into()
        }
        .is_token_invalid()
    );
    assert!(ServerError::TokenInvalid.is_token_invalid());
    assert!(!ServerError::Config("x".into()).is_token_invalid());
}

#[tokio::test]
async fn registry_register_subscribe_query() {
    let registry = InMemoryRegistry::new();
    registry.register(self_hosted("d1")).await.unwrap();
    registry.subscribe("d1", "news").await.unwrap();

    // Implicitly subscribed to TOPIC_ALL.
    assert_eq!(
        registry.devices_for_topic(TOPIC_ALL).await.unwrap().len(),
        1
    );
    assert_eq!(registry.devices_for_topic("news").await.unwrap().len(), 1);
    assert!(
        registry
            .devices_for_topic("absent")
            .await
            .unwrap()
            .is_empty()
    );

    registry.unsubscribe("d1", "news").await.unwrap();
    assert!(registry.devices_for_topic("news").await.unwrap().is_empty());

    registry.unregister("d1").await.unwrap();
    assert!(
        registry
            .devices_for_topic(TOPIC_ALL)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn notify_self_hosted_delivers_to_local_connection() {
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register(self_hosted("d1")).await.unwrap();

    let live = LiveConnections::new();
    let mut rx = live.connect("d1");

    let hub = PushHub::builder().registry(registry).live(live).build();

    hub.notify_device("d1", PushEventMsg::notification("Title", "Body"))
        .await
        .unwrap();

    let frame = rx.recv().await.expect("frame delivered");
    assert_eq!(frame.notification.unwrap().title.as_deref(), Some("Title"));
}

#[tokio::test]
async fn notify_topic_reaches_subscribed_devices_only() {
    let registry = Arc::new(InMemoryRegistry::new());
    registry.register(self_hosted("d1")).await.unwrap();
    registry.register(self_hosted("d2")).await.unwrap();
    registry.subscribe("d1", "sports").await.unwrap();

    let live = LiveConnections::new();
    let mut rx1 = live.connect("d1");
    let mut rx2 = live.connect("d2");

    let hub = PushHub::builder().registry(registry).live(live).build();

    hub.notify_topic("sports", PushEventMsg::notification("Goal", "!"))
        .await
        .unwrap();

    assert!(rx1.recv().await.is_some());
    // d2 is not subscribed to "sports"; nothing should be queued for it.
    assert!(rx2.try_recv().is_err());
}
