use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{Mutex, RwLock, broadcast};
use tokio::task::JoinHandle;

use crate::db;
use crate::models::{ProbeResult, Target};
use crate::probe::{DefaultProbe, Probe};
use crate::status::TargetStatus;

pub type SharedStatuses = Arc<RwLock<HashMap<i64, TargetStatus>>>;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Status { status: TargetStatus },
    Result { result: ProbeResult },
    TargetsChanged,
}

#[derive(Clone)]
pub struct Scheduler {
    pool: SqlitePool,
    statuses: SharedStatuses,
    events: broadcast::Sender<ServerEvent>,
    tasks: Arc<Mutex<HashMap<i64, JoinHandle<()>>>>,
}

impl Scheduler {
    pub fn new(
        pool: SqlitePool,
        statuses: SharedStatuses,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        Self {
            pool,
            statuses,
            events,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn start_existing_targets(&self) -> anyhow::Result<()> {
        for target in db::list_targets(&self.pool).await? {
            if target.enabled {
                self.start_or_replace(target).await;
            } else {
                self.statuses
                    .write()
                    .await
                    .insert(target.id, TargetStatus::disabled(target.id));
            }
        }
        Ok(())
    }

    pub async fn start_or_replace(&self, target: Target) {
        self.stop(target.id).await;
        if !target.enabled {
            let status = TargetStatus::disabled(target.id);
            self.statuses
                .write()
                .await
                .insert(target.id, status.clone());
            let _ = self.events.send(ServerEvent::Status { status });
            return;
        }

        self.statuses
            .write()
            .await
            .entry(target.id)
            .or_insert_with(|| TargetStatus::new(target.id));

        let target_id = target.id;
        let pool = self.pool.clone();
        let statuses = self.statuses.clone();
        let events = self.events.clone();
        let handle = tokio::spawn(async move {
            let probe = DefaultProbe;
            loop {
                let result = probe.probe(&target).await;

                let status = {
                    let mut guard = statuses.write().await;
                    let status = guard
                        .entry(target.id)
                        .or_insert_with(|| TargetStatus::new(target.id));
                    status.apply_result(&result);
                    status.clone()
                };

                if let Err(err) = db::insert_results(&pool, std::slice::from_ref(&result)).await {
                    tracing::warn!(target_id = target.id, error = %err, "failed to insert probe result");
                }
                if let Err(err) = db::upsert_status(&pool, &status).await {
                    tracing::warn!(target_id = target.id, error = %err, "failed to upsert status");
                }
                if let Err(err) = db::upsert_rollup_for_result(&pool, &result).await {
                    tracing::warn!(target_id = target.id, error = %err, "failed to upsert rollup");
                }

                let _ = events.send(ServerEvent::Result { result });
                let _ = events.send(ServerEvent::Status { status });

                tokio::time::sleep(Duration::from_millis(target.interval_ms.max(250))).await;
            }
        });

        self.tasks.lock().await.insert(target_id, handle);
    }

    pub async fn stop(&self, target_id: i64) {
        if let Some(handle) = self.tasks.lock().await.remove(&target_id) {
            handle.abort();
        }
    }

    pub async fn stop_all(&self) {
        let mut tasks = self.tasks.lock().await;
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }
}

pub async fn cleanup_loop(pool: SqlitePool, retention_days: i64) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 60));
    loop {
        interval.tick().await;
        match db::cleanup_old_results(&pool, retention_days).await {
            Ok(rows) if rows > 0 => tracing::info!(rows, "cleaned old probe results"),
            Ok(_) => {}
            Err(err) => tracing::warn!(error = %err, "retention cleanup failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;

    use crate::models::{ProbeResult, ProbeType, Target};
    use crate::probe::Probe;

    struct FakeProbe {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Probe for FakeProbe {
        async fn probe(&self, target: &Target) -> ProbeResult {
            self.calls.fetch_add(1, Ordering::SeqCst);
            ProbeResult {
                target_id: target.id,
                started_at: Utc::now(),
                finished_at: Utc::now(),
                probe_type: target.probe_type,
                resolved_ip: Some("127.0.0.1".into()),
                success: true,
                latency_ms: Some(1.0),
                ttl: None,
                error_kind: None,
                error_message: None,
            }
        }
    }

    #[tokio::test]
    async fn fake_probe_returns_success() {
        let probe = FakeProbe {
            calls: AtomicUsize::new(0),
        };
        let target = Target {
            id: 1,
            name: "local".into(),
            host: "127.0.0.1".into(),
            probe_type: ProbeType::Tcp,
            port: Some(80),
            interval_ms: 1000,
            timeout_ms: 1000,
            enabled: true,
            group_name: None,
            description: None,
        };
        assert!(probe.probe(&target).await.success);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 1);
    }
}
