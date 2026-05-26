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
  postLogout,
  updateRule,
  type RuleStatus,
  type RuleSummary,
} from "../../lib/api";
import ConfirmDialog from "../../components/ConfirmDialog";
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

  const onLogout = async () => {
    await postLogout();
    navigate("/login", { replace: true });
  };

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
    <main class="min-h-screen bg-slate-50">
      <header class="bg-white border-b border-slate-200">
        <div class="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <div class="flex items-center gap-4">
            <A
              href="/"
              class="text-sm text-slate-500 hover:text-slate-700"
              aria-label="Back to dashboard"
            >
              ← Dashboard
            </A>
            <h1 class="text-lg font-semibold text-slate-900">Rules</h1>
          </div>
          <button
            type="button"
            onClick={onLogout}
            class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
          >
            Sign out
          </button>
        </div>
      </header>

      <section class="max-w-5xl mx-auto px-4 py-8">
        <div class="flex items-center justify-between mb-4">
          <p class="text-slate-600 text-sm">
            Automation rules that decide which assets land in which albums.
          </p>
          <A
            href="/rules/new"
            class="rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-medium text-white shadow hover:bg-indigo-500"
          >
            New rule
          </A>
        </div>

        <Show when={error()}>
          <div
            class="mb-4 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700"
            role="alert"
          >
            {error()}
          </div>
        </Show>

        <Show
          when={!rulesResource.loading}
          fallback={<p class="text-slate-500">Loading rules…</p>}
        >
          <Show
            when={(rulesResource() ?? []).length > 0}
            fallback={<EmptyState />}
          >
            <ul class="divide-y divide-slate-200 rounded-md border border-slate-200 bg-white shadow-sm">
              <For each={rulesResource() ?? []}>
                {(rule) => {
                  const isBusy = () => busyId() === rule.id;
                  const dimmed = () => rule.status === "archived";
                  const pauseLabel = () =>
                    rule.status === "paused" ? "Resume" : "Pause";
                  return (
                    <li
                      class="flex items-center justify-between gap-4 px-4 py-3"
                      classList={{ "opacity-60": dimmed() }}
                    >
                      <div class="min-w-0 flex-1">
                        <p class="truncate text-sm font-medium text-slate-900">
                          {rule.name}
                        </p>
                        <p class="mt-0.5 text-xs text-slate-500">
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
                          class="rounded-md border border-slate-300 bg-white px-2.5 py-1 text-xs font-medium text-slate-700 hover:bg-slate-100"
                        >
                          Edit
                        </A>
                        <Show when={rule.status !== "archived"}>
                          <button
                            type="button"
                            disabled={isBusy()}
                            onClick={() => onTogglePause(rule)}
                            class="rounded-md border border-slate-300 bg-white px-2.5 py-1 text-xs font-medium text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                          >
                            {pauseLabel()}
                          </button>
                          <button
                            type="button"
                            disabled={isBusy()}
                            onClick={() => onArchiveClick(rule)}
                            class="rounded-md border border-slate-300 bg-white px-2.5 py-1 text-xs font-medium text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                          >
                            Archive
                          </button>
                        </Show>
                        <button
                          type="button"
                          disabled={isBusy()}
                          onClick={() => onDeleteClick(rule)}
                          class="rounded-md border border-red-300 bg-white px-2.5 py-1 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-60"
                        >
                          {isBusy() ? "Working…" : "Delete"}
                        </button>
                      </div>
                    </li>
                  );
                }}
              </For>
            </ul>
          </Show>
        </Show>
      </section>

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
    </main>
  );
};

const EmptyState: Component = () => (
  <div class="rounded-md border border-dashed border-slate-300 bg-white px-6 py-12 text-center">
    <h2 class="text-base font-medium text-slate-900">No rules yet</h2>
    <p class="mt-1 text-sm text-slate-500">
      Author your first rule by pasting YAML.
    </p>
    <A
      href="/rules/new"
      class="mt-4 inline-block rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white shadow hover:bg-indigo-500"
    >
      Create your first rule
    </A>
  </div>
);

const StatusBadge: Component<{ status: RuleStatus }> = (props) => {
  const styles = () => {
    switch (props.status) {
      case "active":
        return "bg-green-100 text-green-800 ring-green-200";
      case "paused":
        return "bg-amber-100 text-amber-800 ring-amber-200";
      case "archived":
        return "bg-slate-100 text-slate-700 ring-slate-200";
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
