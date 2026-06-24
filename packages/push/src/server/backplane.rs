//! Pub/sub backplane used to route live-connection deliveries across instances.

use super::ServerError;
use async_trait::async_trait;
use futures::stream::{BoxStream, StreamExt};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// A publish/subscribe transport. Every backend instance subscribes to the same
/// channel; a publish from any instance is observed by all of them.
#[async_trait]
pub trait Backplane: Send + Sync {
    async fn publish(&self, channel: &str, payload: &[u8]) -> Result<(), ServerError>;
    async fn subscribe(&self, channel: &str) -> Result<BoxStream<'static, Vec<u8>>, ServerError>;
}

/// An in-process backplane built on `tokio::sync::broadcast`. Single process only.
pub struct InProcessBackplane {
    channels: Mutex<HashMap<String, broadcast::Sender<Vec<u8>>>>,
    capacity: usize,
}

impl Default for InProcessBackplane {
    fn default() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            capacity: 1024,
        }
    }
}

impl InProcessBackplane {
    pub fn new() -> Self {
        Self::default()
    }

    fn sender(&self, channel: &str) -> broadcast::Sender<Vec<u8>> {
        let mut channels = self.channels.lock().expect("backplane lock poisoned");
        channels
            .entry(channel.to_string())
            .or_insert_with(|| broadcast::channel(self.capacity).0)
            .clone()
    }
}

#[async_trait]
impl Backplane for InProcessBackplane {
    async fn publish(&self, channel: &str, payload: &[u8]) -> Result<(), ServerError> {
        // A send error just means nobody is subscribed yet; that is not fatal.
        let _ = self.sender(channel).send(payload.to_vec());
        Ok(())
    }

    async fn subscribe(&self, channel: &str) -> Result<BoxStream<'static, Vec<u8>>, ServerError> {
        let rx = self.sender(channel).subscribe();
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(payload) => return Some((payload, rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        });
        Ok(stream.boxed())
    }
}

#[cfg(feature = "redis")]
mod redis_impl {
    use super::*;
    use redis::AsyncCommands;
    use redis::aio::ConnectionManager;

    /// A backplane backed by Redis pub/sub. Shared across backend instances.
    pub struct RedisBackplane {
        client: redis::Client,
        conn: ConnectionManager,
    }

    impl RedisBackplane {
        /// Connect to Redis at the given URL (e.g. `redis://127.0.0.1/`).
        pub async fn connect(url: &str) -> Result<Self, ServerError> {
            let client = redis::Client::open(url)?;
            let conn = client.get_connection_manager().await?;
            Ok(Self { client, conn })
        }
    }

    #[async_trait]
    impl Backplane for RedisBackplane {
        async fn publish(&self, channel: &str, payload: &[u8]) -> Result<(), ServerError> {
            let mut conn = self.conn.clone();
            let _: () = conn.publish(channel, payload).await?;
            Ok(())
        }

        async fn subscribe(
            &self,
            channel: &str,
        ) -> Result<BoxStream<'static, Vec<u8>>, ServerError> {
            let mut pubsub = self.client.get_async_pubsub().await?;
            pubsub.subscribe(channel).await?;
            let stream = pubsub
                .into_on_message()
                .map(|msg| msg.get_payload_bytes().to_vec());
            Ok(stream.boxed())
        }
    }
}

#[cfg(feature = "redis")]
pub use redis_impl::RedisBackplane;
