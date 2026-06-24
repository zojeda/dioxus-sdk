//! Demonstrates the server-side fan-out hub end-to-end, with no provider credentials.
//!
//! Run with: `cargo run -p push-example --features server --bin hub`
//! (add `--features redis` and set `REDIS_URL` to use a Redis backplane/registry).

#[cfg(not(feature = "server"))]
fn main() {
    eprintln!("re-run with `--features server` to build the hub demo");
}

#[cfg(feature = "server")]
#[tokio::main]
async fn main() {
    use dioxus_sdk_push::server::{
        DeviceRegistry, InMemoryRegistry, LiveConnections, PushEventMsg, PushHub, RegisteredDevice,
    };
    use dioxus_sdk_push::{DevicePushToken, Provider};
    use std::sync::Arc;

    // A SelfHosted device with a live connection held by this instance.
    let registry = Arc::new(InMemoryRegistry::new());
    let device = RegisteredDevice::new(
        "device-1",
        DevicePushToken {
            provider: Provider::SelfHosted,
            token: "device-1".into(),
        },
    );
    registry.register(device).await.unwrap();
    registry.subscribe("device-1", "news").await.unwrap();

    let live = LiveConnections::new();
    let mut rx = live.connect("device-1");

    let hub = PushHub::builder().registry(registry).live(live).build();

    // Fan out to the "news" topic.
    hub.notify_topic(
        "news",
        PushEventMsg::notification("Breaking", "Hello from the hub"),
    )
    .await
    .unwrap();

    // The live connection received the frame the server would push down its socket.
    if let Some(frame) = rx.recv().await {
        println!("delivered to device-1: {frame:?}");
    }
}
