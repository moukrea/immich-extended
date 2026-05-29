# POSTSHIP cycle 7 — final verification (T53)

Date: 2026-05-29. Verifies the inline natural-language rule builder **live against
the deployed build** (not the devpreview harness), then closes the cycle.

## Build + redeploy

- New SPA bundle built: `dist/assets/index-Cp45_NL_.js` (202.43 kB / 62.61 kB gzip).
- Docker image `immich-extended:dev` rebuilt (Rust layers cache-hit — no Rust
  changes this cycle; frontend stage rebuilt, fresh `dist` copied). Verified the
  image's `/app/web/dist/assets/index-*.js` filename matches the local build.
- Redeployed via `make up-immich-extended` (`docker compose --env-file ../.env
  --env-file ../.env.local up -d`); container `Recreated` + `healthy`.
- `curl https://immich-ext.<DOMAIN>/health` → **HTTP 200**, `TLS_verify=0`,
  body `{"status":"ok","version":"0.1.0","db":"ok"}`.

## Gates (all green, HEAD `8c77f3c`)

Frontend (`web/`):
- `npm run build` ✓ — 202.43 kB main + 1054 kB lazy MapPicker.
- `npm run lint` ✓ (0w/0e) · `npm run typecheck` ✓.
- `npm test -- --run` ✓ — **296 vitests** across 23 files.

Rust (workspace, `ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1`):
- `cargo fmt --all --check` ✓ · `cargo clippy --all-targets --workspace -- -D warnings` ✓.
- `cargo test --workspace` ✓ — **482 passing** (unit + integration across all bins).

No source files changed during verification, so these remain valid through close-out
(only `docs/` + `.ralph/` change afterward).

## Live D5 verification (Playwright + Chromium 146 against the deployed SPA)

Driven as the local smoke admin `smoke-local@<DOMAIN>` — the same Immich account
(`eb2d5112`) as the operator, so the real Paloma/Emeric/Manon roster resolves.
Script: `/tmp/t53_verify.py`. **15/15 checks passed.** Test rules created during
the run were deleted afterward; the DB is back to the 3 real rules
(`714dce95`, `beba1580`, `bfad0cb9`).

| Check | Result | Evidence |
|---|---|---|
| New inline builder deployed (not old composer) | PASS | Include/Exclude toggle + "+ condition" + readout present; `cycle7-t53-new-empty-dark.png` |
| Compose headline + **2nd may-be-present person** (the original bug) | PASS | readout "Include to album if Paloma is present and Emeric may be present"; `cycle7-t53-headline-compose-dark.png` |
| YAML serializes `may_include` | PASS | Advanced panel shows `mode: must_include` (Paloma) + `mode: may_include` (Emeric) |
| **Save → reload round-trips as a sentence** | PASS | saved rule reopened renders the same sentence, no fallback; `cycle7-t53-reload-roundtrip-dark.png` |
| Include → **Exclude inverse fill** (`Not(...)`) | PASS | readout "Exclude from album if…"; YAML wraps match in `op: not`; `cycle7-t53-exclude-inverse-dark.png` |
| **Two geo Areas** with numbered linked maps | PASS | inline "taken in Area 1/Area 2" pills + 2 numbered MapLibre OSM blocks; `cycle7-t53-geo-areas-dark.png` |
| Operator rule shape #1 (`714dce95`: must_include) loads as sentence | PASS | "Include to album if Paloma is present"; `cycle7-t53-load-existing-dark.png` |
| Operator rule shape #2 (`beba1580`: must+may+face/YOLO) loads as sentence | PASS | "…Paloma is present and Emeric may be present and all faces must be recognized · reject extra humans (YOLO)"; no fallback; `cycle7-t53-load-managed-dark.png` |
| Light theme | PASS | `cycle7-t53-headline-light.png` |

### D5 critical visual assessment

Screenshots were opened and compared to the cycle-7 design + `immich-style-mirror.md`:

- **Reads as a sentence**, not a form: "Include … to album if Paloma is present
  and Emeric may be present." Each pill shows plain language at rest with a `▾`
  edit affordance and `✕` remove; the always-visible readout restates the whole
  rule in English.
- **The marquee bug is fixed**: a second person with the **"may be present"**
  mode is composable via the inline mode dropdown — the exact thing the flat
  builder could not do.
- **Inverse fill** flips the lead to "Exclude from album if" and the serialized
  match to `op: not` wrapping the clause.
- **Geo areas** render as numbered "taken in Area N" pills linked to numbered
  MapLibre blocks below the sentence (real OSM tiles drew via swiftshader WebGL),
  each with a radius control.
- **Immich style**: dark-first palette, left sidebar (Rules/Activity/Settings),
  circular account avatar top-right, rounded cards, `immich-primary` accents on
  the active toggle and the "and" connectors. Matches the deployed Immich look.
- **No corruption**: every operator rule shape loaded into the sentence (none
  fell back to the YAML panel); the conservative loader's fallback path was not
  triggered for the real rules.

Conclusion: the deployed build satisfies every cycle-7 locked decision (L1–L5)
and fixes the operator's reported bug. Cycle 7 is complete.
