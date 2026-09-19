//! Global voice.
//!
//! Two things Shiver's own chrome has to own, because no single server can: the
//! user's voice status watched across every server, and controls that are reachable whichever
//! server is on screen.
//!
//! Sharkord publishes `currentVoiceChannelId` to plugins but keeps `ownVoiceState` and its toggles
//! in the client's redux store and React context, so the mute flags are read off its own controls
//! and Shiver acts by clicking them (see `bridge/index.ts`). This module is the cross-server half:
//! which entry holds the call, and therefore which pages must refuse to start one.

use std::{
    collections::HashMap,
    sync::{Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};

/// What one page reports about its own voice session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSnapshot {
    pub channel_id: i64,
    #[serde(default)]
    pub channel_name: Option<String>,
    #[serde(default)]
    pub mic_muted: bool,
    #[serde(default)]
    pub sound_muted: bool,
    /// Sharkord disables its own mic control while the user is deafened
    #[serde(default)]
    pub mic_locked: bool,
}

/// The call Shiver shows in the rail, with enough about the server to say where it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceStatus {
    pub entry_id: String,
    pub server_name: String,
    pub account_label: String,
    pub channel_id: i64,
    pub channel_name: Option<String>,
    pub mic_muted: bool,
    pub sound_muted: bool,
    pub mic_locked: bool,
}

#[derive(Debug, Clone)]
struct Session {
    snapshot: VoiceSnapshot,
    server_name: String,
    account_label: String,
    /// where this call sits in join order, which is how the original holder keeps it
    order: u64,
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, Session>,
    /// the lock value last pushed to each page, so an unchanged lock is not re-evaluated every tick
    locks: HashMap<String, bool>,
    /// hands out `Session::order`
    next_order: u64,
}

#[derive(Default)]
pub struct VoiceState(Mutex<State>);

impl VoiceState {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Records what one page reported. Returns whether the rail's view of voice changed.
    pub fn report(
        &self,
        entry_id: &str,
        server_name: &str,
        account_label: &str,
        snapshot: Option<VoiceSnapshot>,
    ) -> bool {
        let mut state = self.state();

        let Some(snapshot) = snapshot else {
            return state.sessions.remove(entry_id).is_some();
        };

        match state.sessions.get_mut(entry_id) {
            Some(existing) => {
                if existing.snapshot == snapshot && existing.server_name == server_name {
                    return false;
                }

                // `order` is deliberately left alone. Walking into another voice channel on the
                // same server is the same session continuing, and restarting its place in the queue
                // would hand the call to a server that joined later and then evict the user from
                // the channel they had just chosen.
                existing.snapshot = snapshot;
                existing.server_name = server_name.to_string();
                existing.account_label = account_label.to_string();
            }
            None => {
                let order = state.next_order;

                state.next_order += 1;
                state.sessions.insert(
                    entry_id.to_string(),
                    Session {
                        snapshot,
                        server_name: server_name.to_string(),
                        account_label: account_label.to_string(),
                        order,
                    },
                );
            }
        }

        true
    }

    /// The entry holding the call: whichever joined first.
    ///
    /// Join order is a counter rather than a timestamp. Two servers can report a call within the
    /// same millisecond — at launch every server connects at once — and a clock at that resolution
    /// would tie, leaving the holder to be decided by a tiebreak instead of by who actually joined.
    fn holder_id(state: &State) -> Option<String> {
        state
            .sessions
            .iter()
            .min_by_key(|(_, session)| session.order)
            .map(|(entry_id, _)| entry_id.clone())
    }

    pub fn status(&self) -> Option<VoiceStatus> {
        let state = self.state();
        let entry_id = Self::holder_id(&state)?;
        let session = state.sessions.get(&entry_id)?;

        Some(VoiceStatus {
            entry_id,
            server_name: session.server_name.clone(),
            account_label: session.account_label.clone(),
            channel_id: session.snapshot.channel_id,
            channel_name: session.snapshot.channel_name.clone(),
            mic_muted: session.snapshot.mic_muted,
            sound_muted: session.snapshot.sound_muted,
            mic_locked: session.snapshot.mic_locked,
        })
    }

    pub fn holder(&self) -> Option<String> {
        Self::holder_id(&self.state())
    }

    /// Entries in a call that is not the one Shiver recognises.
    ///
    /// The lock in each page is what normally stops a second call from starting. This is the
    /// backstop for a join the lock did not catch — one started before Shiver had pushed the lock, or
    /// through a path that is not a channel click — and those pages are told to hang up.
    pub fn intruders(&self) -> Vec<String> {
        let state = self.state();
        let Some(holder) = Self::holder_id(&state) else {
            return Vec::new();
        };

        state
            .sessions
            .keys()
            .filter(|entry_id| **entry_id != holder)
            .cloned()
            .collect()
    }

    /// Which pages need their lock changed, and to what.
    ///
    /// Only differences are returned: the lock is pushed by evaluating a call in the page, and
    /// doing that for every server on every poll would be a needless eval a second per server.
    pub fn pending_locks(&self, entry_ids: &[String]) -> Vec<(String, bool)> {
        let mut state = self.state();
        let holder = Self::holder_id(&state);
        let mut updates = Vec::new();

        for entry_id in entry_ids {
            let locked = holder.as_ref().is_some_and(|holder| holder != entry_id);

            if state.locks.get(entry_id) == Some(&locked) {
                continue;
            }

            state.locks.insert(entry_id.clone(), locked);
            updates.push((entry_id.clone(), locked));
        }

        updates
    }

    /// Drops everything known about one rail entry, including the lock Shiver believes it has.
    pub fn forget_entry(&self, entry_id: &str) {
        let mut state = self.state();

        state.sessions.remove(entry_id);
        state.locks.remove(entry_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(channel_id: i64) -> VoiceSnapshot {
        VoiceSnapshot {
            channel_id,
            channel_name: Some(format!("channel {channel_id}")),
            mic_muted: false,
            sound_muted: false,
            mic_locked: false,
        }
    }

    fn join(voice: &VoiceState, entry_id: &str, channel_id: i64) {
        voice.report(entry_id, "server", "account", Some(snapshot(channel_id)));
    }

    #[test]
    fn the_first_server_to_join_keeps_the_call() {
        let voice = VoiceState::default();

        join(&voice, "a", 1);
        join(&voice, "b", 2);

        assert_eq!(voice.holder().as_deref(), Some("a"));
        assert_eq!(voice.intruders(), vec!["b".to_string()]);
    }

    #[test]
    fn join_order_decides_the_holder_not_the_entry_id() {
        let voice = VoiceState::default();

        // "z" joins first, so it holds the call even though "a" would win any tiebreak on id.
        // every server connects at launch within the same millisecond, so a clock cannot separate
        // these two and whatever broke the tie would silently become the rule.
        join(&voice, "z", 1);
        join(&voice, "a", 2);

        assert_eq!(voice.holder().as_deref(), Some("z"));
        assert_eq!(voice.intruders(), vec!["a".to_string()]);
    }

    #[test]
    fn moving_channel_does_not_lose_the_call_to_a_later_joiner() {
        let voice = VoiceState::default();

        join(&voice, "a", 1);
        join(&voice, "b", 2);
        // the holder walking into another voice channel on its own server is the same session
        join(&voice, "a", 3);

        assert_eq!(voice.holder().as_deref(), Some("a"));
    }

    #[test]
    fn leaving_hands_the_call_to_whoever_is_left() {
        let voice = VoiceState::default();

        join(&voice, "a", 1);
        join(&voice, "b", 2);
        voice.report("a", "server", "account", None);

        assert_eq!(voice.holder().as_deref(), Some("b"));
        assert!(voice.intruders().is_empty());
    }

    #[test]
    fn only_changes_are_reported_so_an_idle_poll_is_silent() {
        let voice = VoiceState::default();

        assert!(voice.report("a", "server", "account", Some(snapshot(1))));
        assert!(!voice.report("a", "server", "account", Some(snapshot(1))));
        assert!(voice.report(
            "a",
            "server",
            "account",
            Some(VoiceSnapshot {
                mic_muted: true,
                ..snapshot(1)
            })
        ));
    }

    #[test]
    fn every_server_but_the_holder_is_locked_and_only_once() {
        let voice = VoiceState::default();
        let rail = ["a".to_string(), "b".to_string(), "c".to_string()];

        join(&voice, "a", 1);

        let mut locks = voice.pending_locks(&rail);

        locks.sort();

        assert_eq!(
            locks,
            vec![
                ("a".to_string(), false),
                ("b".to_string(), true),
                ("c".to_string(), true)
            ]
        );

        // nothing changed, so nothing is pushed into a page again
        assert!(voice.pending_locks(&rail).is_empty());
    }

    #[test]
    fn ending_the_call_unlocks_every_server() {
        let voice = VoiceState::default();
        let rail = ["a".to_string(), "b".to_string()];

        join(&voice, "a", 1);
        voice.pending_locks(&rail);

        voice.report("a", "server", "account", None);

        assert_eq!(voice.pending_locks(&rail), vec![("b".to_string(), false)]);
    }

    #[test]
    fn forgetting_an_entry_drops_its_session_and_its_lock() {
        let voice = VoiceState::default();
        let rail = ["a".to_string(), "b".to_string()];

        join(&voice, "a", 1);
        voice.pending_locks(&rail);

        voice.forget_entry("a");

        assert!(voice.holder().is_none());
        // b is unlocked again, and a is re-stated rather than remembered as already locked
        assert_eq!(
            voice.pending_locks(&rail),
            vec![("a".to_string(), false), ("b".to_string(), false)]
        );
    }
}
