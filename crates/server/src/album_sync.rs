//! Album-fill set math (M3-T5 → POSTSHIP-T29).
//!
//! The poll cycle no longer fetches a fresh page of Immich assets and pushes
//! the new ones; it scans the whole pre-processed `asset_index` (T28), decides
//! which assets match, and then reconciles that match set against the live
//! album. [`compute_album_plan`] is the pure heart of that reconciliation —
//! given the matched ids, the album's current member ids, and what the rule
//! has filed/removed before, it returns exactly what to PUT, what to mark as
//! operator-removed, and the membership baseline to persist.
//!
//! Locked decision D3 (`.ralph/TASKS.md`): managed albums RESPECT manual
//! removals. An asset the rule previously filed (`album_managed_assets.state =
//! 'added'`) that the operator has since pulled out of the album must never be
//! re-added — even though it still matches. That's why the plan subtracts both
//! the previously-recorded removals AND the removals detected this pass from
//! the add set.
//!
//! The HTTP side (GET album ids, PUT new ids) and the `album_managed_assets`
//! writes live in `engine_cycle::fill_album`, which calls this function; the
//! Immich `PUT /api/albums/:id/assets` is itself idempotent for already-present
//! ids, so the diff is about keeping the body small and respecting removals,
//! not about correctness of the PUT.

use std::collections::HashSet;

/// The reconciliation result for one album-fill pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumPlan {
    /// Matched assets to PUT into the album (not already present, not removed).
    /// Preserves the order of the input `matched` slice.
    pub to_add: Vec<String>,
    /// Assets the rule previously filed (`added`) that are no longer in the
    /// live album — the operator removed them. Recorded `removed` so they're
    /// never re-added. Sorted for determinism.
    pub newly_removed: Vec<String>,
    /// Membership baseline to persist as `added`: every matched asset that ends
    /// up in the album this pass (already present or freshly added), minus any
    /// removed asset. Lets a future pass distinguish "operator pulled this out"
    /// from "never added". Preserves the order of the input `matched` slice.
    pub added_baseline: Vec<String>,
}

/// Pure album reconciliation (D3). Inputs:
/// * `matched` — asset ids the rule matched this pass (ordered, no dups).
/// * `in_album` — asset ids currently in the live Immich album.
/// * `prior_added` — asset ids this rule previously recorded as `added`.
/// * `removed_set` — asset ids this rule previously recorded as `removed`.
///
/// The add set excludes assets already in the album, assets the operator
/// removed before (`removed_set`), AND assets detected as removed this pass
/// (`newly_removed`) — the last one is the bug-prone case: an asset the rule
/// filed, the operator deleted from the album, and that still matches would be
/// re-added on the next pass without this subtraction.
pub fn compute_album_plan(
    matched: &[String],
    in_album: &HashSet<String>,
    prior_added: &HashSet<String>,
    removed_set: &HashSet<String>,
) -> AlbumPlan {
    // Operator removals: assets we filed that are no longer in the album.
    let mut newly_removed: Vec<String> = prior_added
        .iter()
        .filter(|id| !in_album.contains(id.as_str()))
        .cloned()
        .collect();
    newly_removed.sort();

    // Everything we must NOT (re-)add: prior removals + the ones just detected.
    let mut effective_removed: HashSet<&str> = removed_set.iter().map(String::as_str).collect();
    for id in &newly_removed {
        effective_removed.insert(id.as_str());
    }

    let to_add: Vec<String> = matched
        .iter()
        .filter(|id| !in_album.contains(id.as_str()) && !effective_removed.contains(id.as_str()))
        .cloned()
        .collect();
    let to_add_set: HashSet<&str> = to_add.iter().map(String::as_str).collect();

    // Baseline = matched assets that are in the album after this pass (already
    // there or just added), excluding anything removed.
    let added_baseline: Vec<String> = matched
        .iter()
        .filter(|id| {
            !effective_removed.contains(id.as_str())
                && (in_album.contains(id.as_str()) || to_add_set.contains(id.as_str()))
        })
        .cloned()
        .collect();

    AlbumPlan {
        to_add,
        newly_removed,
        added_baseline,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn set(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn v(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_album_adds_all_matches() {
        let plan = compute_album_plan(&v(&["a", "b", "c"]), &set(&[]), &set(&[]), &set(&[]));
        assert_eq!(plan.to_add, v(&["a", "b", "c"]));
        assert!(plan.newly_removed.is_empty());
        assert_eq!(plan.added_baseline, v(&["a", "b", "c"]));
    }

    #[test]
    fn already_present_matches_are_not_re_added_but_stay_in_baseline() {
        // a, b already in album; c is new. Only c is PUT, but all three are the
        // membership baseline.
        let plan = compute_album_plan(
            &v(&["a", "b", "c"]),
            &set(&["a", "b"]),
            &set(&["a", "b"]),
            &set(&[]),
        );
        assert_eq!(plan.to_add, v(&["c"]));
        assert!(plan.newly_removed.is_empty());
        assert_eq!(plan.added_baseline, v(&["a", "b", "c"]));
    }

    #[test]
    fn operator_removal_is_detected_and_never_re_added() {
        // We filed a + b. The album now only has b → operator removed a. a still
        // matches, but must NOT be re-added; it's recorded as newly_removed.
        let plan = compute_album_plan(&v(&["a", "b"]), &set(&["b"]), &set(&["a", "b"]), &set(&[]));
        assert!(plan.to_add.is_empty(), "a must not be re-added");
        assert_eq!(plan.newly_removed, v(&["a"]));
        assert_eq!(plan.added_baseline, v(&["b"]), "only b remains managed");
    }

    #[test]
    fn previously_removed_match_is_respected() {
        // a was recorded removed earlier and isn't in the album. It still
        // matches, but the prior removal sticks — no PUT, no baseline.
        let plan = compute_album_plan(&v(&["a", "b"]), &set(&["b"]), &set(&["b"]), &set(&["a"]));
        assert_eq!(plan.to_add, v(&[] as &[&str]));
        assert!(
            plan.newly_removed.is_empty(),
            "a was already removed, not newly removed",
        );
        assert_eq!(plan.added_baseline, v(&["b"]));
    }

    #[test]
    fn no_matches_with_prior_added_detects_full_removal() {
        // The rule matches nothing now, but it had filed a + b and the album is
        // empty → both detected as operator removals.
        let plan = compute_album_plan(&[], &set(&[]), &set(&["a", "b"]), &set(&[]));
        assert!(plan.to_add.is_empty());
        assert_eq!(plan.newly_removed, v(&["a", "b"]));
        assert!(plan.added_baseline.is_empty());
    }
}
