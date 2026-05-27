import {
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { A, useNavigate } from "@solidjs/router";
import {
  fetchRuleRuns,
  getMe,
  listRules,
  type Me,
  type RuleRunItem,
  type RuleSummary,
} from "../lib/api";
import { useLivePoll } from "../lib/livePoll";

const POLL_INTERVAL_MS = 5000;
const RULES_RUNS_LIMIT = 1;

interface RuleSummaryWithRun extends RuleSummary {
  last_run: RuleRunItem | null;
}

const Dashboard: Component = () => {
  const navigate = useNavigate();
  const [me, setMe] = createSignal<Me | null>(null);
  const [rules, setRules] = createSignal<RuleSummaryWithRun[] | null>(null);
  const [error, setError] = createSignal<string | null>(null);

  const refresh = async () => {
    // Identity rarely changes; bootstrap once.
    if (me() === null) {
      const meRes = await getMe();
      if (!meRes.ok) {
        if (meRes.status === 401) {
          navigate("/login", { replace: true });
          return;
        }
      } else {
        setMe(meRes.data);
      }
    }

    const rulesRes = await listRules();
    if (!rulesRes.ok) {
      if (rulesRes.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError("Could not load rules.");
      return;
    }
    setError(null);
    const rows = rulesRes.data.rules;

    // Fetch the latest run for each rule in parallel. N+1 is fine for the
    // expected fan-out (a handful of rules per operator) and keeps the API
    // surface narrow.
    const enriched = await Promise.all(
      rows.map(async (rule): Promise<RuleSummaryWithRun> => {
        const runs = await fetchRuleRuns(rule.id, { limit: RULES_RUNS_LIMIT });
        if (!runs.ok) {
          return { ...rule, last_run: null };
        }
        return { ...rule, last_run: runs.data.runs[0] ?? null };
      }),
    );
    setRules(enriched);
  };

  useLivePoll({ intervalMs: POLL_INTERVAL_MS, fetcher: refresh });

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-baseline gap-3">
        <h1 class="text-2xl font-semibold tracking-tight">Overview</h1>
        <Show when={me()}>
          {(user) => (
            <p class="text-sm text-ui-muted">
              Signed in as{" "}
              <span class="font-medium text-immich-fg dark:text-immich-dark-fg">
                {user().email}
              </span>
              <Show when={user().display_name}>
                {(name) => (
                  <span class="text-ui-muted"> ({name()})</span>
                )}
              </Show>
            </p>
          )}
        </Show>
        <span
          class="ml-auto inline-flex items-center gap-1.5 text-xs text-ui-muted"
          aria-live="polite"
        >
          <span class="relative flex h-2 w-2" aria-hidden="true">
            <span class="absolute inline-flex h-full w-full animate-ping rounded-full bg-immich-primary opacity-60 dark:bg-immich-dark-primary" />
            <span class="relative inline-flex h-2 w-2 rounded-full bg-immich-primary dark:bg-immich-dark-primary" />
          </span>
          Live — refreshing every {POLL_INTERVAL_MS / 1000}s
        </span>
      </div>

      <Show when={error()}>
        <div
          class="mb-4 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-700/40 dark:bg-red-900/20 dark:text-red-200"
          role="alert"
        >
          {error()}
        </div>
      </Show>

      <div class="rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
        <header class="flex items-center justify-between border-b border-ui-border px-5 py-3 dark:border-gray-700">
          <h2 class="text-base font-semibold">Rules</h2>
          <A
            href="/rules/new"
            class="text-sm font-medium text-immich-primary hover:underline dark:text-immich-dark-primary"
          >
            + New rule
          </A>
        </header>

        <Show
          when={rules() !== null}
          fallback={
            <p class="px-5 py-6 text-sm text-ui-muted">Loading rules…</p>
          }
        >
          <Show
            when={(rules() ?? []).length > 0}
            fallback={
              <p class="px-5 py-6 text-sm text-ui-muted">
                No rules yet.{" "}
                <A
                  href="/rules/new"
                  class="text-immich-primary hover:underline dark:text-immich-dark-primary"
                >
                  Create one
                </A>{" "}
                to start filing assets into albums.
              </p>
            }
          >
            <ul class="divide-y divide-ui-border dark:divide-gray-700">
              <For each={rules() ?? []}>
                {(rule) => <RuleCard rule={rule} />}
              </For>
            </ul>
          </Show>
        </Show>
      </div>
    </section>
  );
};

const RuleCard: Component<{ rule: RuleSummaryWithRun }> = (props) => {
  const status = () => props.rule.status;
  const statusClass = () => {
    switch (status()) {
      case "active":
        return "bg-emerald-100 text-emerald-800 dark:bg-emerald-500/20 dark:text-emerald-200";
      case "paused":
        return "bg-amber-100 text-amber-800 dark:bg-amber-500/20 dark:text-amber-200";
      case "archived":
      default:
        return "bg-slate-100 text-slate-700 dark:bg-gray-700 dark:text-gray-200";
    }
  };

  return (
    <li>
      <A
        href={`/rules/${props.rule.id}`}
        class="flex items-center gap-4 px-5 py-4 transition-colors hover:bg-slate-50 dark:hover:bg-gray-800/40"
      >
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate text-sm font-medium text-immich-fg dark:text-immich-dark-fg">
              {props.rule.name}
            </span>
            <span
              class={`inline-flex items-center rounded-full px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide ${statusClass()}`}
            >
              {status()}
            </span>
          </div>
          <LastRunSummary run={props.rule.last_run} />
        </div>
        <A
          href={`/rules/${props.rule.id}/activity`}
          class="hidden text-xs font-medium text-immich-primary hover:underline dark:text-immich-dark-primary sm:inline"
        >
          Activity →
        </A>
      </A>
    </li>
  );
};

const LastRunSummary: Component<{ run: RuleRunItem | null }> = (props) => {
  return (
    <Show
      when={props.run !== null}
      fallback={
        <p class="mt-1 text-xs text-ui-muted">
          No runs yet — waiting for first cycle.
        </p>
      }
    >
      {(_) => {
        const run = () => props.run as RuleRunItem;
        const finished = () => run().finished_at !== null;
        return (
          <p class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-ui-muted">
            <span>
              Last run{" "}
              <span class="text-immich-fg dark:text-immich-dark-fg">
                {formatRelative(run().started_at)}
              </span>
            </span>
            <Show
              when={finished()}
              fallback={
                <span class="inline-flex items-center gap-1 text-immich-primary dark:text-immich-dark-primary">
                  <span
                    class="inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-current"
                    aria-hidden="true"
                  />
                  running…
                </span>
              }
            >
              <Show
                when={run().error_message}
                fallback={
                  <span>
                    <span class="text-emerald-700 dark:text-emerald-300">
                      +{run().assets_added}
                    </span>
                    <span class="ml-1">added</span>
                    <span class="ml-2">{run().assets_skipped} skipped</span>
                  </span>
                }
              >
                {(msg) => (
                  <span
                    class="truncate text-ui-danger"
                    title={msg()}
                    data-testid="rule-last-run-error"
                  >
                    error: {msg()}
                  </span>
                )}
              </Show>
            </Show>
          </p>
        );
      }}
    </Show>
  );
};

function formatRelative(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "—";
  const now = Date.now() / 1000;
  const delta = Math.max(0, now - seconds);
  if (delta < 60) return `${Math.round(delta)}s ago`;
  if (delta < 3600) return `${Math.round(delta / 60)}m ago`;
  if (delta < 86_400) return `${Math.round(delta / 3600)}h ago`;
  return new Date(seconds * 1000).toLocaleDateString();
}

export default Dashboard;
