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

export function createRule(
  yaml_source: string,
): Promise<ApiResult<RuleSummary>> {
  return request<RuleSummary>("/api/v1/rules", {
    method: "POST",
    body: JSON.stringify({ yaml_source }),
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
