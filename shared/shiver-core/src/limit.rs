//! Rationing what a page may ask the core to do on its behalf (opening links in the browser).

use std::{
    collections::HashMap,
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

    pub fn forget(&self, key: &str) {
        self.0.locked().remove(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openings_are_rationed_per_second_and_per_window_and_per_key() {
        let openings = Openings::default();

        assert_eq!(openings.take("a", 8), PER_SECOND);
        assert_eq!(openings.take("a", 1), 0);
        assert_eq!(openings.take("b", 1), 1);

        openings.forget("a");

        assert_eq!(openings.take("a", 1), 1);
    }
}
