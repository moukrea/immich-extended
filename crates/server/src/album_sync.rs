//! Idempotent album-add helper (M3-T5).
//!
//! Wraps the GET → diff → PUT dance the poll cycle uses to push matched assets
//! into a rule's target album. The diff is an optimisation, not a correctness
//! requirement: Immich's `PUT /api/albums/:id/assets` is itself idempotent for
//! already-present ids. The point of the helper is to keep the PUT body small,
//! avoid spurious round-trips when nothing new is matched, and produce a small
//! observable surface (one number: how many ids actually went over the wire).
//!
//! The GET-then-PUT window can race against another client adding the same
//! asset — that's acceptable for v1; the worst case is we send a redundant id
//! and Immich no-ops on it.

use std::collections::HashSet;

use immich_client::{ImmichClient, ValidationError};

/// Push `candidate_ids` into `album_id`, but only the ids not already there.
/// Returns the count of ids actually PUT to Immich.
///
/// Short-circuits:
/// * `candidate_ids.is_empty()` → `Ok(0)` with no HTTP calls.
/// * `album_id.is_empty()` → `Ok(0)` (managed-target rule whose album hasn't
///   been created yet; the cycle still records decisions but skips the push).
/// * Diff resolves to no new ids → `Ok(0)`, no PUT.
pub async fn idempotent_album_add(
    immich: &ImmichClient,
    api_key: &str,
    album_id: &str,
    candidate_ids: &[String],
) -> Result<usize, ValidationError> {
    if candidate_ids.is_empty() || album_id.is_empty() {
        return Ok(0);
    }

    let existing = immich.get_album_asset_ids(api_key, album_id).await?;
    let to_add = diff_new_ids(&existing, candidate_ids);

    if to_add.is_empty() {
        return Ok(0);
    }

    immich
        .add_assets_to_album(api_key, album_id, &to_add)
        .await?;
    Ok(to_add.len())
}

/// Pure diff: subset of `candidate_ids` not already in `existing`, preserving
/// input order. Extracted from `idempotent_album_add` so it can be unit-tested
/// without a wiremock.
fn diff_new_ids(existing: &HashSet<String>, candidate_ids: &[String]) -> Vec<String> {
    candidate_ids
        .iter()
        .filter(|id| !existing.contains(id.as_str()))
        .cloned()
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn diff_returns_only_new_ids_preserving_order() {
        let existing: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let candidates = vec![
            "a".to_string(),
            "c".to_string(),
            "b".to_string(),
            "d".to_string(),
        ];
        let out = diff_new_ids(&existing, &candidates);
        assert_eq!(out, vec!["c".to_string(), "d".to_string()]);
    }

    #[test]
    fn diff_with_empty_existing_returns_all_candidates() {
        let existing: HashSet<String> = HashSet::new();
        let candidates = vec!["a".to_string(), "b".to_string()];
        assert_eq!(diff_new_ids(&existing, &candidates), candidates);
    }

    #[test]
    fn diff_with_all_candidates_already_present_returns_empty() {
        let existing: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let candidates = vec!["a".to_string(), "b".to_string()];
        let out = diff_new_ids(&existing, &candidates);
        assert!(out.is_empty());
    }
}
