// SPDX-License-Identifier: GPL-3.0-only

//! A pomodoro timer: focus, then a break, a long one every fourth; each
//! phase starts the next when it ends.

use std::time::{Duration, Instant};

pub const FOCUS: Duration = Duration::from_secs(25 * 60);
pub const BREAK: Duration = Duration::from_secs(5 * 60);
pub const LONG_BREAK: Duration = Duration::from_secs(15 * 60);
/// Focus phases before a long break.
const ROUNDS: u8 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Focus,
    Break,
    LongBreak,
}

impl Phase {
    pub fn length(self) -> Duration {
        match self {
            Phase::Focus => FOCUS,
            Phase::Break => BREAK,
            Phase::LongBreak => LONG_BREAK,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pomodoro {
    pub phase: Phase,
    /// Focus phases done since the last long break.
    done: u8,
    /// While running: when the phase ends. While paused: what's left.
    ends: Option<Instant>,
    left: Duration,
    /// Started since the last reset, running or paused.
    started: bool,
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self { phase: Phase::Focus, done: 0, ends: None, left: FOCUS, started: false }
    }
}

impl Pomodoro {
    pub fn running(&self) -> bool {
        self.ends.is_some()
    }

    /// Started at all: running, or paused partway.
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn remaining(&self, now: Instant) -> Duration {
        match self.ends {
            Some(ends) => ends.saturating_duration_since(now),
            None => self.left,
        }
    }

    pub fn start(&mut self, now: Instant) {
        if self.ends.is_none() {
            self.ends = Some(now + self.left);
        }
        self.started = true;
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(ends) = self.ends.take() {
            self.left = ends.saturating_duration_since(now);
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// On to the next phase, running on if this one ran.
    pub fn skip(&mut self, now: Instant) {
        let running = self.running();
        self.started = true;
        self.phase = match self.phase {
            Phase::Focus => {
                self.done += 1;
                if self.done >= ROUNDS {
                    self.done = 0;
                    Phase::LongBreak
                } else {
                    Phase::Break
                }
            }
            Phase::Break | Phase::LongBreak => Phase::Focus,
        };
        self.left = self.phase.length();
        self.ends = running.then(|| now + self.left);
    }

    /// The phase that just began, if the running one ended.
    pub fn tick(&mut self, now: Instant) -> Option<Phase> {
        let ends = self.ends?;
        if now < ends {
            return None;
        }
        // Counted from when it ended, not when we noticed.
        self.skip(ends);
        Some(self.phase)
    }
}

/// "12:34"
pub fn clock(left: Duration) -> String {
    let secs = left.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_follow_each_other() {
        let t0 = Instant::now();
        let mut p = Pomodoro::default();
        assert!(!p.started());
        p.start(t0);
        assert_eq!(p.tick(t0 + Duration::from_secs(60)), None);
        let mut now = t0;
        let mut seen = Vec::new();
        for _ in 0..8 {
            now = now + p.remaining(now);
            seen.push(p.tick(now).expect("a phase ended"));
        }
        use Phase::*;
        assert_eq!(seen, [Break, Focus, Break, Focus, Break, Focus, LongBreak, Focus]);
        assert!(p.running());
    }

    #[test]
    fn pausing_keeps_what_is_left() {
        let t0 = Instant::now();
        let mut p = Pomodoro::default();
        p.start(t0);
        p.pause(t0 + Duration::from_secs(100));
        assert_eq!(p.remaining(t0 + Duration::from_secs(5000)), FOCUS - Duration::from_secs(100));
        assert_eq!(p.tick(t0 + Duration::from_secs(5000)), None);
        assert_eq!(clock(Duration::from_secs(25 * 60 - 100)), "23:20");
        p.reset();
        assert!(!p.started());
        // Skipped into the long break while paused: still on.
        for _ in 0..7 {
            p.skip(t0);
        }
        assert_eq!(p.phase, Phase::LongBreak);
        assert!(p.started() && !p.running());
    }
}
