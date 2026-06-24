//! Device registry: who is registered, with which token, on which topics.

use super::{ServerError, TOPIC_ALL};
use crate::core::DevicePushToken;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;

/// A device known to the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredDevice {
    /// App-stable primary key.
    pub device_id: String,
    /// The provider token used to reach this device.
    pub token: DevicePushToken,
    /// Topics this device is subscribed to (always includes [`TOPIC_ALL`]).
    #[serde(default)]
    pub topics: Vec<String>,
    /// APNs `apns-topic` (the app bundle id), required for APNs sends.
    #[serde(default)]
    pub bundle_id: Option<String>,
}

impl RegisteredDevice {
    /// Create a device subscribed to the catch-all topic.
    pub fn new(device_id: impl Into<String>, token: DevicePushToken) -> Self {
        Self {
            device_id: device_id.into(),
            token,
            topics: vec![TOPIC_ALL.to_string()],
            bundle_id: None,
        }
    }

    /// Set the APNs bundle id.
    pub fn bundle_id(mut self, bundle_id: impl Into<String>) -> Self {
        self.bundle_id = Some(bundle_id.into());
        self
    }
}

/// Storage for registered devices and their topic membership.
#[async_trait]
pub trait DeviceRegistry: Send + Sync {
    async fn register(&self, device: RegisteredDevice) -> Result<(), ServerError>;
    async fn unregister(&self, device_id: &str) -> Result<(), ServerError>;
    async fn subscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError>;
    async fn unsubscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError>;
    async fn devices_for_topic(&self, topic: &str) -> Result<Vec<RegisteredDevice>, ServerError>;
    async fn device(&self, device_id: &str) -> Result<Option<RegisteredDevice>, ServerError>;
}

/// An in-process registry backed by a `HashMap`. Good for single-instance and tests.
#[derive(Default)]
pub struct InMemoryRegistry {
    devices: RwLock<HashMap<String, RegisteredDevice>>,
}

impl InMemoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn poisoned() -> ServerError {
        ServerError::Config("registry lock poisoned".into())
    }
}

#[async_trait]
impl DeviceRegistry for InMemoryRegistry {
    async fn register(&self, mut device: RegisteredDevice) -> Result<(), ServerError> {
        if !device.topics.iter().any(|t| t == TOPIC_ALL) {
            device.topics.push(TOPIC_ALL.to_string());
        }
        let mut devices = self.devices.write().map_err(|_| Self::poisoned())?;
        devices.insert(device.device_id.clone(), device);
        Ok(())
    }

    async fn unregister(&self, device_id: &str) -> Result<(), ServerError> {
        let mut devices = self.devices.write().map_err(|_| Self::poisoned())?;
        devices.remove(device_id);
        Ok(())
    }

    async fn subscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError> {
        let mut devices = self.devices.write().map_err(|_| Self::poisoned())?;
        if let Some(device) = devices.get_mut(device_id)
            && !device.topics.iter().any(|t| t == topic)
        {
            device.topics.push(topic.to_string());
        }
        Ok(())
    }

    async fn unsubscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError> {
        let mut devices = self.devices.write().map_err(|_| Self::poisoned())?;
        if let Some(device) = devices.get_mut(device_id) {
            device.topics.retain(|t| t != topic);
        }
        Ok(())
    }

    async fn devices_for_topic(&self, topic: &str) -> Result<Vec<RegisteredDevice>, ServerError> {
        let devices = self.devices.read().map_err(|_| Self::poisoned())?;
        Ok(devices
            .values()
            .filter(|d| d.topics.iter().any(|t| t == topic))
            .cloned()
            .collect())
    }

    async fn device(&self, device_id: &str) -> Result<Option<RegisteredDevice>, ServerError> {
        let devices = self.devices.read().map_err(|_| Self::poisoned())?;
        Ok(devices.get(device_id).cloned())
    }
}

#[cfg(feature = "redis")]
mod redis_impl {
    use super::*;
    use redis::AsyncCommands;
    use redis::aio::ConnectionManager;

    fn device_key(device_id: &str) -> String {
        format!("push:dev:{device_id}")
    }

    fn topic_key(topic: &str) -> String {
        format!("push:topic:{topic}")
    }

    /// A registry backed by Redis: device JSON under `push:dev:{id}`, topic membership
    /// as sets under `push:topic:{t}`. Shared across backend instances.
    pub struct RedisRegistry {
        conn: ConnectionManager,
    }

    impl RedisRegistry {
        /// Connect to Redis at the given URL (e.g. `redis://127.0.0.1/`).
        pub async fn connect(url: &str) -> Result<Self, ServerError> {
            let client = redis::Client::open(url)?;
            let conn = client.get_connection_manager().await?;
            Ok(Self { conn })
        }
    }

    #[async_trait]
    impl DeviceRegistry for RedisRegistry {
        async fn register(&self, mut device: RegisteredDevice) -> Result<(), ServerError> {
            if !device.topics.iter().any(|t| t == TOPIC_ALL) {
                device.topics.push(TOPIC_ALL.to_string());
            }
            let mut conn = self.conn.clone();
            let json = serde_json::to_string(&device)?;
            let _: () = conn.set(device_key(&device.device_id), json).await?;
            for topic in &device.topics {
                let _: () = conn.sadd(topic_key(topic), &device.device_id).await?;
            }
            Ok(())
        }

        async fn unregister(&self, device_id: &str) -> Result<(), ServerError> {
            let mut conn = self.conn.clone();
            if let Some(device) = self.device(device_id).await? {
                for topic in &device.topics {
                    let _: () = conn.srem(topic_key(topic), device_id).await?;
                }
            }
            let _: () = conn.del(device_key(device_id)).await?;
            Ok(())
        }

        async fn subscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError> {
            let mut conn = self.conn.clone();
            if let Some(mut device) = self.device(device_id).await? {
                if !device.topics.iter().any(|t| t == topic) {
                    device.topics.push(topic.to_string());
                    let _: () = conn
                        .set(device_key(device_id), serde_json::to_string(&device)?)
                        .await?;
                }
                let _: () = conn.sadd(topic_key(topic), device_id).await?;
            }
            Ok(())
        }

        async fn unsubscribe(&self, device_id: &str, topic: &str) -> Result<(), ServerError> {
            let mut conn = self.conn.clone();
            if let Some(mut device) = self.device(device_id).await? {
                device.topics.retain(|t| t != topic);
                let _: () = conn
                    .set(device_key(device_id), serde_json::to_string(&device)?)
                    .await?;
            }
            let _: () = conn.srem(topic_key(topic), device_id).await?;
            Ok(())
        }

        async fn devices_for_topic(
            &self,
            topic: &str,
        ) -> Result<Vec<RegisteredDevice>, ServerError> {
            let mut conn = self.conn.clone();
            let ids: Vec<String> = conn.smembers(topic_key(topic)).await?;
            if ids.is_empty() {
                return Ok(Vec::new());
            }
            let keys: Vec<String> = ids.iter().map(|id| device_key(id)).collect();
            let jsons: Vec<Option<String>> = conn.mget(keys).await?;
            Ok(jsons
                .into_iter()
                .flatten()
                .filter_map(|j| serde_json::from_str(&j).ok())
                .collect())
        }

        async fn device(&self, device_id: &str) -> Result<Option<RegisteredDevice>, ServerError> {
            let mut conn = self.conn.clone();
            let json: Option<String> = conn.get(device_key(device_id)).await?;
            match json {
                Some(j) => Ok(Some(serde_json::from_str(&j)?)),
                None => Ok(None),
            }
        }
    }
}

#[cfg(feature = "redis")]
pub use redis_impl::RedisRegistry;
