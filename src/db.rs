use std::path::Path;

use anyhow::Context;
use chrono::{DateTime, Duration, Utc};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::models::{FailureEvent, NewTarget, PingRollup, ProbeResult, ProbeType, Target};
use crate::status::TargetStatus;

pub async fn connect(path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

#[cfg(test)]
pub async fn connect_memory() -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS targets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            host TEXT NOT NULL,
            probe_type TEXT NOT NULL CHECK (probe_type IN ('icmp', 'tcp')),
            port INTEGER,
            interval_ms INTEGER NOT NULL,
            timeout_ms INTEGER NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            group_name TEXT,
            description TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ping_results (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            target_id INTEGER NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT NOT NULL,
            probe_type TEXT NOT NULL,
            resolved_ip TEXT,
            success INTEGER NOT NULL,
            latency_ms REAL,
            ttl INTEGER,
            error_kind TEXT,
            error_message TEXT,
            FOREIGN KEY(target_id) REFERENCES targets(id) ON DELETE CASCADE
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE INDEX IF NOT EXISTS idx_ping_results_target_finished
        ON ping_results(target_id, finished_at DESC);
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS target_status (
            target_id INTEGER PRIMARY KEY,
            state TEXT NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            consecutive_failures INTEGER NOT NULL,
            last_latency_ms REAL,
            min_latency_ms REAL,
            max_latency_ms REAL,
            avg_latency_ms REAL,
            loss_rate REAL NOT NULL,
            last_success_at TEXT,
            last_failure_at TEXT,
            updated_at TEXT NOT NULL,
            FOREIGN KEY(target_id) REFERENCES targets(id) ON DELETE CASCADE
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS ping_rollup_1m (
            target_id INTEGER NOT NULL,
            bucket_start TEXT NOT NULL,
            success_count INTEGER NOT NULL,
            failure_count INTEGER NOT NULL,
            min_latency_ms REAL,
            max_latency_ms REAL,
            avg_latency_ms REAL,
            latency_sum REAL NOT NULL DEFAULT 0,
            latency_samples INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY(target_id, bucket_start)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("ALTER TABLE ping_rollup_1m ADD COLUMN latency_sum REAL NOT NULL DEFAULT 0")
        .execute(pool)
        .await
        .ok();
    sqlx::query("ALTER TABLE ping_rollup_1m ADD COLUMN latency_samples INTEGER NOT NULL DEFAULT 0")
        .execute(pool)
        .await
        .ok();

    Ok(())
}

pub async fn create_target(pool: &SqlitePool, input: &NewTarget) -> anyhow::Result<Target> {
    input.validate().map_err(|err| anyhow::anyhow!(err))?;
    let result = sqlx::query(
        r#"
        INSERT INTO targets
            (name, host, probe_type, port, interval_ms, timeout_ms, enabled, group_name, description)
        VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(input.name.trim())
    .bind(input.host.trim())
    .bind(input.probe_type.as_str())
    .bind(input.port.map(i64::from))
    .bind(input.interval_ms as i64)
    .bind(input.timeout_ms as i64)
    .bind(input.enabled as i64)
    .bind(input.group_name.as_deref())
    .bind(input.description.as_deref())
    .execute(pool)
    .await?;

    get_target(pool, result.last_insert_rowid()).await
}

pub async fn update_target(
    pool: &SqlitePool,
    id: i64,
    input: &NewTarget,
) -> anyhow::Result<Target> {
    input.validate().map_err(|err| anyhow::anyhow!(err))?;
    sqlx::query(
        r#"
        UPDATE targets
        SET name = ?1, host = ?2, probe_type = ?3, port = ?4, interval_ms = ?5,
            timeout_ms = ?6, enabled = ?7, group_name = ?8, description = ?9,
            updated_at = CURRENT_TIMESTAMP
        WHERE id = ?10
        "#,
    )
    .bind(input.name.trim())
    .bind(input.host.trim())
    .bind(input.probe_type.as_str())
    .bind(input.port.map(i64::from))
    .bind(input.interval_ms as i64)
    .bind(input.timeout_ms as i64)
    .bind(input.enabled as i64)
    .bind(input.group_name.as_deref())
    .bind(input.description.as_deref())
    .bind(id)
    .execute(pool)
    .await?;
    get_target(pool, id).await
}

pub async fn delete_target(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM targets WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn clear_target_data(pool: &SqlitePool, target_id: i64) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM ping_results WHERE target_id = ?1")
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM ping_rollup_1m WHERE target_id = ?1")
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM target_status WHERE target_id = ?1")
        .bind(target_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_target_enabled(
    pool: &SqlitePool,
    id: i64,
    enabled: bool,
) -> anyhow::Result<Target> {
    sqlx::query("UPDATE targets SET enabled = ?1, updated_at = CURRENT_TIMESTAMP WHERE id = ?2")
        .bind(enabled as i64)
        .bind(id)
        .execute(pool)
        .await?;
    get_target(pool, id).await
}

pub async fn get_target(pool: &SqlitePool, id: i64) -> anyhow::Result<Target> {
    let row = sqlx::query(
        "SELECT id, name, host, probe_type, port, interval_ms, timeout_ms, enabled, group_name, description FROM targets WHERE id = ?1",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .with_context(|| format!("target {id} not found"))?;
    target_from_row(&row)
}

pub async fn list_targets(pool: &SqlitePool) -> anyhow::Result<Vec<Target>> {
    let rows = sqlx::query(
        "SELECT id, name, host, probe_type, port, interval_ms, timeout_ms, enabled, group_name, description FROM targets ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    rows.iter().map(target_from_row).collect()
}

pub async fn insert_results(pool: &SqlitePool, results: &[ProbeResult]) -> anyhow::Result<()> {
    if results.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for result in results {
        sqlx::query(
            r#"
            INSERT INTO ping_results
                (target_id, started_at, finished_at, probe_type, resolved_ip, success, latency_ms, ttl, error_kind, error_message)
            VALUES
                (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(result.target_id)
        .bind(result.started_at.to_rfc3339())
        .bind(result.finished_at.to_rfc3339())
        .bind(result.probe_type.as_str())
        .bind(result.resolved_ip.as_deref())
        .bind(result.success as i64)
        .bind(result.latency_ms)
        .bind(result.ttl.map(i64::from))
        .bind(result.error_kind.as_deref())
        .bind(result.error_message.as_deref())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn recent_results(
    pool: &SqlitePool,
    target_id: i64,
    limit: i64,
) -> anyhow::Result<Vec<ProbeResult>> {
    let rows = sqlx::query(
        r#"
        SELECT target_id, started_at, finished_at, probe_type, resolved_ip, success, latency_ms, ttl, error_kind, error_message
        FROM ping_results
        WHERE target_id = ?1
        ORDER BY finished_at DESC
        LIMIT ?2
        "#,
    )
    .bind(target_id)
    .bind(limit.clamp(1, 2000))
    .fetch_all(pool)
    .await?;

    rows.iter().map(result_from_row).collect()
}

pub async fn results_window(
    pool: &SqlitePool,
    target_id: i64,
    minutes: i64,
    end_at: DateTime<Utc>,
) -> anyhow::Result<Vec<ProbeResult>> {
    let minutes = minutes.clamp(1, 43_200);
    let cutoff = end_at - Duration::minutes(minutes);
    let rows = sqlx::query(
        r#"
        SELECT target_id, started_at, finished_at, probe_type, resolved_ip, success, latency_ms, ttl, error_kind, error_message
        FROM ping_results
        WHERE target_id = ?1 AND finished_at >= ?2 AND finished_at <= ?3
        ORDER BY finished_at ASC
        "#,
    )
    .bind(target_id)
    .bind(cutoff.to_rfc3339())
    .bind(end_at.to_rfc3339())
    .fetch_all(pool)
    .await?;

    rows.iter().map(result_from_row).collect()
}

pub async fn failure_events_window(
    pool: &SqlitePool,
    target_id: i64,
    minutes: i64,
    end_at: DateTime<Utc>,
    max_gap_seconds: i64,
) -> anyhow::Result<Vec<FailureEvent>> {
    let results = results_window(pool, target_id, minutes, end_at).await?;
    let mut events = Vec::new();
    let max_gap_seconds = max_gap_seconds.max(1);
    let mut current: Option<FailureEvent> = None;

    for result in results.into_iter().filter(|result| !result.success) {
        let finished_at = result.finished_at;
        match current.as_mut() {
            Some(event)
                if finished_at.signed_duration_since(event.ended_at).num_seconds()
                    <= max_gap_seconds =>
            {
                event.ended_at = finished_at;
                event.failure_count += 1;
                event.duration_seconds = event
                    .ended_at
                    .signed_duration_since(event.started_at)
                    .num_seconds();
            }
            Some(_) => {
                events.push(current.take().expect("current event must exist"));
                current = Some(FailureEvent {
                    target_id,
                    started_at: finished_at,
                    ended_at: finished_at,
                    failure_count: 1,
                    duration_seconds: 0,
                });
            }
            None => {
                current = Some(FailureEvent {
                    target_id,
                    started_at: finished_at,
                    ended_at: finished_at,
                    failure_count: 1,
                    duration_seconds: 0,
                });
            }
        }
    }

    if let Some(event) = current {
        events.push(event);
    }

    Ok(events)
}

pub async fn upsert_status(pool: &SqlitePool, status: &TargetStatus) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO target_status
            (target_id, state, success_count, failure_count, consecutive_failures,
             last_latency_ms, min_latency_ms, max_latency_ms, avg_latency_ms, loss_rate,
             last_success_at, last_failure_at, updated_at)
        VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(target_id) DO UPDATE SET
            state = excluded.state,
            success_count = excluded.success_count,
            failure_count = excluded.failure_count,
            consecutive_failures = excluded.consecutive_failures,
            last_latency_ms = excluded.last_latency_ms,
            min_latency_ms = excluded.min_latency_ms,
            max_latency_ms = excluded.max_latency_ms,
            avg_latency_ms = excluded.avg_latency_ms,
            loss_rate = excluded.loss_rate,
            last_success_at = excluded.last_success_at,
            last_failure_at = excluded.last_failure_at,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(status.target_id)
    .bind(format!("{:?}", status.state).to_lowercase())
    .bind(status.success_count as i64)
    .bind(status.failure_count as i64)
    .bind(status.consecutive_failures as i64)
    .bind(status.last_latency_ms)
    .bind(status.min_latency_ms)
    .bind(status.max_latency_ms)
    .bind(status.avg_latency_ms)
    .bind(status.loss_rate)
    .bind(status.last_success_at.map(|v| v.to_rfc3339()))
    .bind(status.last_failure_at.map(|v| v.to_rfc3339()))
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_rollup_for_result(
    pool: &SqlitePool,
    result: &ProbeResult,
) -> anyhow::Result<()> {
    let bucket_start = minute_bucket(result.finished_at);
    let success_count = i64::from(result.success);
    let failure_count = i64::from(!result.success);
    let latency_sum = result.latency_ms.unwrap_or(0.0);
    let latency_samples = i64::from(result.latency_ms.is_some());

    sqlx::query(
        r#"
        INSERT INTO ping_rollup_1m
            (target_id, bucket_start, success_count, failure_count,
             min_latency_ms, max_latency_ms, avg_latency_ms, latency_sum, latency_samples)
        VALUES
            (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(target_id, bucket_start) DO UPDATE SET
            success_count = success_count + excluded.success_count,
            failure_count = failure_count + excluded.failure_count,
            min_latency_ms = CASE
                WHEN excluded.min_latency_ms IS NULL THEN min_latency_ms
                WHEN min_latency_ms IS NULL THEN excluded.min_latency_ms
                ELSE MIN(min_latency_ms, excluded.min_latency_ms)
            END,
            max_latency_ms = CASE
                WHEN excluded.max_latency_ms IS NULL THEN max_latency_ms
                WHEN max_latency_ms IS NULL THEN excluded.max_latency_ms
                ELSE MAX(max_latency_ms, excluded.max_latency_ms)
            END,
            latency_sum = latency_sum + excluded.latency_sum,
            latency_samples = latency_samples + excluded.latency_samples,
            avg_latency_ms = CASE
                WHEN latency_samples + excluded.latency_samples = 0 THEN NULL
                ELSE (latency_sum + excluded.latency_sum) / (latency_samples + excluded.latency_samples)
            END
        "#,
    )
    .bind(result.target_id)
    .bind(bucket_start.to_rfc3339())
    .bind(success_count)
    .bind(failure_count)
    .bind(result.latency_ms)
    .bind(result.latency_ms)
    .bind(result.latency_ms)
    .bind(latency_sum)
    .bind(latency_samples)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn rollups_1m_window(
    pool: &SqlitePool,
    target_id: i64,
    minutes: i64,
    end_at: DateTime<Utc>,
) -> anyhow::Result<Vec<PingRollup>> {
    let minutes = minutes.clamp(1, 43_200);
    let cutoff = end_at - Duration::minutes(minutes);
    let rows = sqlx::query(
        r#"
        SELECT target_id, bucket_start, success_count, failure_count,
               min_latency_ms, max_latency_ms, avg_latency_ms
        FROM ping_rollup_1m
        WHERE target_id = ?1 AND bucket_start >= ?2 AND bucket_start <= ?3
        ORDER BY bucket_start ASC
        "#,
    )
    .bind(target_id)
    .bind(cutoff.to_rfc3339())
    .bind(end_at.to_rfc3339())
    .fetch_all(pool)
    .await?;

    rows.iter().map(rollup_from_row).collect()
}

pub async fn cleanup_old_results(pool: &SqlitePool, retention_days: i64) -> anyhow::Result<u64> {
    let cutoff = Utc::now() - Duration::days(retention_days.max(1));
    let result = sqlx::query("DELETE FROM ping_results WHERE finished_at < ?1")
        .bind(cutoff.to_rfc3339())
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

fn minute_bucket(value: DateTime<Utc>) -> DateTime<Utc> {
    let timestamp = value.timestamp();
    DateTime::from_timestamp(timestamp - timestamp.rem_euclid(60), 0).unwrap_or(value)
}

fn target_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<Target> {
    let probe_type: String = row.try_get("probe_type")?;
    Ok(Target {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        host: row.try_get("host")?,
        probe_type: parse_probe_type(&probe_type)?,
        port: row.try_get::<Option<i64>, _>("port")?.map(|v| v as u16),
        interval_ms: row.try_get::<i64, _>("interval_ms")? as u64,
        timeout_ms: row.try_get::<i64, _>("timeout_ms")? as u64,
        enabled: row.try_get::<i64, _>("enabled")? != 0,
        group_name: row.try_get("group_name")?,
        description: row.try_get("description")?,
    })
}

fn result_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<ProbeResult> {
    let probe_type: String = row.try_get("probe_type")?;
    let started_at: String = row.try_get("started_at")?;
    let finished_at: String = row.try_get("finished_at")?;
    Ok(ProbeResult {
        target_id: row.try_get("target_id")?,
        started_at: DateTime::parse_from_rfc3339(&started_at)?.with_timezone(&Utc),
        finished_at: DateTime::parse_from_rfc3339(&finished_at)?.with_timezone(&Utc),
        probe_type: parse_probe_type(&probe_type)?,
        resolved_ip: row.try_get("resolved_ip")?,
        success: row.try_get::<i64, _>("success")? != 0,
        latency_ms: row.try_get("latency_ms")?,
        ttl: row.try_get::<Option<i64>, _>("ttl")?.map(|v| v as u8),
        error_kind: row.try_get("error_kind")?,
        error_message: row.try_get("error_message")?,
    })
}

fn rollup_from_row(row: &sqlx::sqlite::SqliteRow) -> anyhow::Result<PingRollup> {
    let bucket_start: String = row.try_get("bucket_start")?;
    Ok(PingRollup {
        target_id: row.try_get("target_id")?,
        bucket_start: DateTime::parse_from_rfc3339(&bucket_start)?.with_timezone(&Utc),
        success_count: row.try_get::<i64, _>("success_count")? as u64,
        failure_count: row.try_get::<i64, _>("failure_count")? as u64,
        min_latency_ms: row.try_get("min_latency_ms")?,
        max_latency_ms: row.try_get("max_latency_ms")?,
        avg_latency_ms: row.try_get("avg_latency_ms")?,
    })
}

fn parse_probe_type(value: &str) -> anyhow::Result<ProbeType> {
    match value {
        "icmp" => Ok(ProbeType::Icmp),
        "tcp" => Ok(ProbeType::Tcp),
        _ => Err(anyhow::anyhow!("unknown probe type: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NewTarget, ProbeResult, ProbeType};

    #[tokio::test]
    async fn creates_targets_and_reads_recent_results() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        insert_results(&pool, &[ProbeResult::success_for_test(target.id, 1.5)])
            .await
            .unwrap();
        let results = recent_results(&pool, target.id, 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].latency_ms, Some(1.5));
    }

    #[tokio::test]
    async fn reads_results_for_a_time_window_in_ascending_time_order() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let mut older = ProbeResult::success_for_test(target.id, 8.0);
        older.started_at = Utc::now() - Duration::minutes(20);
        older.finished_at = older.started_at;

        let mut inside = ProbeResult::failure_for_test(target.id, "timeout");
        inside.started_at = Utc::now() - Duration::minutes(5);
        inside.finished_at = inside.started_at;

        let mut latest = ProbeResult::success_for_test(target.id, 12.0);
        latest.started_at = Utc::now() - Duration::minutes(2);
        latest.finished_at = latest.started_at;

        insert_results(&pool, &[older.clone(), latest.clone(), inside.clone()])
            .await
            .unwrap();

        let end_at = Utc::now();
        let results = results_window(&pool, target.id, 10, end_at).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].finished_at, inside.finished_at);
        assert_eq!(results[1].finished_at, latest.finished_at);
        assert_eq!(results[0].success, false);
        assert_eq!(results[1].latency_ms, Some(12.0));
    }

    #[tokio::test]
    async fn groups_failures_into_time_window_events() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let base = Utc::now() - Duration::minutes(5);

        let mut fail_a1 = ProbeResult::failure_for_test(target.id, "timeout");
        fail_a1.started_at = base;
        fail_a1.finished_at = base;

        let mut fail_a2 = ProbeResult::failure_for_test(target.id, "timeout");
        fail_a2.started_at = base + Duration::seconds(40);
        fail_a2.finished_at = fail_a2.started_at;

        let mut fail_b1 = ProbeResult::failure_for_test(target.id, "timeout");
        fail_b1.started_at = base + Duration::minutes(3);
        fail_b1.finished_at = fail_b1.started_at;

        insert_results(&pool, &[fail_b1.clone(), fail_a2.clone(), fail_a1.clone()])
            .await
            .unwrap();

        let events = failure_events_window(&pool, target.id, 10, Utc::now(), 90)
            .await
            .unwrap();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].failure_count, 2);
        assert_eq!(events[0].duration_seconds, 40);
        assert_eq!(events[1].failure_count, 1);
    }

    #[tokio::test]
    async fn cleanup_removes_rows_older_than_retention() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();
        let mut old = ProbeResult::success_for_test(target.id, 1.0);
        old.started_at = Utc::now() - Duration::days(40);
        old.finished_at = old.started_at;
        insert_results(&pool, &[old]).await.unwrap();
        assert_eq!(cleanup_old_results(&pool, 30).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn clear_target_data_removes_only_selected_target_history() {
        let pool = connect_memory().await.unwrap();
        let target_a = create_target(
            &pool,
            &NewTarget {
                name: "a".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();
        let target_b = create_target(
            &pool,
            &NewTarget {
                name: "b".into(),
                host: "192.168.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let result_a = ProbeResult::success_for_test(target_a.id, 10.0);
        let result_b = ProbeResult::success_for_test(target_b.id, 20.0);
        insert_results(&pool, &[result_a.clone(), result_b.clone()])
            .await
            .unwrap();
        upsert_rollup_for_result(&pool, &result_a).await.unwrap();
        upsert_rollup_for_result(&pool, &result_b).await.unwrap();
        upsert_status(&pool, &TargetStatus::new(target_a.id)).await.unwrap();
        upsert_status(&pool, &TargetStatus::new(target_b.id)).await.unwrap();

        clear_target_data(&pool, target_a.id).await.unwrap();

        assert!(recent_results(&pool, target_a.id, 10).await.unwrap().is_empty());
        assert_eq!(recent_results(&pool, target_b.id, 10).await.unwrap().len(), 1);
        assert!(rollups_1m_window(&pool, target_a.id, 60, Utc::now())
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            rollups_1m_window(&pool, target_b.id, 60, Utc::now())
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn rollup_combines_results_in_the_same_minute() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let ok = ProbeResult::success_for_test(target.id, 12.0);
        let mut failed = ProbeResult::failure_for_test(target.id, "timeout");
        failed.started_at = ok.started_at;
        failed.finished_at = ok.finished_at;

        upsert_rollup_for_result(&pool, &ok).await.unwrap();
        upsert_rollup_for_result(&pool, &failed).await.unwrap();

        let rollups = rollups_1m_window(&pool, target.id, 60, Utc::now()).await.unwrap();
        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].success_count, 1);
        assert_eq!(rollups[0].failure_count, 1);
        assert_eq!(rollups[0].avg_latency_ms, Some(12.0));
    }

    #[tokio::test]
    async fn rollup_window_can_end_before_now() {
        let pool = connect_memory().await.unwrap();
        let target = create_target(
            &pool,
            &NewTarget {
                name: "local".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Icmp,
                port: None,
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let mut old = ProbeResult::success_for_test(target.id, 20.0);
        old.started_at = Utc::now() - Duration::hours(5);
        old.finished_at = old.started_at;
        upsert_rollup_for_result(&pool, &old).await.unwrap();

        let end_at = old.finished_at + Duration::minutes(1);
        let rollups = rollups_1m_window(&pool, target.id, 10, end_at).await.unwrap();

        assert_eq!(rollups.len(), 1);
        assert_eq!(rollups[0].avg_latency_ms, Some(20.0));
    }
}
