//! In-memory live-activity ring buffer (POSTSHIP cycle 5 / T33).
//!
//! Powers the global `/activity` live log. The background indexer
//! ([`crate::indexer`]) and the per-rule poll cycle ([`crate::engine_cycle`])
//! publish small events here as they process assets; the SPA polls
//! `GET /api/v1/me/activity/stream?after=<seq>` every couple of seconds and
//! renders the tail.
//!
//! Design contract: `docs/design/preprocessing-index.md` §5. Chosen over an
//! events table because the live log is ephemeral ("what's happening right
//! now") — a bounded ring buffer needs no migration, no retention job, and no
//! write amplification on the hot indexing path. Durability across restarts is
//! deliberately *not* wanted.
//!
//! ### Ordering & the `seq` cursor
//!
//! Every event gets a process-monotonic `seq`. The poller asks for everything
//! `after` the last `seq` it saw, so it never double-counts. `seq` is assigned
//! while the buffer lock is held, so the deque always stays in `seq` order even
//! when the indexer task and several rule-cycle tasks publish concurrently. The
//! buffer is bounded ([`CAP`]); once full the oldest event is dropped, so a poll
//! can miss events under a heavy burst — acceptable for a live tail, and the
//! returned `last_seq` still advances the client cursor past the gap.
//!
//! ### Per-account isolation (PRD §12)
//!
//! Every event carries a `user_id`; [`ActivityBus::since`] filters to the
//! caller's id and the `/me/activity/stream` endpoint passes the session user.
//! `user_id` is never serialized to the wire — the client only ever sees its
//! own events, so it is redundant there.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Maximum events retained across all users (design §5.1). O(CAP) memory
/// regardless of library size; the client caps its own rendered list too.
const CAP: usize = 500;

/// The payload of a live-log event, tagged by `kind` on the wire so the SPA can
/// render a discriminated union. Each variant is one line in the log.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActivityKind {
    /// The indexer upserted one asset's metadata. `asset_id` lets the SPA
    /// correlate this line with the asset's later `Matched`/`Skipped` events
    /// when grouping the flat stream per-asset (cycle-6 §8.2).
    Indexed {
        asset_id: String,
        filename: String,
        person_count: i64,
        has_gps: bool,
        taken_at: Option<i64>,
    },
    /// A rule matched an asset (verdict `added`).
    Matched {
        rule_id: String,
        rule_name: String,
        asset_id: String,
        filename: Option<String>,
    },
    /// A rule skipped an asset, with the reason slug.
    Skipped {
        rule_id: String,
        rule_name: String,
        asset_id: String,
        filename: Option<String>,
        reason: String,
    },
    /// A rule filed `added_count` newly-matched assets into its album.
    AlbumAdd {
        rule_id: String,
        rule_name: String,
        album_id: String,
        added_count: i64,
    },
    /// End of one indexer sweep for a user.
    SweepDone { indexed: i64, took_ms: i64 },
}

/// A live-log entry: a `seq`-stamped, time-stamped, user-scoped [`ActivityKind`].
#[derive(Debug, Clone, Serialize)]
pub struct ActivityEvent {
    pub seq: u64,
    /// Unix seconds the event was published.
    pub at: i64,
    /// Owning user. Skipped on the wire (the client only ever sees its own).
    #[serde(skip)]
    pub user_id: String,
    #[serde(flatten)]
    pub kind: ActivityKind,
}

/// Bounded, process-wide, per-user-filterable ring buffer of recent processing
/// events. Held in [`crate::AppState`] and cloned (via `Arc`) into the indexer
/// and the scheduler's tick function.
#[derive(Debug)]
pub struct ActivityBus {
    inner: Mutex<VecDeque<ActivityEvent>>,
    seq: AtomicU64,
}

impl Default for ActivityBus {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityBus {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(CAP)),
            seq: AtomicU64::new(0),
        }
    }

    /// Append one event for `user_id`. `seq` is assigned under the buffer lock
    /// so the deque stays ordered; the oldest event is evicted at capacity. The
    /// lock is held only for the synchronous push (never across an `.await`).
    fn push(&self, user_id: &str, kind: ActivityKind) {
        // A poisoned lock means another publisher panicked mid-push; the buffer
        // is still structurally sound, so recover the guard rather than panic
        // (publishing must never take down a sweep or a poll).
        let mut buf = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        if buf.len() >= CAP {
            buf.pop_front();
        }
        buf.push_back(ActivityEvent {
            seq,
            at: now_unix_seconds(),
            user_id: user_id.to_string(),
            kind,
        });
    }

    /// Indexer upserted `filename` (asset `asset_id`) for `user_id`.
    pub fn indexed(
        &self,
        user_id: &str,
        asset_id: &str,
        filename: &str,
        person_count: i64,
        has_gps: bool,
        taken_at: Option<i64>,
    ) {
        self.push(
            user_id,
            ActivityKind::Indexed {
                asset_id: asset_id.to_string(),
                filename: filename.to_string(),
                person_count,
                has_gps,
                taken_at,
            },
        );
    }

    /// Rule `rule_name` matched `asset_id` for `user_id`.
    pub fn matched(
        &self,
        user_id: &str,
        rule_id: &str,
        rule_name: &str,
        asset_id: &str,
        filename: Option<&str>,
    ) {
        self.push(
            user_id,
            ActivityKind::Matched {
                rule_id: rule_id.to_string(),
                rule_name: rule_name.to_string(),
                asset_id: asset_id.to_string(),
                filename: filename.map(str::to_string),
            },
        );
    }

    /// Rule `rule_name` skipped `asset_id` for `user_id` with `reason`.
    pub fn skipped(
        &self,
        user_id: &str,
        rule_id: &str,
        rule_name: &str,
        asset_id: &str,
        filename: Option<&str>,
        reason: &str,
    ) {
        self.push(
            user_id,
            ActivityKind::Skipped {
                rule_id: rule_id.to_string(),
                rule_name: rule_name.to_string(),
                asset_id: asset_id.to_string(),
                filename: filename.map(str::to_string),
                reason: reason.to_string(),
            },
        );
    }

    /// Rule `rule_name` filed `added_count` assets into `album_id`.
    pub fn album_add(
        &self,
        user_id: &str,
        rule_id: &str,
        rule_name: &str,
        album_id: &str,
        added_count: i64,
    ) {
        self.push(
            user_id,
            ActivityKind::AlbumAdd {
                rule_id: rule_id.to_string(),
                rule_name: rule_name.to_string(),
                album_id: album_id.to_string(),
                added_count,
            },
        );
    }

    /// One indexer sweep for `user_id` finished.
    pub fn sweep_done(&self, user_id: &str, indexed: i64, took_ms: i64) {
        self.push(user_id, ActivityKind::SweepDone { indexed, took_ms });
    }

    /// Events for `user_id` with `seq > after`, oldest first, plus the current
    /// high-water `seq` so the caller can advance its cursor even past dropped
    /// events.
    pub fn since(&self, user_id: &str, after: u64) -> (Vec<ActivityEvent>, u64) {
        let buf = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let last_seq = self.seq.load(Ordering::Relaxed);
        let events: Vec<ActivityEvent> = buf
            .iter()
            .filter(|e| e.user_id == user_id && e.seq > after)
            .cloned()
            .collect();
        (events, last_seq)
    }
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn since_filters_by_user_and_seq() {
        let bus = ActivityBus::new();
        bus.indexed("alice", "asset-a", "a.jpg", 1, true, Some(100));
        bus.indexed("bob", "asset-b", "b.jpg", 0, false, None);
        bus.matched("alice", "r1", "Rule One", "asset-1", Some("a.jpg"));

        let (alice, last) = bus.since("alice", 0);
        assert_eq!(alice.len(), 2, "alice sees only her two events");
        assert_eq!(last, 3, "high-water seq spans all publishers");
        assert!(matches!(alice[0].kind, ActivityKind::Indexed { .. }));
        assert!(matches!(alice[1].kind, ActivityKind::Matched { .. }));

        let (bob, _) = bus.since("bob", 0);
        assert_eq!(bob.len(), 1, "bob never sees alice's events");

        // `after` cursor excludes already-seen events.
        let (after_first, _) = bus.since("alice", alice[0].seq);
        assert_eq!(after_first.len(), 1);
        assert_eq!(after_first[0].seq, alice[1].seq);
    }

    #[test]
    fn seq_is_monotonic_and_starts_at_one() {
        let bus = ActivityBus::new();
        bus.sweep_done("u", 5, 12);
        let (events, last) = bus.since("u", 0);
        assert_eq!(events[0].seq, 1);
        assert_eq!(last, 1);
    }

    #[test]
    fn buffer_evicts_oldest_past_capacity() {
        let bus = ActivityBus::new();
        for _ in 0..(CAP + 50) {
            bus.indexed("u", "asset-x", "x.jpg", 0, false, None);
        }
        let (events, last) = bus.since("u", 0);
        assert_eq!(events.len(), CAP, "retains at most CAP events");
        assert_eq!(last, (CAP + 50) as u64, "seq keeps counting past evictions");
        // Oldest retained event is the (51st) push, seq = 51.
        assert_eq!(events[0].seq, 51);
    }

    #[test]
    fn skipped_event_serializes_with_kind_tag_and_no_user_id() {
        let bus = ActivityBus::new();
        bus.skipped(
            "u",
            "r1",
            "Rule",
            "asset-9",
            Some("p.jpg"),
            "date_out_of_range",
        );
        let (events, _) = bus.since("u", 0);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["kind"], "skipped");
        assert_eq!(json["reason"], "date_out_of_range");
        assert_eq!(json["filename"], "p.jpg");
        assert_eq!(json["rule_name"], "Rule");
        assert!(
            json.get("user_id").is_none(),
            "user_id must not leak to the wire"
        );
    }

    #[test]
    fn indexed_event_serializes_asset_id_for_client_grouping() {
        let bus = ActivityBus::new();
        bus.indexed("u", "asset-7", "p.jpg", 3, true, Some(1_700_000_000));
        let (events, _) = bus.since("u", 0);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["kind"], "indexed");
        assert_eq!(json["asset_id"], "asset-7");
        assert_eq!(json["filename"], "p.jpg");
        assert_eq!(json["person_count"], 3);
        assert_eq!(json["has_gps"], true);
        assert!(
            json.get("user_id").is_none(),
            "user_id must not leak to the wire"
        );
    }
}
