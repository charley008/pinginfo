use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{ProbeResult, TargetState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetStatus {
    pub target_id: i64,
    pub state: TargetState,
    pub success_count: u64,
    pub failure_count: u64,
    pub consecutive_failures: u64,
    pub last_latency_ms: Option<f64>,
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub avg_latency_ms: Option<f64>,
    pub loss_rate: f64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_failure_at: Option<DateTime<Utc>>,
    #[serde(skip)]
    total_latency_ms: f64,
}

impl TargetStatus {
    pub fn new(target_id: i64) -> Self {
        Self {
            target_id,
            state: TargetState::Healthy,
            success_count: 0,
            failure_count: 0,
            consecutive_failures: 0,
            last_latency_ms: None,
            min_latency_ms: None,
            max_latency_ms: None,
            avg_latency_ms: None,
            loss_rate: 0.0,
            last_success_at: None,
            last_failure_at: None,
            total_latency_ms: 0.0,
        }
    }

    pub fn disabled(target_id: i64) -> Self {
        Self {
            state: TargetState::Disabled,
            ..Self::new(target_id)
        }
    }

    pub fn apply_result(&mut self, result: &ProbeResult) {
        if result.success {
            self.success_count += 1;
            self.consecutive_failures = 0;
            self.last_success_at = Some(result.finished_at);
            if let Some(latency) = result.latency_ms {
                self.last_latency_ms = Some(latency);
                self.min_latency_ms = Some(self.min_latency_ms.map_or(latency, |v| v.min(latency)));
                self.max_latency_ms = Some(self.max_latency_ms.map_or(latency, |v| v.max(latency)));
                self.total_latency_ms += latency;
                self.avg_latency_ms = Some(self.total_latency_ms / self.success_count as f64);
            }
        } else {
            self.failure_count += 1;
            self.consecutive_failures += 1;
            self.last_failure_at = Some(result.finished_at);
            self.last_latency_ms = None;
        }

        let total = self.success_count + self.failure_count;
        self.loss_rate = if total == 0 {
            0.0
        } else {
            (self.failure_count as f64 / total as f64) * 100.0
        };

        self.state = if self.consecutive_failures >= 3 {
            TargetState::Down
        } else if self.failure_count > 0 && self.consecutive_failures > 0 {
            TargetState::Warning
        } else {
            TargetState::Healthy
        };
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{NewTarget, ProbeResult, ProbeType, TargetState};
    use crate::status::TargetStatus;

    #[test]
    fn tcp_targets_require_a_port() {
        let target = NewTarget {
            name: "web".into(),
            host: "example.com".into(),
            probe_type: ProbeType::Tcp,
            port: None,
            interval_ms: 1000,
            timeout_ms: 1000,
            enabled: true,
            group_name: None,
            description: None,
        };
        assert!(target.validate().is_err());
    }

    #[test]
    fn three_consecutive_failures_mark_target_down() {
        let mut status = TargetStatus::new(1);
        status.apply_result(&ProbeResult::success_for_test(1, 10.0));
        status.apply_result(&ProbeResult::failure_for_test(1, "timeout"));
        status.apply_result(&ProbeResult::failure_for_test(1, "timeout"));
        status.apply_result(&ProbeResult::failure_for_test(1, "timeout"));
        assert_eq!(status.state, TargetState::Down);
        assert_eq!(status.consecutive_failures, 3);
    }
}
