# CLAUDE.md — immich-extended behavioral directives

Read each iteration. Stack + conventions + machine-specific tool access. The PRD is *what*; this is *how*.

## Stack (locked)

- **Language**: Rust (stable, latest)
- **HTTP**: `axum` + `tokio`
- **DB**: SQLite via `sqlx` (offline mode; commit `.sqlx/`)
- **Frontend**: SolidJS + TypeScript + Tailwind, Vite
- **Map**: MapLibre GL JS + OpenStreetMap tiles
- **YOLO**: `ort` crate (ONNX Runtime), CPU; YOLOv8n or YOLOv11n
- **Auth**: `openidconnect` crate for OIDC, `argon2` for local passwords
- **Crypto**: AES-256-GCM via `aes-gcm`
- **Image/video**: `image` crate; `ffmpeg-next` or subprocess
- **Logging**: `tracing` + `tracing-subscriber`
- **Config**: env vars

## Credentials store

Single source of truth: **`~/code/immich-extended/.ralph/creds.env`**. Gitignored (whole `.ralph/` dir is). Chmod 600.

Load it at start of every iteration:
```bash
set -a; source ~/code/immich-extended/.ralph/creds.env; set +a
```

`SUDO_PASSWORD` is pre-seeded. Everything else (`AUTHENTIK_BOOTSTRAP_TOKEN`, `IMMICH_BASE_URL`, `DOMAIN`, etc.) you discover from `~/server/` by grepping broadly (`.md`, `.env`, `.yml`, anywhere), then `cat >> creds.env`. If a credential goes stale, re-grep and overwrite.

OIDC credentials you generate (when configuring Authentik for immich-extended) also go in `creds.env`.

**Never log credential VALUES** outside `creds.env`. Reference by env var name in commits, journal, state files.

## Other tool access

- **`gh`** — already authenticated on this burner.
- **`sudo`** — password in `creds.env`. Invoke as `echo "$SUDO_PASSWORD" | sudo -S <cmd>`.
- **Chrome MCP** — for Authentik admin UI and any flow resisting API automation.
- **`~/server/`** — full read/write. Reuse patterns when deploying.

## Workspace layout

```
immich-extended/
├── Cargo.toml                  (workspace)
├── crates/
│   ├── server/                 (binary)
│   ├── engine/                 (rule engine, predicates, scheduler)
│   ├── immich-client/
│   ├── yolo/
│   └── common/
├── web/                        (SolidJS frontend)
├── migrations/                 (sqlx)
├── Dockerfile
├── docker-compose.yml          (local testing)
├── PRD.md                      (immutable)
├── CLAUDE.md                   (this file)
└── README.md
```

## Conventions

### Rust
- `thiserror` for libraries, `anyhow` in the binary crate.
- No `unwrap`/`expect` (outside tests), `unimplemented!()`, `todo!()`, stub `Ok(())` returns.
- Async everywhere except startup/shutdown.

### SQL
- `sqlx migrate add <name>`. Never edit a committed migration.
- `sqlx::query!` / `query_as!`. Commit `.sqlx/`.

### Frontend
- TypeScript strict, no `any`.
- Tailwind utilities only.
- SolidJS primitives, no extra form library.

### Commits
- Conventional, present tense, no period.
- One commit per task.

### Tests
- Public functions with logic: unit test.
- API endpoints: integration test.
- Predicates: positive + negative.
- **Required before M3 verifies**: cross-account isolation integration test.

## Immich API

- Base URL from env (discovered into `creds.env` as `IMMICH_BASE_URL`).
- Auth header: `x-api-key: <key>`.
- Pagination: ~250 cap on list endpoints — use watermarks.
- Idempotent album add: GET existing, diff, PUT only new.

## YOLO

- Model at `${DATA_DIR}/models/yolo.onnx`. Download from pinned GitHub release on first run.
- One `ort::Session` in `OnceCell`.
- Preprocess: letterbox 640×640, BGR→RGB, normalize.
- Postprocess: NMS 0.5, conf 0.5, class 0 only.
- Cache in `asset_yolo_cache` keyed by `(asset_id, model_version)`.

## Per-account isolation (required tests)

1. User A rule referencing User B's `person_id` → 4xx.
2. User A rule targeting an unwritable album → 4xx.
3. Engine running User A's rule never calls Immich with User B's key.
4. `asset_decisions` queries from User A's session never return User B's rows.

## Deployment (M7)

- Discover stack patterns from `~/server/`. Persist to `creds.env`.
- Create `~/server/immich-extended/docker-compose.yml` matching them.
- Authentik OIDC: API first (token from `creds.env`), Chrome MCP fallback.
- Post-deploy: `/health` returns 200 over HTTPS, real rule + real Immich + real poll → real decision.

## When unsure

Read more context. Try different approaches. Don't mark `[BLOCKED]`. Don't escalate. Don't quit.
