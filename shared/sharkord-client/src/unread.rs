//! Unread counts, accumulated the way Sharkord's own client does: the join's `readStates` are the
//! baseline, deltas add to it, updates set it.

use std::collections::{HashMap, HashSet};

use crate::Joined;

/// Sets one channel's unread count outright (a read reported by another device).
pub fn set_unread(read_states: &mut HashMap<i64, u32>, channel_id: i64, count: u32) {
    if count == 0 {
        read_states.remove(&channel_id);
    } else {
        read_states.insert(channel_id, count);
    }
}

/// The floor a join's unread is counted above: the one the user shares across devices (through the
/// companion plugin), else this device's own, else this join itself, so a newly added server's
/// backlog is not unread. Also says whether it differs from `local`, so the caller keeps it.
pub fn choose_floor(
    joined: &Joined,
    local: Option<&HashMap<i64, u32>>,
) -> (HashMap<i64, u32>, bool) {
    let floor = joined
        .shared_floor
        .clone()
        .or_else(|| local.cloned())
        .unwrap_or_else(|| joined.read_states.clone());
    let changed = local != Some(&floor);

    (floor, changed)
}

/// Unread above the baseline, per channel, skipping muted channels. A channel below its baseline
/// (read elsewhere) contributes nothing rather than hiding news in another channel.
pub fn unread_total(
    read_states: &HashMap<i64, u32>,
    baseline: &HashMap<i64, u32>,
    muted: &HashSet<i64>,
) -> u32 {
    read_states
        .iter()
        .filter(|(channel, _)| !muted.contains(*channel))
        .map(|(channel, count)| count.saturating_sub(baseline.get(channel).copied().unwrap_or(0)))
        .fold(0u32, u32::saturating_add)
}

/// Applies a delta to a channel's count, clamped to `0..=u32::MAX`.
pub fn apply_delta(read_states: &mut HashMap<i64, u32>, channel_id: i64, delta: i64) {
    let entry = read_states.entry(channel_id).or_insert(0);

    *entry = i64::from(*entry)
        .saturating_add(delta)
        .clamp(0, i64::from(u32::MAX)) as u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn muted<const N: usize>(channels: [i64; N]) -> HashSet<i64> {
        channels.into_iter().collect()
    }

    #[test]
    fn the_shared_floor_wins_then_the_local_one_then_the_join() {
        let counts = |pairs: &[(i64, u32)]| pairs.iter().copied().collect::<HashMap<_, _>>();
        let mut joined = Joined {
            read_states: counts(&[(1, 9)]),
            ..Joined::default()
        };
        let local = counts(&[(1, 4)]);

        assert_eq!(choose_floor(&joined, None), (counts(&[(1, 9)]), true));
        assert_eq!(choose_floor(&joined, Some(&local)), (local.clone(), false));

        joined.shared_floor = Some(counts(&[(1, 6)]));
        assert_eq!(
            choose_floor(&joined, Some(&local)),
            (counts(&[(1, 6)]), true)
        );
    }

    #[test]
    fn unread_counts_only_what_arrived_above_the_baseline() {
        let states = HashMap::from([(1, 42), (2, 900), (3, 4)]);
        let baseline = HashMap::from([(1, 40), (2, 900)]);

        assert_eq!(unread_total(&states, &baseline, &muted([])), 6);
        assert_eq!(unread_total(&states, &baseline, &muted([3])), 2);
        assert_eq!(
            unread_total(
                &HashMap::from([(1, 0)]),
                &HashMap::from([(1, 10)]),
                &muted([])
            ),
            0
        );
    }

    #[test]
    fn deltas_accumulate_without_going_negative_and_sets_replace() {
        let mut states = HashMap::from([(1, 1)]);

        apply_delta(&mut states, 1, 1);
        apply_delta(&mut states, 2, 1);
        assert_eq!((states[&1], states[&2]), (2, 1));

        apply_delta(&mut states, 1, -5);
        assert_eq!(states[&1], 0);

        set_unread(&mut states, 2, 7);
        assert_eq!(states[&2], 7);

        set_unread(&mut states, 2, 0);
        assert!(!states.contains_key(&2));
    }
}
