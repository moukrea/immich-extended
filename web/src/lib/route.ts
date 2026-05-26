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

/**
 * Path-aware bootstrap nav: returns the path to redirect to, or `null` to stay
 * put. Lets authenticated users deep-link to inner pages (e.g. `/rules`) on
 * page load instead of being bounced to `/`.
 */
export function decideBootstrapNavigation(
  state: SetupStateForRoute,
  me: MeForRoute,
  currentPath: string,
): string | null {
  if (state.needs_setup) {
    return currentPath === "/setup" ? null : "/setup";
  }
  if (!me.authed) {
    return currentPath === "/login" ? null : "/login";
  }
  if (currentPath === "/login" || currentPath === "/setup") {
    return "/";
  }
  return null;
}
