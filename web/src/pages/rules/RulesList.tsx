import {
  createResource,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { A, useNavigate } from "@solidjs/router";
import {
  deleteRule,
  fetchRuleRuns,
  listRules,
  updateRule,
  type RuleRunItem,
  type RuleStatus,
  type RuleSummary,
} from "../../lib/api";
import ConfirmDialog from "../../components/ConfirmDialog";
import { Button } from "../../components/ui";
import { humanRuleError } from "./errors";

const RUNS_LIMIT = 1;

interface RuleSummaryWithRun extends RuleSummary {
  last_run: RuleRunItem | null;
}

type PendingAction =
  | { kind: "archive"; rule: RuleSummary }
  | { kind: "delete"; rule: RuleSummary };

const RulesList: Component = () => {
  const navigate = useNavigate();
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<PendingAction | null>(null);

  const [rulesResource, { refetch }] = createResource<
    RuleSummaryWithRun[] | null
  >(async () => {
    setError(null);
    const result = await listRules();
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return null;
      }
      setError(humanRuleError(result.error));
      return [];
    }
    // One extra fetch per rule for its latest run. The fan-out is a handful of
    // rules per operator, so N+1 is acceptable and keeps the list API narrow.
    return Promise.all(
      result.data.rules.map(async (rule): Promise<RuleSummaryWithRun> => {
        const runs = await fetchRuleRuns(rule.id, { limit: RUNS_LIMIT });
        if (!runs.ok) return { ...rule, last_run: null };
        return { ...rule, last_run: runs.data.runs[0] ?? null };
      }),
    );
  });

  const setStatus = async (rule: RuleSummary, status: RuleStatus) => {
    setBusyId(rule.id);
    setError(null);
    const result = await updateRule(rule.id, { status });
    setBusyId(null);
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError(humanRuleError(result.error));
      return;
    }
    await refetch();
  };

  const onTogglePause = (rule: RuleSummary) => {
    const next: RuleStatus = rule.status === "paused" ? "active" : "paused";
    void setStatus(rule, next);
  };

  const onArchiveClick = (rule: RuleSummary) => {
    setPending({ kind: "archive", rule });
  };

  const onDeleteClick = (rule: RuleSummary) => {
    setPending({ kind: "delete", rule });
  };

  const onCancelPending = () => setPending(null);

  const onConfirmPending = async () => {
    const action = pending();
    if (!action) return;
    setPending(null);
    if (action.kind === "archive") {
      await setStatus(action.rule, "archived");
      return;
    }
    const rule = action.rule;
    setBusyId(rule.id);
    setError(null);
    const result = await deleteRule(rule.id);
    setBusyId(null);
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError(humanRuleError(result.error));
      return;
    }
    await refetch();
  };

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-baseline gap-3">
        <h1 class="text-2xl font-semibold tracking-tight">Rules</h1>
        <p class="text-sm text-ui-muted">
          Automation rules that decide which assets land in which albums.
        </p>
        <A
          href="/rules/new"
          class="ml-auto inline-flex items-center justify-center gap-2 rounded-lg bg-immich-primary px-4 py-2 text-sm font-medium text-white shadow-md shadow-ui-primary/20 transition ease-immich duration-150 hover:bg-immich-primary/90 focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary dark:bg-immich-dark-primary dark:text-immich-dark-bg dark:hover:bg-immich-dark-primary/90"
        >
          New rule
        </A>
      </div>

      <Show when={error()}>
        <div
          class="mb-4 rounded-xl border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-700/40 dark:bg-red-900/20 dark:text-red-200"
          role="alert"
        >
          {error()}
        </div>
      </Show>

      <Show
        when={!rulesResource.loading}
        fallback={<p class="text-ui-muted">Loading rules…</p>}
      >
        <Show
          when={(rulesResource() ?? []).length > 0}
          fallback={<EmptyState />}
        >
          <ul class="space-y-3">
            <For each={rulesResource() ?? []}>
              {(rule) => (
                <RuleCard
                  rule={rule}
                  busy={busyId() === rule.id}
                  onTogglePause={() => onTogglePause(rule)}
                  onArchive={() => onArchiveClick(rule)}
                  onDelete={() => onDeleteClick(rule)}
                />
              )}
            </For>
          </ul>
        </Show>
      </Show>

      <ConfirmDialog
        open={pending()?.kind === "archive"}
        title="Archive rule"
        message={`Archive "${pending()?.rule.name ?? ""}"? It will stop running until you reactivate it.`}
        confirmLabel="Archive"
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
      <ConfirmDialog
        open={pending()?.kind === "delete"}
        title="Delete rule"
        message={`Delete "${pending()?.rule.name ?? ""}"? Decisions and run history will be removed. This cannot be undone.`}
        confirmLabel="Delete"
        destructive
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
    </section>
  );
};

interface RuleCardProps {
  rule: RuleSummaryWithRun;
  busy: boolean;
  onTogglePause: () => void;
  onArchive: () => void;
  onDelete: () => void;
}

const RuleCard: Component<RuleCardProps> = (props) => {
  const dimmed = () => props.rule.status === "archived";
  const pauseLabel = () => (props.rule.status === "paused" ? "Resume" : "Pause");
  const strategyLabel = () =>
    props.rule.target_album_strategy === "managed"
      ? "Managed album"
      : "Existing album";

  return (
    <li
      class="rounded-2xl border border-ui-border bg-white p-5 shadow-sm transition ease-immich duration-150 hover:ring-1 hover:ring-immich-primary/30 dark:border-immich-dark-gray dark:bg-immich-dark-gray dark:hover:ring-immich-dark-primary/30"
      classList={{ "opacity-60": dimmed() }}
    >
      <div class="flex items-start justify-between gap-4">
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <StatusDot status={props.rule.status} />
            <A
              href={`/rules/${props.rule.id}`}
              class="truncate text-sm font-semibold text-immich-fg hover:underline dark:text-immich-dark-fg"
            >
              {props.rule.name}
            </A>
            <StatusBadge status={props.rule.status} />
          </div>
          <p class="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-xs text-ui-muted">
            <span>{strategyLabel()}</span>
            <span aria-hidden="true">·</span>
            <MatchCount />
          </p>
          <LastRunSummary run={props.rule.last_run} />
        </div>
        <A
          href={`/rules/${props.rule.id}/activity`}
          class="shrink-0 text-xs font-medium text-immich-primary hover:underline dark:text-immich-dark-primary"
        >
          Activity →
        </A>
      </div>

      <div class="mt-4 flex flex-wrap items-center gap-2">
        <A
          href={`/rules/${props.rule.id}`}
          class="inline-flex items-center justify-center gap-2 rounded-lg bg-slate-200 px-3 py-1.5 text-xs font-medium text-immich-fg transition ease-immich duration-150 hover:bg-slate-300 dark:bg-gray-700 dark:text-immich-dark-fg dark:hover:bg-gray-600"
        >
          Edit
        </A>
        <Show when={props.rule.status !== "archived"}>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={props.busy}
            onClick={() => props.onTogglePause()}
          >
            {pauseLabel()}
          </Button>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={props.busy}
            onClick={() => props.onArchive()}
          >
            Archive
          </Button>
        </Show>
        <Button
          type="button"
          variant="destructive"
          size="sm"
          disabled={props.busy}
          onClick={() => props.onDelete()}
        >
          {props.busy ? "Working…" : "Delete"}
        </Button>
      </div>
    </li>
  );
};

const EmptyState: Component = () => (
  <div class="rounded-2xl border border-dashed border-ui-border bg-white px-6 py-12 text-center dark:border-gray-700 dark:bg-immich-dark-gray">
    <h2 class="text-base font-medium text-immich-fg dark:text-immich-dark-fg">
      No rules yet
    </h2>
    <p class="mt-1 text-sm text-ui-muted">
      Author your first rule to start filing assets into albums.
    </p>
    <A
      href="/rules/new"
      class="mt-4 inline-flex items-center justify-center gap-2 rounded-lg bg-immich-primary px-4 py-2 text-sm font-medium text-white shadow-md shadow-ui-primary/20 hover:bg-immich-primary/90 dark:bg-immich-dark-primary dark:text-immich-dark-bg dark:hover:bg-immich-dark-primary/90"
    >
      Create your first rule
    </A>
  </div>
);

const StatusDot: Component<{ status: RuleStatus }> = (props) => {
  const color = () => {
    switch (props.status) {
      case "active":
        return "bg-emerald-500";
      case "paused":
        return "bg-amber-500";
      case "archived":
      default:
        return "bg-slate-400 dark:bg-gray-500";
    }
  };
  return (
    <span
      class={`inline-block h-2 w-2 shrink-0 rounded-full ${color()}`}
      aria-hidden="true"
    />
  );
};

const StatusBadge: Component<{ status: RuleStatus }> = (props) => {
  const styles = () => {
    switch (props.status) {
      case "active":
        return "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30";
      case "paused":
        return "bg-amber-100 text-amber-800 ring-amber-200 dark:bg-amber-500/20 dark:text-amber-200 dark:ring-amber-500/30";
      case "archived":
        return "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600";
    }
  };
  return (
    <span
      class={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${styles()}`}
    >
      {props.status}
    </span>
  );
};

// Placeholder for the per-rule match count. POSTSHIP-T36 fills in the real
// "N matched · N in album" figures from the index + Immich album.
const MatchCount: Component = () => (
  <span data-testid="rule-match-count" title="Match count lands in a coming update">
    — matched
  </span>
);

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

export default RulesList;
