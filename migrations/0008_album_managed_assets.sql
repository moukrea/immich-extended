-- Album membership tracking for managed album fills (POSTSHIP cycle 5 / T26 + T29).
--
-- Locked decision D3: managed albums must RESPECT manual removals. To do that
-- the engine has to remember which assets *it* filed into a rule's album, so a
-- later fill pass can tell "operator pulled this out" (we recorded `added`, the
-- live album no longer contains it) apart from "never added yet".
--
-- `state` is `added` (we put it in the album and intend to keep it there) or
-- `removed` (we detected the operator removed it; never re-add). The full diff
-- logic that writes `removed` lands in T29; T26 only starts populating `added`
-- rows on each successful Immich PUT so T29 has a baseline.
--
-- Composite PK `(rule_id, asset_id)` mirrors `asset_decisions`: one membership
-- verdict per (rule, asset). FK + cascade so the rows die with the rule.
--
-- Timestamp convention: INTEGER unix-seconds, matching every other table.

CREATE TABLE IF NOT EXISTS album_managed_assets (
    rule_id     TEXT    NOT NULL REFERENCES rules(id) ON DELETE CASCADE,
    asset_id    TEXT    NOT NULL,
    state       TEXT    NOT NULL CHECK (state IN ('added', 'removed')),
    changed_at  INTEGER NOT NULL,
    PRIMARY KEY (rule_id, asset_id)
);

CREATE INDEX IF NOT EXISTS album_managed_assets_rule_id_state_idx
    ON album_managed_assets (rule_id, state);
