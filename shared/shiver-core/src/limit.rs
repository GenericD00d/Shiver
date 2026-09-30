//! Rationing: what a page may ask the core to do on its behalf (opening links in the browser), and
//! how often Shiver joins a server.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::LockExt;

const PER_SECOND: usize = 5;
const PER_WINDOW: usize = 10;
const WINDOW: Duration = Duration::from_secs(10);

/// Recent link openings per key (a page), so a hostile page cannot flood the browser with tabs:
/// at most five a second and ten per ten seconds.
#[derive(Default)]
pub struct Openings(Mutex<HashMap<String, Vec<Instant>>>);

impl Openings {
    /// Grants up to `wanted` openings for `key` now, recording them; returns how many.
    pub fn take(&self, key: &str, wanted: usize) -> usize {
        let mut seen = self.0.locked();
        let recent = seen.entry(key.to_string()).or_default();

        recent.retain(|at| at.elapsed() < WINDOW);

        let this_second = recent
            .iter()
            .filter(|at| at.elapsed() < Duration::from_secs(1))
            .count();
        let granted = wanted
            .min(PER_SECOND.saturating_sub(this_second))
            .min(PER_WINDOW.saturating_sub(recent.len()));

        recent.extend(std::iter::repeat(Instant::now()).take(granted));

        granted
    }

    /// The leading part of `wanted` that `key` may open now.
    pub fn grant<'a, T>(&self, key: &str, wanted: &'a [T]) -> &'a [T] {
        let granted = self.take(key, wanted.len());

        if granted < wanted.len() {
            eprintln!(
                "[shiver] {key} asked to open {} links, opening {granted}",
                wanted.len()
            );
        }

        &wanted[..granted]
    }

    pub fn forget(&self, key: &str) {
        self.0.locked().remove(key);
    }
}

/// Joins Shiver makes per rail entry within `JOIN_WINDOW`. Sharkord allows a user five joins a
/// minute (`joinServer`, one fixed window shared by all their devices), so one is left for the
/// user's others.
const JOINS: usize = 4;
/// A little over Sharkord's minute, for the time a request takes to arrive.
const JOIN_WINDOW: Duration = Duration::from_secs(62);

/// When Shiver joined, or will join, each rail entry's server.
///
/// Sharkord refuses a join past its limit, and its client then drops the session as if it had been
/// refused, leaving the sign-in form. Every join Shiver makes (a page loading, the core's own
/// socket connecting) takes a place here first and waits for it, so none is refused for coming too
/// often; places are handed out in order, so callers waiting at once queue.
#[derive(Default)]
pub struct Joins(Mutex<HashMap<String, VecDeque<Instant>>>);

/// A place taken for one join: `wait` is how long until it may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JoinPlace {
    pub wait: Duration,
    at: Instant,
}

impl Joins {
    /// Takes the next free place for a join of `key`'s server.
    pub fn take(&self, key: &str) -> JoinPlace {
        self.take_at(key, Instant::now())
    }

    fn take_at(&self, key: &str, now: Instant) -> JoinPlace {
        let mut all = self.0.locked();
        let joins = all.entry(key.to_string()).or_default();

        joins.retain(|at| *at + JOIN_WINDOW > now);

        let at = match joins.len().checked_sub(JOINS) {
            Some(index) => (joins[index] + JOIN_WINDOW).max(now),
            None => now,
        };

        joins.push_back(at);
        joins.make_contiguous().sort();

        JoinPlace {
            wait: at.saturating_duration_since(now),
            at,
        }
    }

    /// Gives back a place that was not used (the join was abandoned while waiting for it).
    pub fn give_back(&self, key: &str, place: JoinPlace) {
        if let Some(joins) = self.0.locked().get_mut(key) {
            if let Some(index) = joins.iter().position(|at| *at == place.at) {
                joins.remove(index);
            }
        }
    }

    pub fn forget(&self, key: &str) {
        self.0.locked().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_are_paced_below_sharkords_limit_and_queue_in_order() {
        let joins = Joins::default();
        let start = Instant::now();
        let seconds = |n: u64| start + Duration::from_secs(n);

        for n in 0..JOINS as u64 {
            assert_eq!(joins.take_at("a", seconds(n)).wait, Duration::ZERO);
        }

        // the fifth waits until the first leaves the window, the sixth for the second
        let fifth = joins.take_at("a", seconds(10));
        let sixth = joins.take_at("a", seconds(10));

        assert_eq!(fifth.wait, JOIN_WINDOW - Duration::from_secs(10));
        assert_eq!(sixth.wait, JOIN_WINDOW - Duration::from_secs(9));
        assert_eq!(
            joins.take_at("b", seconds(10)).wait,
            Duration::ZERO,
            "per entry"
        );

        // an abandoned place is free again
        joins.give_back("a", sixth);
        assert_eq!(
            joins.take_at("a", seconds(10)).wait,
            JOIN_WINDOW - Duration::from_secs(9)
        );

        // once the window has passed, joins are free again
        assert_eq!(
            joins.take_at("a", seconds(10) + JOIN_WINDOW * 2).wait,
            Duration::ZERO
        );
    }

    #[test]
    fn openings_are_rationed_per_second_and_per_window_and_per_key() {
        let openings = Openings::default();

        assert_eq!(openings.take("a", 8), PER_SECOND);
        assert_eq!(openings.take("a", 1), 0);
        assert_eq!(openings.take("b", 1), 1);

        openings.forget("a");

        assert_eq!(openings.take("a", 1), 1);
        assert_eq!(openings.grant("a", &[1, 2, 3, 4, 5, 6]), &[1, 2, 3, 4]);
    }
}
