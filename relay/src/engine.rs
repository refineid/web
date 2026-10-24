// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Pure leg-matching engine: no IO, no time source beyond [`Instant`].
//!
//! A leg joins with (purpose, role, token). The first leg for a token
//! waits; a complementary second leg matches it. Anything else on the
//! token (same role, third leg, purpose mismatch) is rejected. The
//! engine never signals why: the server closes rejected legs silently.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::protocol::{Join, Purpose, Role};

/// Opaque leg handle issued by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LegId(pub u64);

/// Slot key: purpose and token. Roles match inside the slot.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SlotKey {
    purpose: Purpose,
    token: Vec<u8>,
}

/// One waiting leg: which leg, which role, since when.
#[derive(Debug, Clone, Copy)]
struct Waiting {
    leg: LegId,
    role: Role,
    since: Instant,
}

/// Outcome of registering a join.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchOutcome {
    /// First leg on the token: wait for the complement.
    Wait,
    /// Complement arrived: these two legs are now joined.
    Matched {
        /// The previously waiting leg.
        peer: LegId,
    },
    /// Token busy (same role waiting, pair complete, or purpose clash):
    /// close the newcomer with no signal.
    Reject,
}

/// The matcher. Single-threaded by construction; the server holds it
/// behind a mutex for short critical sections only.
#[derive(Debug, Default)]
pub struct Matcher {
    next_leg: u64,
    waiting: HashMap<SlotKey, Waiting>,
    joined: HashMap<LegId, LegId>,
    leg_slot: HashMap<LegId, SlotKey>,
}

impl Matcher {
    /// Issue the next leg handle.
    pub fn issue_leg(&mut self) -> LegId {
        let leg = LegId(self.next_leg);
        self.next_leg += 1;
        leg
    }

    /// Register a validated join frame. Each leg joins at most once;
    /// the server enforces this, the engine does not trust it.
    pub fn join(&mut self, leg: LegId, join: &Join, now: Instant) -> MatchOutcome {
        if self.leg_slot.contains_key(&leg) {
            return MatchOutcome::Reject;
        }
        let key = SlotKey {
            purpose: join.purpose,
            token: join.token.clone(),
        };
        match self.waiting.remove(&key) {
            None => {
                // No waiter: hold this leg, unless the token already has
                // a live pair (its slot was taken when it matched).
                if self.slot_busy(&key) {
                    return MatchOutcome::Reject;
                }
                self.waiting.insert(
                    key.clone(),
                    Waiting {
                        leg,
                        role: join.role,
                        since: now,
                    },
                );
                self.leg_slot.insert(leg, key);
                MatchOutcome::Wait
            }
            Some(first) => {
                if !first.role.complements(join.role) {
                    // Same role twice: put the first leg back, reject the
                    // newcomer. No signal either way.
                    self.waiting.insert(key, first);
                    MatchOutcome::Reject
                } else {
                    self.leg_slot.insert(leg, key);
                    self.joined.insert(leg, first.leg);
                    self.joined.insert(first.leg, leg);
                    MatchOutcome::Matched { peer: first.leg }
                }
            }
        }
    }

    /// Forget a leg (close, error, timeout). Returns the joined peer, if
    /// any, so the server can close it too. Deletes all match state.
    pub fn remove(&mut self, leg: LegId) -> Option<LegId> {
        let key = self.leg_slot.remove(&leg)?;
        if let Some(waiting) = self.waiting.get(&key)
            && waiting.leg == leg
        {
            self.waiting.remove(&key);
            return None;
        }
        if let Some(peer) = self.joined.remove(&leg) {
            self.joined.remove(&peer);
            self.leg_slot.remove(&peer);
            return Some(peer);
        }
        None
    }

    /// Legs still unmatched past the deadline. The server closes them
    /// with no signal; removal happens through [`remove`].
    pub fn expired(&self, now: Instant, timeout: Duration) -> Vec<LegId> {
        self.waiting
            .values()
            .filter(|waiting| now.duration_since(waiting.since) >= timeout)
            .map(|waiting| waiting.leg)
            .collect()
    }

    /// Live legs tracked right now (waiting + joined).
    #[must_use]
    pub fn leg_count(&self) -> usize {
        self.leg_slot.len()
    }

    /// The joined peer of a leg, if matched.
    #[must_use]
    pub fn peer_of(&self, leg: LegId) -> Option<LegId> {
        self.joined.get(&leg).copied()
    }

    /// True when any live leg is mapped to this slot key.
    fn slot_busy(&self, key: &SlotKey) -> bool {
        self.leg_slot.values().any(|mapped| mapped == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SESSION_TOKEN_SIZE;

    fn join(role: Role, token: u8) -> Join {
        Join {
            purpose: Purpose::Session,
            role,
            token: vec![token; SESSION_TOKEN_SIZE],
        }
    }

    #[test]
    fn complement_matches() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let a = matcher.issue_leg();
        let b = matcher.issue_leg();
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 1), now),
            MatchOutcome::Wait
        );
        assert_eq!(
            matcher.join(b, &join(Role::Proxy, 1), now),
            MatchOutcome::Matched { peer: a }
        );
        assert_eq!(matcher.leg_count(), 2);
    }

    #[test]
    fn same_role_rejected_and_first_kept() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let a = matcher.issue_leg();
        let b = matcher.issue_leg();
        let c = matcher.issue_leg();
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 2), now),
            MatchOutcome::Wait
        );
        assert_eq!(
            matcher.join(b, &join(Role::Requester, 2), now),
            MatchOutcome::Reject
        );
        // The first leg still waits: a complement matches it.
        assert_eq!(
            matcher.join(c, &join(Role::Proxy, 2), now),
            MatchOutcome::Matched { peer: a }
        );
    }

    #[test]
    fn third_leg_rejected() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let (a, b, c) = (
            matcher.issue_leg(),
            matcher.issue_leg(),
            matcher.issue_leg(),
        );
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 3), now),
            MatchOutcome::Wait
        );
        assert_eq!(
            matcher.join(b, &join(Role::Proxy, 3), now),
            MatchOutcome::Matched { peer: a }
        );
        assert_eq!(
            matcher.join(c, &join(Role::Requester, 3), now),
            MatchOutcome::Reject
        );
        assert_eq!(
            matcher.join(c, &join(Role::Proxy, 3), now),
            MatchOutcome::Reject
        );
    }

    #[test]
    fn purpose_isolates_tokens() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let a = matcher.issue_leg();
        let b = matcher.issue_leg();
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 4), now),
            MatchOutcome::Wait
        );
        let pairing = Join {
            purpose: Purpose::Pairing,
            ..join(Role::Proxy, 4)
        };
        // Same token bytes, other purpose: a different slot, so it waits.
        assert_eq!(matcher.join(b, &pairing, now), MatchOutcome::Wait);
    }

    #[test]
    fn remove_closes_pair() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let a = matcher.issue_leg();
        let b = matcher.issue_leg();
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 5), now),
            MatchOutcome::Wait
        );
        assert_eq!(
            matcher.join(b, &join(Role::Proxy, 5), now),
            MatchOutcome::Matched { peer: a }
        );
        assert_eq!(matcher.remove(a), Some(b));
        assert_eq!(matcher.leg_count(), 0);
        assert_eq!(matcher.remove(b), None);
    }

    #[test]
    fn remove_waiter_frees_token() {
        let mut matcher = Matcher::default();
        let now = Instant::now();
        let a = matcher.issue_leg();
        let b = matcher.issue_leg();
        assert_eq!(
            matcher.join(a, &join(Role::Requester, 6), now),
            MatchOutcome::Wait
        );
        assert_eq!(matcher.remove(a), None);
        assert_eq!(
            matcher.join(b, &join(Role::Proxy, 6), now),
            MatchOutcome::Wait
        );
    }

    #[test]
    fn expiry_lists_only_stale_waiters() {
        let mut matcher = Matcher::default();
        let start = Instant::now();
        let stale = matcher.issue_leg();
        assert_eq!(
            matcher.join(stale, &join(Role::Requester, 7), start),
            MatchOutcome::Wait
        );
        let fresh = matcher.issue_leg();
        let later = start + Duration::from_millis(10);
        assert_eq!(
            matcher.join(fresh, &join(Role::Requester, 8), later),
            MatchOutcome::Wait
        );
        let expired = matcher.expired(later, Duration::from_millis(10));
        assert_eq!(expired, vec![stale]);
    }
}
