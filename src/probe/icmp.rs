use chrono::Utc;
use std::net::SocketAddr;

use crate::models::ProbeResult;
use crate::models::Target;
use crate::probe::{failure_result, resolve_host_port, timeout_duration};

pub async fn probe_icmp(target: &Target) -> ProbeResult {
    let started_at = Utc::now();
    let addr = match resolve_host_port(&target.host, 0).await {
        Ok(addr) => addr,
        Err(err) => return failure_result(target, started_at, "resolve", err.to_string(), None),
    };

    let mut config = surge_ping::Config::builder();
    if addr.is_ipv6() {
        config = config.kind(surge_ping::ICMP::V6);
    }

    let client = match surge_ping::Client::new(&config.build()) {
        Ok(client) => client,
        Err(err) => {
            return failure_result(
                target,
                started_at,
                "socket",
                err.to_string(),
                Some(addr.ip().to_string()),
            );
        }
    };

    let ident = surge_ping::PingIdentifier((target.id as u16).wrapping_add(1));
    let mut pinger = client.pinger(addr.ip(), ident).await;
    if let SocketAddr::V6(v6) = addr {
        pinger.scope_id(v6.scope_id());
    }
    pinger.timeout(timeout_duration(target));

    match pinger.ping(surge_ping::PingSequence(0), &[0; 32]).await {
        Ok((packet, duration)) => {
            let ttl = match packet {
                surge_ping::IcmpPacket::V4(packet) => packet.get_ttl(),
                surge_ping::IcmpPacket::V6(_) => None,
            };
            ProbeResult {
                target_id: target.id,
                started_at,
                finished_at: Utc::now(),
                probe_type: target.probe_type,
                resolved_ip: Some(addr.ip().to_string()),
                success: true,
                latency_ms: Some(duration.as_secs_f64() * 1000.0),
                ttl,
                error_kind: None,
                error_message: None,
            }
        }
        Err(err) => failure_result(
            target,
            started_at,
            "icmp",
            err.to_string(),
            Some(addr.ip().to_string()),
        ),
    }
}
