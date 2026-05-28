export interface SetupState {
  needs_setup: boolean;
  oidc_enabled: boolean;
}

export interface Me {
  user_id: string;
  email: string;
  display_name: string | null;
}

export interface SetupInitialPayload {
  email: string;
  password: string;
  display_name?: string;
  immich_base_url?: string;
  immich_api_key?: string;
}

export interface SetupInitialResult {
  user_id: string;
  email: string;
  display_name: string | null;
  immich_user_id?: string | null;
}

export interface LoginResult {
  user_id: string;
  email: string;
  display_name: string | null;
}

export interface ApiError {
  error: string;
  field?: string;
  [k: string]: unknown;
}

export type ApiResult<T> =
  | { ok: true; data: T }
  | { ok: false; status: number; error: ApiError };

async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<ApiResult<T>> {
  let response: Response;
  try {
    response = await fetch(path, {
      ...init,
      credentials: "include",
      headers: {
        Accept: "application/json",
        ...(init.body ? { "Content-Type": "application/json" } : {}),
        ...(init.headers ?? {}),
      },
    });
  } catch (cause) {
    return {
      ok: false,
      status: 0,
      error: {
        error: "network_error",
        message: cause instanceof Error ? cause.message : String(cause),
      },
    };
  }

  if (response.status === 204) {
    return { ok: true, data: undefined as T };
  }

  let body: unknown = null;
  const text = await response.text();
  if (text.length > 0) {
    try {
      body = JSON.parse(text);
    } catch {
      body = { raw: text };
    }
  }

  if (response.ok) {
    return { ok: true, data: body as T };
  }
  const err: ApiError =
    body && typeof body === "object"
      ? (body as ApiError)
      : { error: "unknown_error" };
  return { ok: false, status: response.status, error: err };
}

export function getSetupState(): Promise<ApiResult<SetupState>> {
  return request<SetupState>("/api/v1/setup/state", { method: "GET" });
}

export function getMe(): Promise<ApiResult<Me>> {
  return request<Me>("/api/v1/auth/me", { method: "GET" });
}

export function postLogin(
  email: string,
  password: string,
): Promise<ApiResult<LoginResult>> {
  return request<LoginResult>("/api/v1/auth/login", {
    method: "POST",
    body: JSON.stringify({ email, password }),
  });
}

export function postLogout(): Promise<ApiResult<void>> {
  return request<void>("/api/v1/auth/logout", { method: "POST" });
}

export function postSetupInitial(
  payload: SetupInitialPayload,
): Promise<ApiResult<SetupInitialResult>> {
  return request<SetupInitialResult>("/api/v1/setup/initial", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export type RuleStatus = "active" | "paused" | "archived";
export type TargetAlbumStrategy = "existing" | "managed";

export interface RuleSummary {
  id: string;
  name: string;
  status: RuleStatus;
  target_album_strategy: TargetAlbumStrategy;
  updated_at: number;
}

export interface ListRulesResponse {
  rules: RuleSummary[];
}

export interface Rule {
  id: string;
  name: string;
  yaml_source: string;
  status: RuleStatus;
  target_album_strategy: TargetAlbumStrategy;
  target_album_id: string;
  poll_interval_seconds: number;
  created_at: number;
  updated_at: number;
}

export function listRules(): Promise<ApiResult<ListRulesResponse>> {
  return request<ListRulesResponse>("/api/v1/rules", { method: "GET" });
}

export function getRule(id: string): Promise<ApiResult<Rule>> {
  return request<Rule>(`/api/v1/rules/${encodeURIComponent(id)}`, {
    method: "GET",
  });
}

export interface CreateRulePayload {
  yaml_source: string;
}

export function createRule(
  payload: CreateRulePayload | string,
): Promise<ApiResult<RuleSummary>> {
  const body =
    typeof payload === "string" ? { yaml_source: payload } : payload;
  return request<RuleSummary>("/api/v1/rules", {
    method: "POST",
    body: JSON.stringify(body),
  });
}

export interface UpdateRulePayload {
  yaml_source?: string;
  status?: RuleStatus;
}

export function updateRule(
  id: string,
  payload: UpdateRulePayload,
): Promise<ApiResult<RuleSummary>> {
  return request<RuleSummary>(`/api/v1/rules/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
}

export function deleteRule(id: string): Promise<ApiResult<void>> {
  return request<void>(`/api/v1/rules/${encodeURIComponent(id)}`, {
    method: "DELETE",
  });
}

export interface DecisionItem {
  asset_id: string;
  decision: "added" | "skipped";
  reason: string;
  run_id: string | null;
  decided_at: number;
  /// Human filename from the asset index; null when the asset isn't indexed
  /// (deleted / not yet swept) — the UI falls back to a short hash of the id.
  filename: string | null;
}

export interface DecisionsResponse {
  decisions: DecisionItem[];
  total: number;
  limit: number;
  offset: number;
}

export interface FetchDecisionsParams {
  limit?: number;
  offset?: number;
  reasons?: string[];
  /// Verdict filter for the Matched/Skipped chips. Omit for the All chip.
  decision?: "added" | "skipped";
}

export function fetchDecisions(
  ruleId: string,
  params: FetchDecisionsParams = {},
): Promise<ApiResult<DecisionsResponse>> {
  const search = new URLSearchParams();
  if (params.limit !== undefined) search.set("limit", String(params.limit));
  if (params.offset !== undefined) search.set("offset", String(params.offset));
  if (params.reasons && params.reasons.length > 0) {
    search.set("reason", params.reasons.join(","));
  }
  if (params.decision !== undefined) search.set("decision", params.decision);
  const qs = search.toString();
  const path = `/api/v1/rules/${encodeURIComponent(ruleId)}/decisions${
    qs ? `?${qs}` : ""
  }`;
  return request<DecisionsResponse>(path, { method: "GET" });
}

export interface RuleRunItem {
  id: string;
  started_at: number;
  finished_at: number | null;
  assets_evaluated: number;
  assets_added: number;
  assets_skipped: number;
  error_message: string | null;
}

export interface RuleRunsResponse {
  runs: RuleRunItem[];
  total: number;
  limit: number;
  offset: number;
}

export interface FetchRuleRunsParams {
  limit?: number;
  offset?: number;
}

export function fetchRuleRuns(
  ruleId: string,
  params: FetchRuleRunsParams = {},
): Promise<ApiResult<RuleRunsResponse>> {
  const search = new URLSearchParams();
  if (params.limit !== undefined) search.set("limit", String(params.limit));
  if (params.offset !== undefined) search.set("offset", String(params.offset));
  const qs = search.toString();
  const path = `/api/v1/rules/${encodeURIComponent(ruleId)}/runs${
    qs ? `?${qs}` : ""
  }`;
  return request<RuleRunsResponse>(path, { method: "GET" });
}

export interface RuleMatchCount {
  /// Assets in the owner's index that currently satisfy the rule's predicate
  /// tree (exact for cheap-metadata rules; a lower bound for YOLO rules).
  matched: number;
  /// Live asset count of the target album, or `null` when no album is bound
  /// yet or Immich was unreachable. A `matched` ≠ `in_album` gap is a backfill
  /// warning.
  in_album: number | null;
}

export function fetchRuleMatchCount(
  ruleId: string,
): Promise<ApiResult<RuleMatchCount>> {
  return request<RuleMatchCount>(
    `/api/v1/rules/${encodeURIComponent(ruleId)}/match-count`,
    { method: "GET" },
  );
}

export interface MePerson {
  id: string;
  name: string;
  /// Server-relative URL. The browser includes the session cookie
  /// automatically, so no extra wiring is needed at the call site.
  thumbnail_url: string;
}

export interface MeAlbum {
  id: string;
  name: string;
  asset_count: number;
  is_writable: boolean;
}

/**
 * Result of fetching a per-user Immich resource. The `noImmichKey` branch is
 * how the SPA distinguishes "this user hasn't pasted a key yet" (412) from
 * any other failure — the rule builder swaps in a CTA to `/me` instead of
 * the misleading "library is empty" copy.
 */
export type MeFetchResult<T> =
  | { ok: true; data: T }
  | { ok: false; noImmichKey: true }
  | { ok: false; noImmichKey?: false; status: number; error: ApiError };

async function requestMeResource<T>(path: string): Promise<MeFetchResult<T>> {
  const result = await request<T>(path, { method: "GET" });
  if (result.ok) {
    return { ok: true, data: result.data };
  }
  if (result.status === 412 && result.error.error === "no_immich_key") {
    return { ok: false, noImmichKey: true };
  }
  return {
    ok: false,
    noImmichKey: false,
    status: result.status,
    error: result.error,
  };
}

export function fetchPeople(): Promise<MeFetchResult<MePerson[]>> {
  return requestMeResource<MePerson[]>("/api/v1/me/people");
}

export function fetchAlbums(): Promise<MeFetchResult<MeAlbum[]>> {
  return requestMeResource<MeAlbum[]>("/api/v1/me/albums");
}

export interface ImmichKeyInfo {
  base_url: string;
  immich_user_id: string | null;
  last_validated_at: number;
}

export interface PasteImmichKeyPayload {
  base_url: string;
  api_key: string;
}

export function fetchImmichKey(): Promise<ApiResult<ImmichKeyInfo>> {
  return request<ImmichKeyInfo>("/api/v1/me/immich-key", { method: "GET" });
}

export function pasteImmichKey(
  payload: PasteImmichKeyPayload,
): Promise<ApiResult<ImmichKeyInfo>> {
  return request<ImmichKeyInfo>("/api/v1/me/immich-key", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export function deleteImmichKey(): Promise<ApiResult<void>> {
  return request<void>("/api/v1/me/immich-key", { method: "DELETE" });
}

/// One entry in the global live-activity log (T33). A discriminated union on
/// `kind`, mirroring the server's `ActivityKind`. `seq` is a process-monotonic
/// cursor; `at` is unix seconds.
interface ActivityEventBase {
  seq: number;
  at: number;
}
export type ActivityEvent =
  | (ActivityEventBase & {
      kind: "indexed";
      filename: string;
      person_count: number;
      has_gps: boolean;
      taken_at: number | null;
    })
  | (ActivityEventBase & {
      kind: "matched";
      rule_id: string;
      rule_name: string;
      asset_id: string;
      filename: string | null;
    })
  | (ActivityEventBase & {
      kind: "skipped";
      rule_id: string;
      rule_name: string;
      asset_id: string;
      filename: string | null;
      reason: string;
    })
  | (ActivityEventBase & {
      kind: "album_add";
      rule_id: string;
      rule_name: string;
      album_id: string;
      added_count: number;
    })
  | (ActivityEventBase & {
      kind: "sweep_done";
      indexed: number;
      took_ms: number;
    });

export interface ActivityStreamResponse {
  events: ActivityEvent[];
  /// High-water seq the client should send as the next `after`, even when some
  /// events were evicted from the server's ring buffer between polls.
  last_seq: number;
}

/// Poll the caller's live-activity stream for events newer than `after`
/// (0 = the whole retained tail).
export function fetchActivityStream(
  after = 0,
): Promise<ApiResult<ActivityStreamResponse>> {
  const qs = after > 0 ? `?after=${after}` : "";
  return request<ActivityStreamResponse>(`/api/v1/me/activity/stream${qs}`, {
    method: "GET",
  });
}
