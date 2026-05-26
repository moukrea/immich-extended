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
  type RuleStatus,
  type RuleSummary,
} from "../../lib/api";
import { humanRuleError } from "./errors";

const RulesList: Component = () => {
  const navigate = useNavigate();
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<string | null>(null);

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

  const onDelete = async (rule: RuleSummary) => {
    const confirmed = window.confirm(
      `Delete rule "${rule.name}"? This cannot be undone.`,
    );
    if (!confirmed) return;
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
                {(rule) => (
                  <li class="flex items-center justify-between gap-4 px-4 py-3">
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
                      <button
                        type="button"
                        disabled={busyId() === rule.id}
                        onClick={() => onDelete(rule)}
                        class="rounded-md border border-red-300 bg-white px-2.5 py-1 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-60"
                      >
                        {busyId() === rule.id ? "Deleting…" : "Delete"}
                      </button>
                    </div>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </Show>
      </section>
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
