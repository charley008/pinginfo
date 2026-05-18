use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "TEXT", rename_all = "snake_case")]
pub enum ProbeType {
    Icmp,
    Tcp,
}

impl ProbeType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Icmp => "icmp",
            Self::Tcp => "tcp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetState {
    Healthy,
    Warning,
    Down,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTarget {
    pub name: String,
    pub host: String,
    pub probe_type: ProbeType,
    pub port: Option<u16>,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub enabled: bool,
    pub group_name: Option<String>,
    pub description: Option<String>,
}

impl NewTarget {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name is required".into());
        }
        if self.host.trim().is_empty() {
            return Err("host is required".into());
        }
        if self.probe_type == ProbeType::Tcp && self.port.is_none() {
            return Err("tcp targets require a port".into());
        }
        if self.probe_type == ProbeType::Icmp && self.port.is_some() {
            return Err("icmp targets must not set a port".into());
        }
        if self.interval_ms < 250 {
            return Err("interval_ms must be at least 250".into());
        }
        if self.timeout_ms == 0 {
            return Err("timeout_ms must be greater than 0".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub probe_type: ProbeType,
    pub port: Option<u16>,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub enabled: bool,
    pub group_name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub target_id: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub probe_type: ProbeType,
    pub resolved_ip: Option<String>,
    pub success: bool,
    pub latency_ms: Option<f64>,
    pub ttl: Option<u8>,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingRollup {
    pub target_id: i64,
    pub bucket_start: DateTime<Utc>,
    pub success_count: u64,
    pub failure_count: u64,
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub avg_latency_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureEvent {
    pub target_id: i64,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub failure_count: u64,
    pub duration_seconds: i64,
}

impl ProbeResult {
    #[cfg(test)]
    pub fn success_for_test(target_id: i64, latency_ms: f64) -> Self {
        let now = Utc::now();
        Self {
            target_id,
            started_at: now,
            finished_at: now,
            probe_type: ProbeType::Icmp,
            resolved_ip: Some("127.0.0.1".into()),
            success: true,
            latency_ms: Some(latency_ms),
            ttl: Some(64),
            error_kind: None,
            error_message: None,
        }
    }

    #[cfg(test)]
    pub fn failure_for_test(target_id: i64, error_kind: &str) -> Self {
        let now = Utc::now();
        Self {
            target_id,
            started_at: now,
            finished_at: now,
            probe_type: ProbeType::Icmp,
            resolved_ip: Some("127.0.0.1".into()),
            success: false,
            latency_ms: None,
            ttl: None,
            error_kind: Some(error_kind.into()),
            error_message: Some(error_kind.into()),
        }
    }
}
