import {
  createResource,
  createSignal,
  For,
  Show,
  type Component,
} from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import {
  fetchDecisions,
  postLogout,
  type DecisionItem,
  type DecisionsResponse,
} from "../../lib/api";
import { humanRuleError } from "./errors";

const RuleDecisions: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id: string }>();
  const [error, setError] = createSignal<string | null>(null);

  const [data] = createResource<DecisionsResponse | null, string>(
    () => params.id,
    async (id) => {
      setError(null);
      const result = await fetchDecisions(id, { limit: 25, offset: 0 });
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

  const onLogout = async () => {
    await postLogout();
    navigate("/login", { replace: true });
  };

  return (
    <main class="min-h-screen bg-slate-50">
      <header class="bg-white border-b border-slate-200">
        <div class="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <div class="flex items-center gap-4">
            <A
              href={`/rules/${params.id}`}
              class="text-sm text-slate-500 hover:text-slate-700"
            >
              ← Rule
            </A>
            <h1 class="text-lg font-semibold text-slate-900">Decisions</h1>
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
        <Show when={error()}>
          <div
            class="mb-4 rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700"
            role="alert"
          >
            {error()}
          </div>
        </Show>

        <Show
          when={!data.loading}
          fallback={<p class="text-slate-500">Loading decisions…</p>}
        >
          <Show when={data()}>
            {(d) => (
              <Show
                when={d().decisions.length > 0}
                fallback={
                  <p class="text-slate-500">
                    No decisions recorded yet. They will appear here after
                    the rule's next poll cycle.
                  </p>
                }
              >
                <p class="mb-3 text-sm text-slate-600">
                  Showing {d().decisions.length} of {d().total} decisions.
                </p>
                <div class="overflow-hidden rounded-md border border-slate-200 bg-white shadow-sm">
                  <table class="min-w-full divide-y divide-slate-200 text-sm">
                    <thead class="bg-slate-50">
                      <tr>
                        <th class="px-4 py-2 text-left font-medium text-slate-600">
                          Asset
                        </th>
                        <th class="px-4 py-2 text-left font-medium text-slate-600">
                          Decision
                        </th>
                        <th class="px-4 py-2 text-left font-medium text-slate-600">
                          Reason
                        </th>
                        <th class="px-4 py-2 text-left font-medium text-slate-600">
                          Decided
                        </th>
                      </tr>
                    </thead>
                    <tbody class="divide-y divide-slate-100">
                      <For each={d().decisions}>
                        {(row) => <DecisionRow row={row} />}
                      </For>
                    </tbody>
                  </table>
                </div>
              </Show>
            )}
          </Show>
        </Show>
      </section>
    </main>
  );
};

const DecisionRow: Component<{ row: DecisionItem }> = (props) => {
  const decisionClass = () =>
    props.row.decision === "added"
      ? "bg-green-100 text-green-800 ring-green-200"
      : "bg-slate-100 text-slate-700 ring-slate-200";

  return (
    <tr>
      <td class="whitespace-nowrap px-4 py-2 font-mono text-xs text-slate-700">
        {shortHash(props.row.asset_id)}
      </td>
      <td class="px-4 py-2">
        <span
          class={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${decisionClass()}`}
        >
          {props.row.decision}
        </span>
      </td>
      <td class="px-4 py-2 text-slate-700">{props.row.reason}</td>
      <td class="whitespace-nowrap px-4 py-2 text-slate-500">
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
