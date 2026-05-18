use chrono::Utc;
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

use crate::models::ProbeResult;
use crate::models::Target;
use crate::probe::{failure_result, resolve_host_port, timeout_duration};

pub async fn probe_tcp(target: &Target) -> ProbeResult {
    let started_at = Utc::now();
    let Some(port) = target.port else {
        return failure_result(
            target,
            started_at,
            "validation",
            "tcp target is missing port",
            None,
        );
    };
    let addr = match resolve_host_port(&target.host, port).await {
        Ok(addr) => addr,
        Err(err) => return failure_result(target, started_at, "resolve", err.to_string(), None),
    };
    let begin = Instant::now();
    match timeout(timeout_duration(target), TcpStream::connect(addr)).await {
        Ok(Ok(_stream)) => ProbeResult {
            target_id: target.id,
            started_at,
            finished_at: Utc::now(),
            probe_type: target.probe_type,
            resolved_ip: Some(addr.ip().to_string()),
            success: true,
            latency_ms: Some(begin.elapsed().as_secs_f64() * 1000.0),
            ttl: None,
            error_kind: None,
            error_message: None,
        },
        Ok(Err(err)) => failure_result(
            target,
            started_at,
            "connect",
            err.to_string(),
            Some(addr.ip().to_string()),
        ),
        Err(_) => failure_result(
            target,
            started_at,
            "timeout",
            "tcp probe timed out",
            Some(addr.ip().to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use crate::models::{ProbeType, Target};
    use crate::probe::tcp::probe_tcp;

    #[tokio::test]
    async fn tcp_probe_succeeds_against_local_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        let result = probe_tcp(&Target {
            id: 1,
            name: "local".into(),
            host: "127.0.0.1".into(),
            probe_type: ProbeType::Tcp,
            port: Some(port),
            interval_ms: 1000,
            timeout_ms: 1000,
            enabled: true,
            group_name: None,
            description: None,
        })
        .await;
        assert!(result.success);
        assert!(result.latency_ms.unwrap() >= 0.0);
    }
}
