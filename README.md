# immich-extended

A rule engine that sits next to [Immich](https://immich.app) and auto-populates
albums based on compound predicates (faces present, faces absent, geographic
zone, time window, and unidentifiable humans detected via YOLO). It fills the
gap left by Immich's built-in Smart Albums, which cannot express rules such as
*"photos of my daughter where I'm also present and no unknown adults are
visible — share with grandma"*.

immich-extended is a sidecar service: Immich keeps doing face recognition,
geolocation, and storage; immich-extended polls Immich's API, evaluates rules,
and synchronizes the target albums. Per-account isolation is preserved — each
user's rules operate only on their own library and face registry.

See [`PRD.md`](./PRD.md) for the full product specification.

## Status

All milestones M0–M7 complete. Deployed as a single container behind Traefik
with Authentik OIDC; verified end-to-end against a live Immich instance
(real rule → real Immich poll → real album side-effect). See [`PRD.md`](./PRD.md)
for scope, [`docs/API.md`](./docs/API.md) for the HTTP surface, and
[`examples/`](./examples/) for sample rule YAMLs.

## Workspace layout

```
crates/
├── server/          axum HTTP server (binary)
├── engine/          rule engine, predicates, scheduler
├── immich-client/   Immich REST API client
├── yolo/            ONNX Runtime YOLO person detector
└── common/          shared types, sqlite pool, migrations
migrations/          sqlx SQL migrations
web/                 SolidJS + Tailwind frontend (M2+)
```

## Required environment variables

| Variable        | Default                                          | Purpose                                      |
| --------------- | ------------------------------------------------ | -------------------------------------------- |
| `HTTP_BIND`     | `0.0.0.0:8080`                                   | HTTP listener bind address                   |
| `LOG_LEVEL`     | `info`                                           | `tracing` env-filter directive               |
| `DATA_DIR`      | `./data`                                         | Directory for SQLite DB, models, cache       |
| `DATABASE_URL`  | `sqlite://${DATA_DIR}/immich-extended.sqlite?mode=rwc` | sqlx connection URL                  |

Additional variables (Immich base URL, OIDC issuer, master encryption key) are
introduced in later milestones — see `PRD.md` §9.

## Local development

Prerequisites: Rust stable (≥ 1.91), SQLite, `pkg-config`, Docker (for the
container smoke test).

```bash
# Build the whole workspace
cargo build --workspace

# Run tests
cargo test --workspace

# Run the server (binds 0.0.0.0:8080 by default)
cargo run -p immich-extended-server

# In another shell
curl -fsS http://127.0.0.1:8080/health
# => {"status":"ok","version":"0.1.0","db":"ok"}
```

## Docker

```bash
# Build the image (multi-stage; cargo-chef for dep caching)
docker build -t immich-extended:dev .

# Run; mount a volume for persistent data
docker run --rm -d \
  -p 18080:8080 \
  -v immich-ext-data:/data \
  --name immich-extended \
  immich-extended:dev

curl -fsS http://127.0.0.1:18080/health
```

The runtime image is `debian:trixie-slim` (~102 MB total). Trixie is required
because the Rust 1.91 builder image links against glibc 2.41; pairing with
`bookworm-slim` (glibc 2.36) yields a runtime `GLIBC_2.39 not found` panic.

## Configuration reference

| Variable                | Required | Purpose                                                                 |
| ----------------------- | -------- | ----------------------------------------------------------------------- |
| `HTTP_BIND`             | no       | HTTP listener bind address (default `0.0.0.0:8080`)                     |
| `LOG_LEVEL`             | no       | `tracing` env-filter directive (default `info`)                         |
| `DATA_DIR`              | no       | SQLite DB, YOLO model, cache (default `./data`)                         |
| `DATABASE_URL`          | no       | sqlx URL (default `sqlite://${DATA_DIR}/immich-extended.sqlite?mode=rwc`) |
| `IMMICH_EXT_MASTER_KEY` | **yes**  | 32-byte hex AES-256-GCM key encrypting stored Immich API keys           |
| `SESSION_COOKIE_SECURE` | no       | Set to `true` when terminating TLS in front of the service              |
| `OIDC_ISSUER_URL`       | no       | Enable OIDC login when set; full discovery URL                          |
| `OIDC_CLIENT_ID`        | with OIDC| OIDC client id                                                          |
| `OIDC_CLIENT_SECRET`    | with OIDC| OIDC client secret                                                      |
| `OIDC_REDIRECT_URL`     | with OIDC| Public callback URL (e.g. `https://immich-ext.<DOMAIN>/api/v1/auth/oidc/callback`) |
| `ORT_DYLIB_PATH`        | dev only | Path to ONNX Runtime `.so` (Docker image bundles its own)              |
| `WEB_DIST_DIR`          | no       | Frontend bundle directory; omit for API-only mode                       |

## Troubleshooting

- **Container exits with `GLIBC_2.XX not found`** — runtime image glibc is
  older than the builder's. The shipped Dockerfile pins both to `trixie`; if
  you change one, change both.
- **`OIDC discovery failed` at boot** — issuer URL unreachable from the
  container's network namespace. Check Traefik routing, DNS, and that the
  issuer's `/.well-known/openid-configuration` returns 200.
- **Immich API key paste returns 4xx** — the key was rejected by Immich's
  `/api/users/me`. Re-mint in Immich (Account → API Keys) and paste again.
- **Rule cycles log `evaluated=N skipped=N` but no `added`** — date or
  location predicate has zero matching assets in the watermark window.
  Widen the date predicate or inspect `GET /api/v1/rules/:id/decisions?reason=date_out_of_range`.
- **`/api/v1/me/people` slow on first call** — Immich paginates persons in
  pages of 30; the proxy walks them all. Subsequent calls are not cached
  (per-request fan-out is deliberate).

## Quality gates

The same commands CI runs:

```bash
cargo fmt --all --check
cargo clippy --all-targets --workspace -- -D warnings
cargo test --workspace
```

No `unwrap`/`expect`/`todo!()`/`unimplemented!()`/`panic!()`/`dbg!()` outside
`#[cfg(test)]` — enforced by workspace lints.

## Roadmap

- **M0 — Skeleton** — workspace, axum `/health`, sqlx, Dockerfile, logging.
- **M1 — Auth & onboarding** — local accounts, OIDC, sessions, encrypted Immich API key, `/setup`.
- **M2 — Rule CRUD** — YAML schema, parser, validator, CRUD API, manual editor UI.
- **M3 — Engine MVP** — per-rule scheduler, Immich client, predicates (date, media, people), idempotent album sync, decision log.
- **M4 — Geo predicate + map UI** — haversine location predicate, MapLibre map picker.
- **M5 — YOLO integration** — `ort`-based person counter, model fetch, video frame sampler, inference cache.
- **M6 — Visual rule builder & polish** — full builder, lifecycle controls, decisions browser, YAML import/export.
- **M7 — Deployment** — Traefik + Authentik OIDC integration, single-container deploy.

## License

MIT
