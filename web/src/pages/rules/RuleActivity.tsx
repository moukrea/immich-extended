import {
  createMemo,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import {
  fetchDecisions,
  fetchRuleRuns,
  type DecisionItem,
  type RuleRunItem,
} from "../../lib/api";
import { useLivePoll } from "../../lib/livePoll";
import { reasonLabel } from "../../lib/decisionReasons";

const RUNS_LIMIT = 20;
const DECISIONS_LIMIT = 50;
const POLL_INTERVAL_MS = 5000;

const RuleActivity: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id: string }>();
  const [runs, setRuns] = createSignal<RuleRunItem[] | null>(null);
  const [runsError, setRunsError] = createSignal<string | null>(null);
  const [decisions, setDecisions] = createSignal<DecisionItem[] | null>(null);
  const [decisionsTotal, setDecisionsTotal] = createSignal<number>(0);
  const [decisionsError, setDecisionsError] = createSignal<string | null>(null);

  const refresh = async () => {
    const [runsRes, decisionsRes] = await Promise.all([
      fetchRuleRuns(params.id, { limit: RUNS_LIMIT, offset: 0 }),
      fetchDecisions(params.id, { limit: DECISIONS_LIMIT, offset: 0 }),
    ]);

    if (runsRes.ok) {
      setRuns(runsRes.data.runs);
      setRunsError(null);
    } else if (runsRes.status === 401) {
      navigate("/login", { replace: true });
      return;
    } else if (runsRes.status === 404) {
      setRunsError("Rule not found.");
    } else {
      setRunsError("Could not load recent runs.");
    }

    if (decisionsRes.ok) {
      setDecisions(decisionsRes.data.decisions);
      setDecisionsTotal(decisionsRes.data.total);
      setDecisionsError(null);
    } else if (decisionsRes.status === 401) {
      navigate("/login", { replace: true });
      return;
    } else if (decisionsRes.status === 404) {
      setDecisionsError("Rule not found.");
    } else {
      setDecisionsError("Could not load recent decisions.");
    }
  };

  useLivePoll({ intervalMs: POLL_INTERVAL_MS, fetcher: refresh });

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-center gap-3">
        <A
          href={`/rules/${params.id}`}
          class="text-sm text-ui-muted hover:text-immich-fg dark:hover:text-immich-dark-fg"
        >
          ← Rule
        </A>
        <h1 class="text-2xl font-semibold tracking-tight">Activity</h1>
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

      <RunsPanel runs={runs()} error={runsError()} />
      <DecisionsPanel
        decisions={decisions()}
        total={decisionsTotal()}
        error={decisionsError()}
      />
    </section>
  );
};

const RunsPanel: Component<{
  runs: RuleRunItem[] | null;
  error: string | null;
}> = (props) => {
  return (
    <div class="mb-8 rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
      <header class="border-b border-ui-border px-5 py-3 dark:border-gray-700">
        <h2 class="text-base font-semibold">Recent runs</h2>
        <p class="mt-0.5 text-xs text-ui-muted">
          Last {RUNS_LIMIT} cycles, newest first.
        </p>
      </header>
      <Show
        when={props.error === null}
        fallback={
          <p class="px-5 py-6 text-sm text-ui-danger" role="alert">
            {props.error}
          </p>
        }
      >
        <Show
          when={props.runs !== null}
          fallback={
            <p class="px-5 py-6 text-sm text-ui-muted">Loading runs…</p>
          }
        >
          <Show
            when={(props.runs ?? []).length > 0}
            fallback={
              <p class="px-5 py-6 text-sm text-ui-muted">
                No runs yet. The first row will appear after the rule's next
                poll cycle.
              </p>
            }
          >
            <div class="overflow-x-auto">
              <table class="min-w-full divide-y divide-ui-border text-sm dark:divide-gray-700">
                <thead class="bg-slate-50 dark:bg-gray-800/40">
                  <tr>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Started
                    </th>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Duration
                    </th>
                    <th class="px-4 py-2 text-right font-medium text-ui-muted">
                      Evaluated
                    </th>
                    <th class="px-4 py-2 text-right font-medium text-ui-muted">
                      Added
                    </th>
                    <th class="px-4 py-2 text-right font-medium text-ui-muted">
                      Skipped
                    </th>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Status
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-ui-border dark:divide-gray-800">
                  <For each={props.runs ?? []}>
                    {(row) => <RunRow row={row} />}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
};

const RunRow: Component<{ row: RuleRunItem }> = (props) => {
  const duration = createMemo(() => {
    if (props.row.finished_at === null) return null;
    const ms = (props.row.finished_at - props.row.started_at) * 1000;
    return ms;
  });
  const hasError = createMemo(() => props.row.error_message !== null);
  return (
    <tr
      class={
        hasError()
          ? "bg-red-50 text-red-900 dark:bg-red-900/20 dark:text-red-200"
          : ""
      }
      data-testid={hasError() ? "run-row-error" : "run-row"}
    >
      <td class="whitespace-nowrap px-4 py-2 text-immich-fg dark:text-immich-dark-fg">
        {formatTimestamp(props.row.started_at)}
      </td>
      <td class="whitespace-nowrap px-4 py-2 text-ui-muted">
        {props.row.finished_at === null
          ? "running…"
          : formatDuration(duration() ?? 0)}
      </td>
      <td class="whitespace-nowrap px-4 py-2 text-right tabular-nums">
        {props.row.assets_evaluated}
      </td>
      <td class="whitespace-nowrap px-4 py-2 text-right tabular-nums">
        {props.row.assets_added}
      </td>
      <td class="whitespace-nowrap px-4 py-2 text-right tabular-nums">
        {props.row.assets_skipped}
      </td>
      <td class="px-4 py-2">
        <Show
          when={props.row.error_message}
          fallback={
            <span class="inline-flex items-center rounded-full bg-emerald-100 px-2 py-0.5 text-xs font-medium text-emerald-800 dark:bg-emerald-500/20 dark:text-emerald-200">
              ok
            </span>
          }
        >
          {(msg) => (
            <span
              class="inline-block max-w-xs truncate align-middle text-xs"
              title={msg()}
            >
              {msg()}
            </span>
          )}
        </Show>
      </td>
    </tr>
  );
};

const DecisionsPanel: Component<{
  decisions: DecisionItem[] | null;
  total: number;
  error: string | null;
}> = (props) => {
  return (
    <div class="rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
      <header class="border-b border-ui-border px-5 py-3 dark:border-gray-700">
        <h2 class="text-base font-semibold">Recent decisions</h2>
        <p class="mt-0.5 text-xs text-ui-muted">
          Last {DECISIONS_LIMIT} decisions, newest first.
          <Show when={props.total > 0}>
            {" "}
            <span>{props.total} total</span>
          </Show>
        </p>
      </header>
      <Show
        when={props.error === null}
        fallback={
          <p class="px-5 py-6 text-sm text-ui-danger" role="alert">
            {props.error}
          </p>
        }
      >
        <Show
          when={props.decisions !== null}
          fallback={
            <p class="px-5 py-6 text-sm text-ui-muted">Loading decisions…</p>
          }
        >
          <Show
            when={(props.decisions ?? []).length > 0}
            fallback={
              <p class="px-5 py-6 text-sm text-ui-muted">
                No decisions yet. They'll appear here after the rule's next
                poll cycle.
              </p>
            }
          >
            <div class="overflow-x-auto">
              <table class="min-w-full divide-y divide-ui-border text-sm dark:divide-gray-700">
                <thead class="bg-slate-50 dark:bg-gray-800/40">
                  <tr>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Asset
                    </th>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Decision
                    </th>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Reason
                    </th>
                    <th class="px-4 py-2 text-left font-medium text-ui-muted">
                      Decided
                    </th>
                  </tr>
                </thead>
                <tbody class="divide-y divide-ui-border dark:divide-gray-800">
                  <For each={props.decisions ?? []}>
                    {(row) => <DecisionRow row={row} />}
                  </For>
                </tbody>
              </table>
            </div>
          </Show>
        </Show>
      </Show>
    </div>
  );
};

const DecisionRow: Component<{ row: DecisionItem }> = (props) => {
  const decisionClass = () =>
    props.row.decision === "added"
      ? "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30"
      : "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600";
  return (
    <tr>
      <td class="whitespace-nowrap px-4 py-2 font-mono text-xs">
        {shortHash(props.row.asset_id)}
      </td>
      <td class="px-4 py-2">
        <span
          class={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${decisionClass()}`}
        >
          {props.row.decision}
        </span>
      </td>
      <td class="px-4 py-2">{reasonLabel(props.row.reason)}</td>
      <td class="whitespace-nowrap px-4 py-2 text-ui-muted">
        {formatTimestamp(props.row.decided_at)}
      </td>
    </tr>
  );
};

function shortHash(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 6)}…${id.slice(-4)}`;
}

function formatTimestamp(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "—";
  return new Date(seconds * 1000).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes}m ${seconds}s`;
}

export default RuleActivity;
