//! Starting again a copy that stopped on its own.
//!
//! In memory, on purpose: a node that restarts starts every copy at boot
//! anyway, and an episode of stopping is about this process's last few
//! minutes, not something to carry across one.

use std::collections::HashMap;
use std::time::Instant;

use crate::domain::policies::restarts::{wait_before, STABLE};

#[derive(Default)]
pub struct Restarts {
    episodes: HashMap<String, Episode>,
    up_since: HashMap<String, Instant>,
}

struct Episode {
    attempts: u32,
    next_at: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Leave,
    Restart {
        attempt: u32,
    },
    /// Seen running for as long as forgiving takes. Said on every pass
    /// after that rather than once, and of a copy with no episode too: the
    /// note a restart left on its row may have been written before this
    /// process started, and an answer that depended on remembering the
    /// restart would never clear one of those.
    Settled,
}

impl Restarts {
    /// What to do about one copy, as seen now.
    ///
    /// `running` is `None` when the runtime could not be asked, and
    /// `deployed` is whether its last deployment gave it an address — one
    /// that failed is an answer for the operator, not a stop to retry,
    /// unless it failed while this was already starting the copy again.
    pub fn decide(
        &mut self,
        replica: &str,
        running: Option<bool>,
        deployed: bool,
        now: Instant,
    ) -> Verdict {
        match running {
            None => Verdict::Leave,
            Some(true) => {
                let since = *self.up_since.entry(replica.to_string()).or_insert(now);
                if now.duration_since(since) < STABLE {
                    return Verdict::Leave;
                }
                self.episodes.remove(replica);
                Verdict::Settled
            }
            Some(false) => {
                self.up_since.remove(replica);
                if !deployed && !self.episodes.contains_key(replica) {
                    return Verdict::Leave;
                }
                let episode = self.episodes.entry(replica.to_string()).or_insert(Episode {
                    attempts: 0,
                    next_at: now,
                });
                if now < episode.next_at {
                    return Verdict::Leave;
                }
                episode.attempts += 1;
                episode.next_at = now + wait_before(episode.attempts + 1);
                Verdict::Restart {
                    attempt: episode.attempts,
                }
            }
        }
    }

    pub fn forget(&mut self, replica: &str) {
        self.episodes.remove(replica);
        self.up_since.remove(replica);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_copy_that_stopped_is_started_at_once_and_then_ever_more_patiently() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        assert_eq!(
            restarts.decide("r1", Some(false), true, start),
            Verdict::Restart { attempt: 1 }
        );
        assert_eq!(
            restarts.decide("r1", Some(false), true, start + Duration::from_secs(5)),
            Verdict::Leave,
            "the second start waits"
        );
        assert_eq!(
            restarts.decide("r1", Some(false), true, start + Duration::from_secs(10)),
            Verdict::Restart { attempt: 2 }
        );
        assert_eq!(
            restarts.decide("r1", Some(false), true, start + Duration::from_secs(30)),
            Verdict::Leave
        );
        assert_eq!(
            restarts.decide("r1", Some(false), true, start + Duration::from_secs(40)),
            Verdict::Restart { attempt: 3 }
        );
    }

    #[test]
    fn a_deployment_that_failed_is_not_retried() {
        let mut restarts = Restarts::default();
        assert_eq!(
            restarts.decide("r1", Some(false), false, Instant::now()),
            Verdict::Leave
        );
    }

    /// Once an episode has begun, a start that fails — the registry away
    /// for a minute — must not end the service's life.
    #[test]
    fn a_start_that_fails_inside_an_episode_is_tried_again() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        restarts.decide("r1", Some(false), true, start);
        assert_eq!(
            restarts.decide("r1", Some(false), false, start + Duration::from_secs(10)),
            Verdict::Restart { attempt: 2 }
        );
    }

    #[test]
    fn a_runtime_that_cannot_be_asked_is_not_a_copy_that_stopped() {
        let mut restarts = Restarts::default();
        assert_eq!(
            restarts.decide("r1", None, true, Instant::now()),
            Verdict::Leave
        );
    }

    #[test]
    fn a_copy_that_stays_up_long_enough_is_forgiven_its_history() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        restarts.decide("r1", Some(false), true, start);
        restarts.decide("r1", Some(false), true, start + Duration::from_secs(10));
        restarts.decide("r1", Some(true), true, start + Duration::from_secs(20));
        assert_eq!(
            restarts.decide(
                "r1",
                Some(true),
                true,
                start + Duration::from_secs(20) + STABLE,
            ),
            Verdict::Settled
        );
        assert_eq!(
            restarts.decide(
                "r1",
                Some(false),
                true,
                start + Duration::from_secs(21) + STABLE
            ),
            Verdict::Restart { attempt: 1 },
            "a new episode, started at once"
        );
    }

    /// **A copy is settled by what this process has seen, not by what it
    /// remembers.** The note a restart writes outlived the restart: it was
    /// forgiven only in memory, so the row kept saying `Failed` for days —
    /// airbag-tech-db on node-0 carried one from the disk incident of
    /// 2026-11-02, written before the process that would have forgiven it
    /// was replaced. Reported by Jorge with ghost beside it.
    #[test]
    fn a_copy_seen_up_long_enough_is_settled_whatever_this_process_remembers() {
        let mut restarts = Restarts::default();
        let start = Instant::now();
        assert_eq!(
            restarts.decide("r1", Some(true), true, start),
            Verdict::Leave
        );
        assert_eq!(
            restarts.decide("r1", Some(true), true, start + STABLE),
            Verdict::Settled
        );

        restarts.decide(
            "r1",
            Some(false),
            true,
            start + STABLE + Duration::from_secs(1),
        );
        assert_eq!(
            restarts.decide(
                "r1",
                Some(true),
                true,
                start + STABLE + Duration::from_secs(2)
            ),
            Verdict::Leave,
            "a stop starts the count again"
        );
        assert_eq!(
            restarts.decide(
                "r1",
                None,
                true,
                start + STABLE * 2 + Duration::from_secs(2)
            ),
            Verdict::Leave,
            "a runtime that cannot be asked says nothing about the copy"
        );
        assert_eq!(
            restarts.decide(
                "r1",
                Some(true),
                true,
                start + STABLE * 2 + Duration::from_secs(2)
            ),
            Verdict::Settled
        );
    }
}
