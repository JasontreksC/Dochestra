//! Observer that persists node state to the DB, then broadcasts it (persist-then-publish).

use std::collections::HashMap;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::PgPool;
use uuid::Uuid;

use super::events::EventHub;
use super::{NodeUpdate, RunObserver};
use crate::db::runs;
use crate::types::{NodeKind, RunEvent};

pub struct DbObserver {
    pub db: PgPool,
    pub events: EventHub,
    pub run_id: Uuid,
    /// Kind per planned node id (needed to order API outputs by catalog).
    pub kinds: HashMap<String, NodeKind>,
}

impl RunObserver for DbObserver {
    fn node_changed(&self, update: NodeUpdate) -> BoxFuture<'static, ()> {
        let db = self.db.clone();
        let events = self.events.clone();
        let run_id = self.run_id;
        let kind = self.kinds.get(&update.node_id).copied();
        async move {
            if let Err(err) = runs::update_node(
                &db,
                run_id,
                &update.node_id,
                update.status,
                update.error.as_deref(),
                update.outputs.as_ref(),
            )
            .await
            {
                tracing::error!(%run_id, node_id = %update.node_id, error = ?err, "failed to persist node state");
            }

            let outputs = match (&update.outputs, kind) {
                (Some(outputs), Some(kind)) => {
                    runs::to_node_outputs(run_id, &update.node_id, kind, outputs)
                }
                _ => Vec::new(),
            };
            events.send(
                run_id,
                RunEvent::NodeStatus {
                    node_id: update.node_id,
                    status: update.status,
                    error: update.error,
                    outputs,
                },
            );
        }
        .boxed()
    }
}
