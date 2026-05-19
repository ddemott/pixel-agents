use tokio::time::{Duration, Instant};

/// Backoff state while the daemon connection is down.
#[derive(Debug)]
pub struct ReconnectState {
    pub attempt: u32,
    pub since: Instant,
    pub next_try: Instant,
    pub fork_attempted: bool,
}

impl ReconnectState {
    /// Post-disconnect: first probe after 250 ms.
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            attempt: 0,
            since: now,
            next_try: now + Duration::from_millis(250),
            fork_attempted: false,
        }
    }

    /// Boot-time: probe immediately (no need to wait).
    pub fn new_immediate() -> Self {
        let now = Instant::now();
        Self { attempt: 0, since: now, next_try: now, fork_attempted: false }
    }

    /// Advance after a failed attempt.
    pub fn on_failure(&mut self) {
        // attempt 0→1: wait 750 ms more (1 s total); 1+→ every 3 s
        let delay = match self.attempt {
            0 => Duration::from_millis(750),
            _ => Duration::from_secs(3),
        };
        self.attempt += 1;
        self.next_try = Instant::now() + delay;
    }

    pub fn is_due(&self) -> bool {
        Instant::now() >= self.next_try
    }

    /// Try to fork the daemon once, after the second probe fails.
    pub fn should_fork(&self) -> bool {
        self.attempt == 1 && !self.fork_attempted
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.since.elapsed().as_secs()
    }
}

/// Try to spawn a detached daemon process.
/// Returns true if a child was forked.
/// No-op (returns false) until `pixel-agents-daemon` lands as an installed binary.
pub fn try_fork_daemon() -> bool {
    use std::process::{Command, Stdio};
    Command::new("pixel-agents-daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_immediate_is_due() {
        assert!(ReconnectState::new_immediate().is_due());
    }

    #[test]
    fn new_has_250ms_delay() {
        let rs = ReconnectState::new();
        assert!(!rs.is_due());
        assert_eq!(rs.attempt, 0);
    }

    #[test]
    fn on_failure_advances_attempt() {
        let mut rs = ReconnectState::new();
        rs.on_failure();
        assert_eq!(rs.attempt, 1);
    }

    #[test]
    fn second_probe_delay_is_750ms() {
        let mut rs = ReconnectState::new();
        rs.on_failure(); // attempt 0 → 1, delay 750 ms
        let remaining = rs.next_try.saturating_duration_since(Instant::now());
        assert!(remaining >= Duration::from_millis(600), "too short: {remaining:?}");
        assert!(remaining <= Duration::from_millis(800), "too long: {remaining:?}");
    }

    #[test]
    fn subsequent_probes_3s_delay() {
        let mut rs = ReconnectState::new();
        rs.on_failure(); // 0 → 1
        rs.on_failure(); // 1 → 2, delay 3 s
        let remaining = rs.next_try.saturating_duration_since(Instant::now());
        assert!(remaining >= Duration::from_millis(2800), "too short: {remaining:?}");
        assert!(remaining <= Duration::from_millis(3100), "too long: {remaining:?}");
    }

    #[test]
    fn fork_fires_on_attempt_1() {
        let mut rs = ReconnectState::new();
        rs.on_failure(); // 0 → 1
        assert!(rs.should_fork());
    }

    #[test]
    fn fork_not_on_attempt_0() {
        assert!(!ReconnectState::new().should_fork());
    }

    #[test]
    fn fork_not_after_fork_attempted() {
        let mut rs = ReconnectState::new();
        rs.on_failure();
        rs.fork_attempted = true;
        assert!(!rs.should_fork());
    }

    #[test]
    fn fork_not_on_attempt_2_plus() {
        let mut rs = ReconnectState::new();
        rs.on_failure(); // 0 → 1
        rs.fork_attempted = true;
        rs.on_failure(); // 1 → 2
        assert!(!rs.should_fork());
    }

    #[test]
    fn elapsed_secs_near_zero() {
        let rs = ReconnectState::new();
        assert!(rs.elapsed_secs() <= 1);
    }
}
