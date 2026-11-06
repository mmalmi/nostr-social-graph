//! In-memory operational counters only; no event, author, URL or error-body data.
use std::time::{Duration, Instant};

const REPORT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Outcome {
    Accepted,
    SendTimeout,
    SendError,
    TransportFailure,
    ReportedRejection,
    OtherDeliveryFailure,
    CooldownSkip,
    DiskReserve,
}

impl Outcome {
    pub(super) fn accepted(self) -> bool {
        self == Self::Accepted
    }

    pub(super) fn starts_cooldown(self) -> bool {
        matches!(
            self,
            Self::SendTimeout | Self::SendError | Self::TransportFailure
        )
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::SendTimeout => "send timeout",
            Self::SendError => "send error",
            Self::TransportFailure => "transport",
            Self::ReportedRejection => "reported relay rejection",
            Self::OtherDeliveryFailure => "unclassified delivery failure",
            Self::CooldownSkip => "cooldown skip",
            Self::DiskReserve => "disk reserve",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    pub send_attempts: u64,
    pub accepted: u64,
    pub send_timeouts: u64,
    pub send_errors: u64,
    pub transport_failures: u64,
    pub reported_rejections: u64,
    pub other_delivery_failures: u64,
    pub cooldown_skips: u64,
    pub disk_reserve_skips: u64,
}

#[derive(Default)]
pub(super) struct Diagnostics {
    counts: Counts,
    failures: u64,
    reported_failures: u64,
    last_report: Option<Instant>,
}

impl Diagnostics {
    pub(super) fn record(&mut self, outcome: Outcome) {
        if !matches!(outcome, Outcome::CooldownSkip | Outcome::DiskReserve) {
            self.counts.send_attempts = self.counts.send_attempts.saturating_add(1);
        }
        if outcome != Outcome::Accepted {
            self.failures = self.failures.saturating_add(1);
        }
        let counter = match outcome {
            Outcome::Accepted => &mut self.counts.accepted,
            Outcome::SendTimeout => &mut self.counts.send_timeouts,
            Outcome::SendError => &mut self.counts.send_errors,
            Outcome::TransportFailure => &mut self.counts.transport_failures,
            Outcome::ReportedRejection => &mut self.counts.reported_rejections,
            Outcome::OtherDeliveryFailure => &mut self.counts.other_delivery_failures,
            Outcome::CooldownSkip => &mut self.counts.cooldown_skips,
            Outcome::DiskReserve => &mut self.counts.disk_reserve_skips,
        };
        *counter = counter.saturating_add(1);
    }

    pub(super) fn take_report(&mut self, now: Instant) -> Option<Counts> {
        if self.failures == self.reported_failures
            || self
                .last_report
                .is_some_and(|at| now.duration_since(at) < REPORT_INTERVAL)
        {
            return None;
        }
        self.reported_failures = self.failures;
        self.last_report = Some(now);
        Some(self.counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hundreds_of_cooldown_skips_are_not_send_timeouts_or_attempts() {
        let mut diagnostics = Diagnostics::default();
        diagnostics.record(Outcome::SendTimeout);
        for _ in 0..652 {
            diagnostics.record(Outcome::CooldownSkip);
        }
        let report = diagnostics.take_report(Instant::now()).unwrap();
        assert_eq!(report.send_attempts, 1);
        assert_eq!(report.send_timeouts, 1);
        assert_eq!(report.cooldown_skips, 652);
        assert_eq!(report.send_errors + report.reported_rejections, 0);
    }

    #[test]
    fn error_classes_are_distinct_and_keep_existing_cooldown_policy() {
        let mut diagnostics = Diagnostics::default();
        for outcome in [
            Outcome::Accepted,
            Outcome::SendError,
            Outcome::TransportFailure,
            Outcome::ReportedRejection,
            Outcome::OtherDeliveryFailure,
            Outcome::DiskReserve,
        ] {
            diagnostics.record(outcome);
        }
        let report = diagnostics.take_report(Instant::now()).unwrap();
        assert_eq!(report.send_attempts, 5);
        assert_eq!(report.accepted, 1);
        assert_eq!(report.send_errors, 1);
        assert_eq!(report.transport_failures, 1);
        assert_eq!(report.reported_rejections, 1);
        assert_eq!(report.other_delivery_failures, 1);
        assert_eq!(report.disk_reserve_skips, 1);
        for outcome in [
            Outcome::SendTimeout,
            Outcome::SendError,
            Outcome::TransportFailure,
        ] {
            assert!(outcome.starts_cooldown());
        }
        assert!(!Outcome::ReportedRejection.starts_cooldown());
        assert!(!Outcome::OtherDeliveryFailure.starts_cooldown());
        assert!(Outcome::Accepted.accepted());
    }

    #[test]
    fn reports_are_rate_limited_and_quiet_without_new_failures() {
        let mut diagnostics = Diagnostics::default();
        let now = Instant::now();
        diagnostics.record(Outcome::Accepted);
        assert!(diagnostics.take_report(now).is_none());
        diagnostics.record(Outcome::SendError);
        assert!(diagnostics.take_report(now).is_some());
        diagnostics.record(Outcome::CooldownSkip);
        assert!(
            diagnostics
                .take_report(now + Duration::from_secs(29))
                .is_none()
        );
        assert!(
            diagnostics
                .take_report(now + Duration::from_secs(30))
                .is_some()
        );
        diagnostics.record(Outcome::Accepted);
        assert!(
            diagnostics
                .take_report(now + Duration::from_secs(60))
                .is_none()
        );
    }
}
