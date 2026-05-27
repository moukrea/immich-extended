# immich-extended HTTP API reference

All endpoints are JSON unless noted. Cookie auth (session) is required on every
`/api/v1/*` route except `/api/v1/auth/login` and `/api/v1/auth/oidc/*`. The
session cookie is set by a successful login.

Base URL: `https://<host>` (typically `https://immich-ext.<DOMAIN>` behind
Traefik). All paths below are absolute.

---

## Health

### `GET /health`
Liveness + DB probe. No auth.

`200 OK`
```json
{ "status": "ok", "version": "0.1.0", "db": "ok" }
```

---

## Auth

### `POST /api/v1/auth/login`
Local-account login.

Request:
```json
{ "email": "user@example.com", "password": "..." }
```

Responses:
- `200 OK` + `Set-Cookie: session=...` on valid credentials.
- `401 Unauthorized` on invalid credentials.

### `POST /api/v1/auth/logout`
Clears the session cookie. Always `204 No Content`.

### `GET /api/v1/auth/me`
Returns the current session's user.

`200 OK`
```json
{ "id": "<uuid>", "email": "...", "display_name": "...", "is_admin": false }
```

### `GET /api/v1/auth/oidc/login`
Initiates OIDC login. Server generates state + PKCE, sets a short-lived
correlation cookie, and `303 See Other` to the configured issuer's
authorize endpoint.

When OIDC is not configured, returns `404 Not Found`.

### `GET /api/v1/auth/oidc/callback?code=...&state=...`
OIDC callback. Exchanges the code, validates the ID token, creates or
updates the local user, sets the session cookie, and `303 See Other` to `/`.

---

## Setup

### `GET /api/v1/setup/state`
Whether the deployment needs first-time setup (zero users).

`200 OK`
```json
{ "needs_initial_setup": true }
```

### `POST /api/v1/setup/initial`
Creates the first admin user when `needs_initial_setup` is `true`. Closed
permanently after first success.

Request:
```json
{ "email": "admin@example.com", "password": "...", "display_name": "Admin" }
```

---

## Per-user Immich credentials

### `POST /api/v1/me/immich-key`
Validate and persist an Immich API key for the current user. The server
calls Immich's `/api/users/me` with the key, then encrypts and stores it
using `IMMICH_EXT_MASTER_KEY` (AES-256-GCM).

Request:
```json
{ "base_url": "https://immich.example.com", "api_key": "..." }
```

Responses:
- `200 OK` with `{ base_url, immich_user_id, last_validated_at }` on success.
- `400 Bad Request` if Immich rejected the key.

### `GET /api/v1/me/immich-key`
Returns the stored metadata (never the key itself).

### `DELETE /api/v1/me/immich-key`
Removes the stored credentials.

---

## Immich proxies (per-user, key-authenticated server-side)

### `GET /api/v1/me/people`
Lists the current user's Immich persons. Internally paginates Immich's
`/api/people` (capped at 30/page) and merges.

`200 OK`
```json
{ "people": [{ "id": "<uuid>", "name": "...", "thumbnailPath": "..." }] }
```

### `GET /api/v1/me/people/:id/thumbnail`
Streams the JPEG thumbnail bytes from Immich for the given person. Used by
the frontend `<img src="...">` to render people pickers.

### `GET /api/v1/me/albums`
Lists albums the user can write to (owner OR `albumUsers[].role==editor`).

`200 OK`
```json
{ "albums": [{ "id": "<uuid>", "name": "...", "asset_count": 123, "is_writable": true }] }
```

---

## Rules

All rule routes are scoped to the current user; foreign access yields `404`
(rules) or `400` (foreign person/album refs).

### `POST /api/v1/rules`
Creates a rule from YAML. Request body is `application/x-yaml` (or
`text/plain`; the server parses by content type).

```yaml
name: "Family"
target_album: { type: managed, name: "Family" }
match:
  people: { must_include: ["<person-uuid>"] }
status: active
```

Responses:
- `201 Created` + body `{ id, name, status, parsed }`.
- `400 Bad Request` with `{ error, message }` on YAML errors, foreign IDs,
  empty match, or unwritable albums.

### `GET /api/v1/rules`
Lists the current user's rules.

`200 OK`
```json
{ "rules": [{ "id", "name", "status", "updated_at" }] }
```

### `GET /api/v1/rules/:id`
Returns full rule body + parsed predicates.

### `PATCH /api/v1/rules/:id`
Update YAML and/or status (`active|paused|archived`). Body fields are all
optional; only the supplied fields are touched.

### `DELETE /api/v1/rules/:id`
Removes the rule and tears down its scheduler task.

### `GET /api/v1/rules/:id/decisions?cursor=...&reason=...`
Paginated decision log for a rule. `reason` accepts `matched`,
`date_out_of_range`, `media_type_mismatch`, `location_out_of_range`,
`person_missing`, `person_excluded`, `yolo_unidentified_human`,
`asset_missing`, `album_unwritable`.

`200 OK`
```json
{
  "decisions": [{ "asset_id", "decision": "added|skipped", "reason", "decided_at" }],
  "next_cursor": "<opaque>|null"
}
```

---

## Error envelope

All non-2xx responses use a stable JSON envelope:

```json
{ "error": "<machine_code>", "message": "human-readable" }
```

Common codes: `invalid_yaml`, `foreign_resource`, `empty_match`,
`unwritable_album`, `not_found`, `unauthorized`, `internal`.
