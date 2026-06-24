//! Process-global event sink shared by every platform backend.
//!
//! Native, service-worker, and background-task callbacks fire from arbitrary threads,
//! outside any Dioxus scope, so they cannot hold the coroutine sender directly. They
//! instead call [`emit`], which forwards to the registered sink or buffers the event
//! until [`listen`] installs one. Buffering is essential for cold-start notification
//! taps that fire before the Dioxus tree has mounted.

use crate::core::{Error, PushEvent};
use std::sync::{Arc, Mutex, OnceLock};

/// A callback that forwards a [`PushEvent`] into the hook coroutine.
pub type Sink = Arc<dyn Fn(PushEvent) + Send + Sync>;

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
static PENDING: OnceLock<Mutex<Vec<PushEvent>>> = OnceLock::new();

fn sink_slot() -> &'static Mutex<Option<Sink>> {
    SINK.get_or_init(|| Mutex::new(None))
}

fn pending_slot() -> &'static Mutex<Vec<PushEvent>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
}

/// Deliver an event to the active sink, or buffer it until one is installed.
pub fn emit(event: PushEvent) {
    if let Ok(slot) = sink_slot().lock()
        && let Some(cb) = slot.as_ref()
    {
        cb(event);
        return;
    }
    if let Ok(mut pending) = pending_slot().lock() {
        pending.push(event);
    }
}

/// Install the sink, replaying any events buffered before it was ready.
pub fn listen(callback: Sink) -> Result<(), Error> {
    if let Ok(mut pending) = pending_slot().lock() {
        for event in pending.drain(..) {
            callback(event);
        }
    }
    match sink_slot().lock() {
        Ok(mut slot) => {
            *slot = Some(callback);
            Ok(())
        }
        Err(_) => Err(Error::DeviceError("event sink lock poisoned".into())),
    }
}
