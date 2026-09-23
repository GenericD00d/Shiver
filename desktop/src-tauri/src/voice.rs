//! One voice call across every server: which entry holds it, and which pages must refuse to start one.
//!
//! Sharkord keeps the mute state in its own redux store, so pages report it by reading their own
//! controls and Shiver acts by clicking them (see the bridge).

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use shiver_core::LockExt;

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
    /// Sharkord disables its mic control while deafened
    #[serde(default)]
    pub mic_locked: bool,
}

/// The call shown in the rail.
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

const MAX_CHANNEL_NAME: usize = 100;

#[derive(Debug, Clone)]
struct Session {
    snapshot: VoiceSnapshot,
    server_name: String,
    account_label: String,
    /// join order; the earliest joiner holds the call
    order: u64,
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, Session>,
    /// the lock last pushed to each page, so unchanged locks are not re-sent
    locks: HashMap<String, bool>,
    next_order: u64,
}

#[derive(Default)]
pub struct VoiceState(Mutex<State>);

impl VoiceState {
    /// Records one page's report. Returns whether the rail's view changed.
    pub fn report(
        &self,
        entry_id: &str,
        server_name: &str,
        account_label: &str,
        snapshot: Option<VoiceSnapshot>,
    ) -> bool {
        let mut state = self.0.locked();

        let Some(mut snapshot) = snapshot else {
            return state.sessions.remove(entry_id).is_some();
        };

        snapshot.channel_name = snapshot
            .channel_name
            .map(|name| name.chars().take(MAX_CHANNEL_NAME).collect());

        if let Some(existing) = state.sessions.get_mut(entry_id) {
            if existing.snapshot == snapshot && existing.server_name == server_name {
                return false;
            }

            // moving channel on the same server keeps its place in the queue
            existing.snapshot = snapshot;
            existing.server_name = server_name.to_string();
            existing.account_label = account_label.to_string();

            return true;
        }

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

        true
    }

    fn holder_id(state: &State) -> Option<String> {
        state
            .sessions
            .iter()
            .min_by_key(|(_, session)| session.order)
            .map(|(entry_id, _)| entry_id.clone())
    }

    pub fn status(&self) -> Option<VoiceStatus> {
        let state = self.0.locked();
        let entry_id = Self::holder_id(&state)?;
        let session = state.sessions.get(&entry_id)?;

        Some(VoiceStatus {
            server_name: session.server_name.clone(),
            account_label: session.account_label.clone(),
            channel_id: session.snapshot.channel_id,
            channel_name: session.snapshot.channel_name.clone(),
            mic_muted: session.snapshot.mic_muted,
            sound_muted: session.snapshot.sound_muted,
            mic_locked: session.snapshot.mic_locked,
            entry_id,
        })
    }

    pub fn holder(&self) -> Option<String> {
        Self::holder_id(&self.0.locked())
    }

    /// Entries in a call that is not the recognised one; they are told to hang up.
    pub fn intruders(&self) -> Vec<String> {
        let state = self.0.locked();
        let holder = Self::holder_id(&state);

        state
            .sessions
            .keys()
            .filter(|entry_id| Some(*entry_id) != holder.as_ref())
            .cloned()
            .collect()
    }

    /// Pages whose voice lock must change, and to what. Only differences are returned.
    pub fn pending_locks(&self, entry_ids: &[String]) -> Vec<(String, bool)> {
        let mut state = self.0.locked();
        let holder = Self::holder_id(&state);

        state
            .locks
            .retain(|entry_id, _| entry_ids.contains(entry_id));

        entry_ids
            .iter()
            .filter_map(|entry_id| {
                let locked = holder.as_ref().is_some_and(|holder| holder != entry_id);

                (state.locks.insert(entry_id.clone(), locked) != Some(locked))
                    .then(|| (entry_id.clone(), locked))
            })
            .collect()
    }

    /// Forgets an entry's session and lock (its page closed, or it left the rail).
    pub fn forget_entry(&self, entry_id: &str) {
        let mut state = self.0.locked();

        state.sessions.remove(entry_id);
        state.locks.remove(entry_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn join(voice: &VoiceState, entry_id: &str, channel_id: i64) {
        voice.report(
            entry_id,
            "server",
            "account",
            Some(VoiceSnapshot {
                channel_id,
                channel_name: Some(format!("channel {channel_id}")),
                mic_muted: false,
                sound_muted: false,
                mic_locked: false,
            }),
        );
    }

    #[test]
    fn the_first_to_join_holds_the_call_even_when_moving_channel() {
        let voice = VoiceState::default();

        join(&voice, "z", 1);
        join(&voice, "a", 2);
        join(&voice, "z", 3);

        assert_eq!(voice.holder().as_deref(), Some("z"));
        assert_eq!(voice.intruders(), vec!["a".to_string()]);

        voice.report("z", "server", "account", None);

        assert_eq!(voice.holder().as_deref(), Some("a"));
        assert!(voice.intruders().is_empty());
    }

    #[test]
    fn locks_are_pushed_once_per_change() {
        let voice = VoiceState::default();
        let rail = ["a".to_string(), "b".to_string()];

        join(&voice, "a", 1);

        let mut locks = voice.pending_locks(&rail);

        locks.sort();
        assert_eq!(
            locks,
            vec![("a".to_string(), false), ("b".to_string(), true)]
        );
        assert!(voice.pending_locks(&rail).is_empty());

        voice.forget_entry("a");
        assert_eq!(
            voice.pending_locks(&rail),
            vec![("a".to_string(), false), ("b".to_string(), false)]
        );
    }

    #[test]
    fn an_unchanged_report_is_silent() {
        let voice = VoiceState::default();

        join(&voice, "a", 1);
        assert!(!voice.report(
            "a",
            "server",
            "account",
            Some(VoiceSnapshot {
                channel_id: 1,
                channel_name: Some("channel 1".into()),
                mic_muted: false,
                sound_muted: false,
                mic_locked: false
            })
        ));
    }
}
