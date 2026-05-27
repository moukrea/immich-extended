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
  listRules,
  updateRule,
  type RuleStatus,
  type RuleSummary,
} from "../../lib/api";
import ConfirmDialog from "../../components/ConfirmDialog";
import { Button } from "../../components/ui";
import { humanRuleError } from "./errors";

type PendingAction =
  | { kind: "archive"; rule: RuleSummary }
  | { kind: "delete"; rule: RuleSummary };

const RulesList: Component = () => {
  const navigate = useNavigate();
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal<PendingAction | null>(null);

  const [rulesResource, { refetch }] = createResource<RuleSummary[] | null>(
    async () => {
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
      return result.data.rules;
    },
  );

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
          <div class="rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
            <ul class="divide-y divide-ui-border dark:divide-gray-700">
              <For each={rulesResource() ?? []}>
                {(rule) => {
                  const isBusy = () => busyId() === rule.id;
                  const dimmed = () => rule.status === "archived";
                  const pauseLabel = () =>
                    rule.status === "paused" ? "Resume" : "Pause";
                  return (
                    <li
                      class="flex items-center justify-between gap-4 px-5 py-4"
                      classList={{ "opacity-60": dimmed() }}
                    >
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-sm font-medium text-immich-fg dark:text-immich-dark-fg">
                          {rule.name}
                        </p>
                        <p class="mt-0.5 text-xs text-ui-muted">
                          {rule.target_album_strategy === "managed"
                            ? "Managed album"
                            : "Existing album"}{" "}
                          · updated {formatTimestamp(rule.updated_at)}
                        </p>
                      </div>
                      <StatusBadge status={rule.status} />
                      <div class="flex items-center gap-2">
                        <A
                          href={`/rules/${rule.id}`}
                          class="inline-flex items-center justify-center gap-2 rounded-lg bg-slate-200 px-3 py-1.5 text-xs font-medium text-immich-fg transition ease-immich duration-150 hover:bg-slate-300 dark:bg-gray-700 dark:text-immich-dark-fg dark:hover:bg-gray-600"
                        >
                          Edit
                        </A>
                        <Show when={rule.status !== "archived"}>
                          <Button
                            type="button"
                            variant="secondary"
                            size="sm"
                            disabled={isBusy()}
                            onClick={() => onTogglePause(rule)}
                          >
                            {pauseLabel()}
                          </Button>
                          <Button
                            type="button"
                            variant="secondary"
                            size="sm"
                            disabled={isBusy()}
                            onClick={() => onArchiveClick(rule)}
                          >
                            Archive
                          </Button>
                        </Show>
                        <Button
                          type="button"
                          variant="destructive"
                          size="sm"
                          disabled={isBusy()}
                          onClick={() => onDeleteClick(rule)}
                        >
                          {isBusy() ? "Working…" : "Delete"}
                        </Button>
                      </div>
                    </li>
                  );
                }}
              </For>
            </ul>
          </div>
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

const EmptyState: Component = () => (
  <div class="rounded-2xl border border-dashed border-ui-border bg-white px-6 py-12 text-center dark:border-gray-700 dark:bg-immich-dark-gray">
    <h2 class="text-base font-medium text-immich-fg dark:text-immich-dark-fg">
      No rules yet
    </h2>
    <p class="mt-1 text-sm text-ui-muted">
      Author your first rule by pasting YAML.
    </p>
    <A
      href="/rules/new"
      class="mt-4 inline-flex items-center justify-center gap-2 rounded-lg bg-immich-primary px-4 py-2 text-sm font-medium text-white shadow-md shadow-ui-primary/20 hover:bg-immich-primary/90 dark:bg-immich-dark-primary dark:text-immich-dark-bg dark:hover:bg-immich-dark-primary/90"
    >
      Create your first rule
    </A>
  </div>
);

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

function formatTimestamp(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "—";
  const date = new Date(seconds * 1000);
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default RulesList;
