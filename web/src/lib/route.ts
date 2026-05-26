export interface SetupStateForRoute {
  needs_setup: boolean;
}

export interface MeForRoute {
  authed: boolean;
}

export type InitialRoute = "/setup" | "/login" | "/";

/**
 * Pure routing decision for the bootstrap fetch.
 *
 * - `needs_setup=true` always wins → `/setup` (server gate is authoritative).
 * - else `authed=true` → `/`.
 * - else → `/login`.
 */
export function decideInitialRoute(
  state: SetupStateForRoute,
  me: MeForRoute,
): InitialRoute {
  if (state.needs_setup) {
    return "/setup";
  }
  if (me.authed) {
    return "/";
  }
  return "/login";
}
