import {
  createMemo,
  createResource,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import {
  fetchDecisions,
  type DecisionItem,
  type DecisionsResponse,
} from "../../lib/api";
import { DECISION_REASONS, reasonLabel } from "../../lib/decisionReasons";
import { Button } from "../../components/ui";
import { humanRuleError } from "./errors";

const PAGE_SIZE = 25;

const RuleDecisions: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id: string }>();
  const [error, setError] = createSignal<string | null>(null);
  const [page, setPage] = createSignal(1);
  const [reasons, setReasons] = createSignal<string[]>([]);

  // SolidJS resources refetch whenever any signal read in the source closure
  // changes; reading id / page / reasons here makes pagination + filter live.
  const [data] = createResource<DecisionsResponse | null, {
    id: string;
    page: number;
    reasons: string[];
  }>(
    () => ({ id: params.id, page: page(), reasons: reasons() }),
    async ({ id, page: p, reasons: rs }) => {
      setError(null);
      const result = await fetchDecisions(id, {
        limit: PAGE_SIZE,
        offset: (p - 1) * PAGE_SIZE,
        reasons: rs,
      });
      if (!result.ok) {
        if (result.status === 401) {
          navigate("/login", { replace: true });
          return null;
        }
        if (result.status === 404) {
          setError("Rule not found.");
        } else {
          setError(humanRuleError(result.error));
        }
        return null;
      }
      return result.data;
    },
  );

  const totalPages = createMemo(() => {
    const d = data();
    if (!d || d.total <= 0) return 1;
    return Math.max(1, Math.ceil(d.total / PAGE_SIZE));
  });

  const onPrev = () => {
    setPage((p) => Math.max(1, p - 1));
  };
  const onNext = () => {
    setPage((p) => Math.min(totalPages(), p + 1));
  };

  const onToggleReason = (slug: string, checked: boolean) => {
    setPage(1);
    setReasons((prev) => {
      const set = new Set(prev);
      if (checked) set.add(slug);
      else set.delete(slug);
      // Preserve canonical ordering (matches DECISION_REASONS) so URLs are
      // stable regardless of click sequence.
      return DECISION_REASONS.filter((r) => set.has(r));
    });
  };

  const onClearReasons = () => {
    setPage(1);
    setReasons([]);
  };

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-center gap-3">
        <A
          href={`/rules/${params.id}`}
          class="text-sm text-ui-muted hover:text-immich-fg dark:hover:text-immich-dark-fg"
        >
          ← Rule
        </A>
        <h1 class="text-2xl font-semibold tracking-tight">Decisions</h1>
      </div>

      <Show when={error()}>
        <div
          class="mb-4 rounded-xl border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 dark:border-red-700/40 dark:bg-red-900/20 dark:text-red-200"
          role="alert"
        >
          {error()}
        </div>
      </Show>

      <fieldset class="mb-4 rounded-2xl border border-ui-border bg-white p-4 shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
        <legend class="px-1 text-xs font-medium uppercase tracking-wide text-ui-muted">
          Filter by reason
        </legend>
        <div class="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm">
          <For each={DECISION_REASONS}>
            {(slug) => (
              <label class="inline-flex items-center gap-1.5 text-immich-fg dark:text-immich-dark-fg">
                <input
                  type="checkbox"
                  class="rounded border-ui-border text-immich-primary focus:ring-immich-primary dark:border-gray-600 dark:bg-gray-700 dark:text-immich-dark-primary"
                  checked={reasons().includes(slug)}
                  onChange={(e) =>
                    onToggleReason(slug, e.currentTarget.checked)
                  }
                />
                <span>{reasonLabel(slug)}</span>
              </label>
            )}
          </For>
          <Show when={reasons().length > 0}>
            <button
              type="button"
              onClick={onClearReasons}
              class="ml-auto text-xs text-ui-muted underline hover:text-immich-fg dark:hover:text-immich-dark-fg"
            >
              Clear
            </button>
          </Show>
        </div>
      </fieldset>

      <Show
        when={!data.loading}
        fallback={<p class="text-ui-muted">Loading decisions…</p>}
      >
        <Show when={data()}>
          {(d) => (
            <Show
              when={d().decisions.length > 0}
              fallback={
                <p class="text-ui-muted">
                  {reasons().length > 0
                    ? "No decisions match the current filter."
                    : "No decisions recorded yet. They will appear here after the rule's next poll cycle."}
                </p>
              }
            >
              <p class="mb-3 text-sm text-ui-muted">
                Showing {d().decisions.length} of {d().total} decisions
                {reasons().length > 0 ? " (filtered)" : ""}.
              </p>
              <div class="overflow-hidden rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
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
                    <For each={d().decisions}>
                      {(row) => <DecisionRow row={row} />}
                    </For>
                  </tbody>
                </table>
              </div>
              <nav
                class="mt-4 flex items-center justify-between text-sm"
                aria-label="Decisions pagination"
              >
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={onPrev}
                  disabled={page() <= 1}
                >
                  ← Previous
                </Button>
                <span class="text-ui-muted" aria-live="polite">
                  Page {page()} of {totalPages()}
                </span>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={onNext}
                  disabled={page() >= totalPages()}
                >
                  Next →
                </Button>
              </nav>
            </Show>
          )}
        </Show>
      </Show>
    </section>
  );
};

const DecisionRow: Component<{ row: DecisionItem }> = (props) => {
  const decisionClass = () =>
    props.row.decision === "added"
      ? "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30"
      : "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600";

  return (
    <tr>
      <td class="whitespace-nowrap px-4 py-2 font-mono text-xs text-immich-fg dark:text-immich-dark-fg">
        {shortHash(props.row.asset_id)}
      </td>
      <td class="px-4 py-2">
        <span
          class={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${decisionClass()}`}
        >
          {props.row.decision}
        </span>
      </td>
      <td class="px-4 py-2 text-immich-fg dark:text-immich-dark-fg">
        {props.row.reason}
      </td>
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
  const date = new Date(seconds * 1000);
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export default RuleDecisions;
