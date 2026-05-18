pub mod icmp;
pub mod tcp;

use async_trait::async_trait;
use chrono::Utc;
use std::net::ToSocketAddrs;
use std::time::Duration;

use crate::models::{ProbeResult, ProbeType, Target};

#[async_trait]
pub trait Probe: Send + Sync {
    async fn probe(&self, target: &Target) -> ProbeResult;
}

#[derive(Debug, Clone, Default)]
pub struct DefaultProbe;

#[async_trait]
impl Probe for DefaultProbe {
    async fn probe(&self, target: &Target) -> ProbeResult {
        match target.probe_type {
            ProbeType::Icmp => icmp::probe_icmp(target).await,
            ProbeType::Tcp => tcp::probe_tcp(target).await,
        }
    }
}

pub fn failure_result(
    target: &Target,
    started_at: chrono::DateTime<Utc>,
    kind: &str,
    message: impl Into<String>,
    resolved_ip: Option<String>,
) -> ProbeResult {
    ProbeResult {
        target_id: target.id,
        started_at,
        finished_at: Utc::now(),
        probe_type: target.probe_type,
        resolved_ip,
        success: false,
        latency_ms: None,
        ttl: None,
        error_kind: Some(kind.into()),
        error_message: Some(message.into()),
    }
}

pub async fn resolve_host_port(host: &str, port: u16) -> anyhow::Result<std::net::SocketAddr> {
    let host = host.to_string();
    tokio::task::spawn_blocking(move || {
        (host.as_str(), port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("no address resolved"))
    })
    .await?
}

pub fn timeout_duration(target: &Target) -> Duration {
    Duration::from_millis(target.timeout_ms.max(1))
}
