//! The fan-out hub: one event, many devices, across instances.
//!
//! Provider sends (FCM / APNs / Web Push) are performed by the instance that receives
//! the `notify_*` call — the registry is shared, so any instance can reach every
//! provider token, and there are no duplicate sends. The backplane is used only to
//! route deliveries for `SelfHosted` devices whose live WebSocket is held by a
//! *different* instance.

use super::{
    ApnsClient, ApnsPayload, Backplane, DeviceRegistry, FcmClient, Frame, InMemoryRegistry,
    InProcessBackplane, LiveConnections, Message, RegisteredDevice, ServerError, TOPIC_ALL,
    WebPushClient,
};
use crate::core::{NotificationContent, Provider};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const LIVE_CHANNEL: &str = "dioxus-push:live";

/// Who a notification is addressed to.
#[derive(Debug, Clone)]
pub enum Target {
    /// Every registered device (the [`TOPIC_ALL`] topic).
    All,
    /// Every device subscribed to a topic.
    Topic(String),
    /// A single device by id.
    Device(String),
}

/// The content of a notification to fan out.
#[derive(Debug, Clone, Default)]
pub struct PushEventMsg {
    pub notification: Option<NotificationContent>,
    pub data: HashMap<String, String>,
}

impl PushEventMsg {
    /// Create a message with a display notification.
    pub fn notification(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            notification: Some(NotificationContent {
                title: Some(title.into()),
                body: Some(body.into()),
            }),
            data: HashMap::new(),
        }
    }

    /// Add a key/value data entry.
    pub fn data(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

#[derive(Serialize, Deserialize)]
struct LiveRoute {
    device_id: String,
    frame: Frame,
}

/// The fan-out hub. Build one with [`PushHub::builder`].
pub struct PushHub {
    registry: Arc<dyn DeviceRegistry>,
    backplane: Arc<dyn Backplane>,
    live: Arc<LiveConnections>,
    fcm: Option<Arc<FcmClient>>,
    apns: Option<Arc<ApnsClient>>,
    web_push: Option<Arc<WebPushClient>>,
    concurrency: usize,
}

impl PushHub {
    /// Start configuring a hub.
    pub fn builder() -> PushHubBuilder {
        PushHubBuilder::default()
    }

    /// The device registry, for the server's registration endpoints.
    pub fn registry(&self) -> &Arc<dyn DeviceRegistry> {
        &self.registry
    }

    /// The live-connection table, for the server's WebSocket accept loop.
    pub fn live(&self) -> &Arc<LiveConnections> {
        &self.live
    }

    /// Notify every registered device.
    pub async fn notify_all(&self, msg: PushEventMsg) -> Result<(), ServerError> {
        self.notify(Target::All, msg).await
    }

    /// Notify every device subscribed to `topic`.
    pub async fn notify_topic(
        &self,
        topic: impl Into<String>,
        msg: PushEventMsg,
    ) -> Result<(), ServerError> {
        self.notify(Target::Topic(topic.into()), msg).await
    }

    /// Notify a single device by id.
    pub async fn notify_device(
        &self,
        device_id: impl Into<String>,
        msg: PushEventMsg,
    ) -> Result<(), ServerError> {
        self.notify(Target::Device(device_id.into()), msg).await
    }

    /// Resolve the target devices and fan out the message with bounded concurrency.
    pub async fn notify(&self, target: Target, msg: PushEventMsg) -> Result<(), ServerError> {
        let devices = match target {
            Target::All => self.registry.devices_for_topic(TOPIC_ALL).await?,
            Target::Topic(topic) => self.registry.devices_for_topic(&topic).await?,
            Target::Device(id) => self.registry.device(&id).await?.into_iter().collect(),
        };

        // Per-device send failures are not fatal to the batch; invalid tokens are
        // pruned inside `dispatch`.
        stream::iter(devices)
            .map(|device| self.dispatch(device, &msg))
            .buffer_unordered(self.concurrency.max(1))
            .for_each(|_result| async {})
            .await;

        Ok(())
    }

    /// Run the backplane consumer loop, delivering routed frames to live connections
    /// this instance owns. Call once per instance and keep it running.
    pub async fn run(&self) -> Result<(), ServerError> {
        let mut stream = self.backplane.subscribe(LIVE_CHANNEL).await?;
        while let Some(payload) = stream.next().await {
            if let Ok(route) = serde_json::from_slice::<LiveRoute>(&payload) {
                self.live.deliver(&route.device_id, &route.frame);
            }
        }
        Ok(())
    }

    async fn dispatch(
        &self,
        device: RegisteredDevice,
        msg: &PushEventMsg,
    ) -> Result<(), ServerError> {
        match device.token.provider {
            Provider::Fcm => {
                let fcm = self
                    .fcm
                    .as_ref()
                    .ok_or_else(|| ServerError::Config("no FCM client configured".into()))?;
                let message = Message {
                    token: device.token.token.clone(),
                    notification: msg.notification.clone(),
                    data: msg.data.clone(),
                };
                self.guard(&device.device_id, fcm.send(&message).await.map(|_| ()))
                    .await
            }
            Provider::Apns => {
                let apns = self
                    .apns
                    .as_ref()
                    .ok_or_else(|| ServerError::Config("no APNs client configured".into()))?;
                let bundle_id = device
                    .bundle_id
                    .clone()
                    .ok_or_else(|| ServerError::Config("device missing bundle_id".into()))?;
                let payload = ApnsPayload {
                    title: msg.notification.as_ref().and_then(|n| n.title.clone()),
                    body: msg.notification.as_ref().and_then(|n| n.body.clone()),
                    custom: msg.data.clone(),
                };
                self.guard(
                    &device.device_id,
                    apns.send(&device.token.token, &bundle_id, payload).await,
                )
                .await
            }
            Provider::WebPush => {
                let web_push = self
                    .web_push
                    .as_ref()
                    .ok_or_else(|| ServerError::Config("no Web Push client configured".into()))?;
                self.guard(
                    &device.device_id,
                    web_push
                        .send(&device.token.token, msg.notification.as_ref(), &msg.data)
                        .await,
                )
                .await
            }
            Provider::SelfHosted => {
                let frame = Frame {
                    notification: msg.notification.clone(),
                    data: msg.data.clone(),
                    message_id: None,
                };
                if !self.live.deliver(&device.device_id, &frame) {
                    let route = LiveRoute {
                        device_id: device.device_id.clone(),
                        frame,
                    };
                    self.backplane
                        .publish(LIVE_CHANNEL, &serde_json::to_vec(&route)?)
                        .await?;
                }
                Ok(())
            }
        }
    }

    /// Treat a provider "token dead" error as a prune-and-continue, not a hard failure.
    async fn guard(
        &self,
        device_id: &str,
        result: Result<(), ServerError>,
    ) -> Result<(), ServerError> {
        match result {
            Ok(()) => Ok(()),
            Err(e) if e.is_token_invalid() => {
                self.registry.unregister(device_id).await?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

/// Builder for [`PushHub`].
#[derive(Default)]
pub struct PushHubBuilder {
    registry: Option<Arc<dyn DeviceRegistry>>,
    backplane: Option<Arc<dyn Backplane>>,
    live: Option<Arc<LiveConnections>>,
    fcm: Option<Arc<FcmClient>>,
    apns: Option<Arc<ApnsClient>>,
    web_push: Option<Arc<WebPushClient>>,
    concurrency: Option<usize>,
}

impl PushHubBuilder {
    /// Use a custom device registry (defaults to [`InMemoryRegistry`]).
    pub fn registry(mut self, registry: Arc<dyn DeviceRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Use a custom backplane (defaults to [`InProcessBackplane`]).
    pub fn backplane(mut self, backplane: Arc<dyn Backplane>) -> Self {
        self.backplane = Some(backplane);
        self
    }

    /// Provide a shared live-connection table (defaults to a fresh one).
    pub fn live(mut self, live: Arc<LiveConnections>) -> Self {
        self.live = Some(live);
        self
    }

    /// Configure the FCM client.
    pub fn fcm(mut self, fcm: FcmClient) -> Self {
        self.fcm = Some(Arc::new(fcm));
        self
    }

    /// Configure the APNs client.
    pub fn apns(mut self, apns: ApnsClient) -> Self {
        self.apns = Some(Arc::new(apns));
        self
    }

    /// Configure the Web Push client.
    pub fn web_push(mut self, web_push: WebPushClient) -> Self {
        self.web_push = Some(Arc::new(web_push));
        self
    }

    /// Set the fan-out concurrency (defaults to 32).
    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = Some(concurrency);
        self
    }

    /// Build the hub.
    pub fn build(self) -> PushHub {
        PushHub {
            registry: self
                .registry
                .unwrap_or_else(|| Arc::new(InMemoryRegistry::new())),
            backplane: self
                .backplane
                .unwrap_or_else(|| Arc::new(InProcessBackplane::new())),
            live: self.live.unwrap_or_default(),
            fcm: self.fcm,
            apns: self.apns,
            web_push: self.web_push,
            concurrency: self.concurrency.unwrap_or(32),
        }
    }
}
