//! Per-run event hub (broadcast channels) feeding SSE subscribers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::broadcast;
use uuid::Uuid;

use crate::types::RunEvent;

const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone, Default)]
pub struct EventHub {
    inner: Arc<Mutex<HashMap<Uuid, broadcast::Sender<RunEvent>>>>,
}

impl EventHub {
    pub fn new() -> Self {
        Self::default()
    }

    fn channels(&self) -> MutexGuard<'_, HashMap<Uuid, broadcast::Sender<RunEvent>>> {
        // The map is always left consistent, so a poisoned lock is safe to reuse.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Creates (or replaces) the channel for a run.
    pub fn create(&self, run_id: Uuid) -> broadcast::Sender<RunEvent> {
        let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
        self.channels().insert(run_id, tx.clone());
        tx
    }

    /// Subscribes to a run's events; `None` if the run has no channel.
    pub fn subscribe(&self, run_id: Uuid) -> Option<broadcast::Receiver<RunEvent>> {
        self.channels()
            .get(&run_id)
            .map(broadcast::Sender::subscribe)
    }

    /// Publishes an event; silently dropped when the run has no channel or no subscribers.
    pub fn send(&self, run_id: Uuid, event: RunEvent) {
        let sender = self.channels().get(&run_id).cloned();
        if let Some(sender) = sender {
            let _ = sender.send(event);
        }
    }

    pub fn remove(&self, run_id: Uuid) {
        self.channels().remove(&run_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RunStatus;

    fn finished(status: RunStatus) -> RunEvent {
        RunEvent::RunFinished { status }
    }

    #[tokio::test]
    async fn send_before_subscribe_is_ignored() {
        let hub = EventHub::new();
        let run = Uuid::new_v4();
        hub.create(run);
        hub.send(run, finished(RunStatus::Succeeded)); // no subscribers: no error
        let mut rx = hub.subscribe(run).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn subscribers_receive_events_after_subscribing() {
        let hub = EventHub::new();
        let run = Uuid::new_v4();
        hub.create(run);
        let mut a = hub.subscribe(run).unwrap();
        let mut b = hub.subscribe(run).unwrap();
        hub.send(run, finished(RunStatus::Failed));
        for rx in [&mut a, &mut b] {
            assert!(matches!(
                rx.recv().await.unwrap(),
                RunEvent::RunFinished {
                    status: RunStatus::Failed
                }
            ));
        }
    }

    #[tokio::test]
    async fn events_are_scoped_per_run() {
        let hub = EventHub::new();
        let (r1, r2) = (Uuid::new_v4(), Uuid::new_v4());
        hub.create(r1);
        hub.create(r2);
        let mut rx1 = hub.subscribe(r1).unwrap();
        hub.send(r2, finished(RunStatus::Succeeded));
        assert!(rx1.try_recv().is_err());
    }

    #[tokio::test]
    async fn remove_drops_channel_and_closes_receivers() {
        let hub = EventHub::new();
        let run = Uuid::new_v4();
        hub.create(run);
        let mut rx = hub.subscribe(run).unwrap();
        hub.remove(run);
        assert!(hub.subscribe(run).is_none());
        hub.send(run, finished(RunStatus::Succeeded)); // no-op
        assert!(matches!(
            rx.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
    }

    #[test]
    fn subscribe_unknown_run_is_none() {
        assert!(EventHub::new().subscribe(Uuid::new_v4()).is_none());
    }
}
