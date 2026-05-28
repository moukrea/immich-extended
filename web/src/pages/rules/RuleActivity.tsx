import { createSignal, For, onMount, Show, type Component } from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import { fetchDecisions, getRule, type DecisionItem } from "../../lib/api";
import { reasonLabel } from "../../lib/decisionReasons";

const PAGE_SIZE = 50;
/// How close to the bottom of the scroll container (px) before the next page
/// is fetched. Generous so the append feels seamless rather than juddery.
const SCROLL_THRESHOLD_PX = 160;

type Filter = "all" | "matched" | "skipped";

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "matched", label: "Matched" },
  { key: "skipped", label: "Skipped" },
];

/// The Matched/Skipped chips map onto the `decision` verdict column; All sends
/// no filter.
function decisionParam(filter: Filter): "added" | "skipped" | undefined {
  if (filter === "matched") return "added";
  if (filter === "skipped") return "skipped";
  return undefined;
}

function assetThumbUrl(assetId: string): string {
  return `/api/v1/me/assets/${encodeURIComponent(assetId)}/thumbnail`;
}

const RuleActivity: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id: string }>();

  const [ruleName, setRuleName] = createSignal<string | null>(null);
  const [filter, setFilter] = createSignal<Filter>("all");
  const [decisions, setDecisions] = createSignal<DecisionItem[]>([]);
  const [total, setTotal] = createSignal(0);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(false);

  // Enlarged-thumbnail hover preview. Held at the page level with a `fixed`
  // position so it escapes the table's `overflow` clip instead of being
  // truncated inside the scroll container.
  const [preview, setPreview] = createSignal<{
    src: string;
    x: number;
    y: number;
  } | null>(null);

  const hasMore = () => decisions().length < total();

  // Monotonic request token: a newer load (e.g. a filter switch) supersedes an
  // in-flight one so a slow earlier response can't clobber fresher rows.
  let reqSeq = 0;

  const loadPage = async (reset: boolean) => {
    if (loading() && !reset) return;
    const mySeq = ++reqSeq;
    setLoading(true);
    const offset = reset ? 0 : decisions().length;
    const res = await fetchDecisions(params.id, {
      limit: PAGE_SIZE,
      offset,
      decision: decisionParam(filter()),
    });
    if (mySeq !== reqSeq) return; // superseded
    setLoading(false);
    setLoaded(true);
    if (res.ok) {
      setError(null);
      setTotal(res.data.total);
      if (reset) {
        setDecisions(res.data.decisions);
      } else {
        setDecisions((prev) => [...prev, ...res.data.decisions]);
      }
    } else if (res.status === 401) {
      navigate("/login", { replace: true });
    } else if (res.status === 404) {
      setError("Rule not found.");
    } else {
      setError("Could not load decisions.");
    }
  };

  const selectFilter = (next: Filter) => {
    if (next === filter()) return;
    setFilter(next);
    setDecisions([]);
    void loadPage(true);
  };

  const onScroll = (e: Event) => {
    const el = e.currentTarget as HTMLElement;
    const remaining = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (remaining <= SCROLL_THRESHOLD_PX && hasMore() && !loading()) {
      void loadPage(false);
    }
  };

  onMount(() => {
    void getRule(params.id).then((res) => {
      if (res.ok) setRuleName(res.data.name);
      else if (res.status === 401) navigate("/login", { replace: true });
    });
    void loadPage(true);
  });

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6">
        <A
          href={`/rules/${params.id}`}
          class="text-sm text-ui-muted hover:text-immich-fg dark:hover:text-immich-dark-fg"
        >
          ← Back to rule
        </A>
        <h1 class="mt-1 text-2xl font-semibold tracking-tight">
          Activity
          <Show when={ruleName()}>
            {(name) => (
              <span class="text-ui-muted font-normal"> — {name()}</span>
            )}
          </Show>
        </h1>
        <p class="mt-1 text-sm text-ui-muted">
          Every asset this rule has matched or skipped, newest first.
        </p>
      </div>

      <div class="rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
        <header class="flex flex-wrap items-center gap-3 border-b border-ui-border px-5 py-3 dark:border-gray-700">
          <h2 class="text-base font-semibold">Decisions</h2>
          <div
            class="flex items-center gap-1"
            role="tablist"
            aria-label="Filter decisions"
          >
            <For each={FILTERS}>
              {(f) => (
                <button
                  type="button"
                  role="tab"
                  aria-selected={filter() === f.key}
                  data-testid={`filter-${f.key}`}
                  onClick={() => selectFilter(f.key)}
                  class={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                    filter() === f.key
                      ? "bg-immich-primary text-white dark:bg-immich-dark-primary dark:text-immich-dark-bg"
                      : "text-ui-muted hover:bg-slate-100 dark:hover:bg-gray-700/50"
                  }`}
                >
                  {f.label}
                </button>
              )}
            </For>
          </div>
          <span class="ml-auto text-xs text-ui-muted tabular-nums">
            <Show when={loaded()} fallback="Loading…">
              {total()} {filter() === "all" ? "total" : "shown"}
            </Show>
          </span>
        </header>

        <Show
          when={error() === null}
          fallback={
            <p class="px-5 py-6 text-sm text-ui-danger" role="alert">
              {error()}
            </p>
          }
        >
          <Show
            when={loaded()}
            fallback={
              <p class="px-5 py-6 text-sm text-ui-muted">Loading decisions…</p>
            }
          >
            <Show
              when={decisions().length > 0}
              fallback={
                <p class="px-5 py-6 text-sm text-ui-muted">
                  No {filter() === "all" ? "" : `${filter()} `}decisions yet.
                  They'll appear here after the rule's next poll cycle.
                </p>
              }
            >
              <div
                class="max-h-[28rem] overflow-y-auto"
                data-testid="decisions-scroll"
                onScroll={onScroll}
              >
                <table class="min-w-full text-sm">
                  <thead class="sticky top-0 z-10 bg-slate-50 dark:bg-immich-dark-gray">
                    <tr class="border-b border-ui-border dark:border-gray-700">
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
                    <For each={decisions()}>
                      {(row) => (
                        <DecisionRow
                          row={row}
                          onPreview={setPreview}
                          onPreviewEnd={() => setPreview(null)}
                        />
                      )}
                    </For>
                  </tbody>
                </table>

                <div class="px-4 py-3 text-center">
                  <Show
                    when={hasMore()}
                    fallback={
                      <span class="text-xs text-ui-muted">
                        <Show when={decisions().length > 0}>
                          End of list — {decisions().length} shown
                        </Show>
                      </span>
                    }
                  >
                    <button
                      type="button"
                      data-testid="load-more"
                      disabled={loading()}
                      onClick={() => void loadPage(false)}
                      class="rounded-lg border border-ui-border px-3 py-1.5 text-xs font-medium text-ui-muted hover:bg-slate-100 disabled:opacity-50 dark:border-gray-700 dark:hover:bg-gray-700/50"
                    >
                      {loading() ? "Loading…" : "Load more"}
                    </button>
                  </Show>
                </div>
              </div>
            </Show>
          </Show>
        </Show>
      </div>

      <Show when={preview()}>
        {(p) => (
          <div
            ref={(el) => {
              el.style.left = `${p().x}px`;
              el.style.top = `${p().y}px`;
            }}
            class="pointer-events-none fixed z-50 overflow-hidden rounded-xl border border-ui-border bg-white shadow-xl dark:border-gray-700 dark:bg-immich-dark-gray"
            data-testid="thumb-preview"
          >
            <img
              src={p().src}
              alt=""
              class="h-48 w-48 object-cover"
              loading="lazy"
            />
          </div>
        )}
      </Show>
    </section>
  );
};

const DecisionRow: Component<{
  row: DecisionItem;
  onPreview: (p: { src: string; x: number; y: number }) => void;
  onPreviewEnd: () => void;
}> = (props) => {
  const [broken, setBroken] = createSignal(false);
  const src = () => assetThumbUrl(props.row.asset_id);
  const label = () => props.row.filename ?? shortHash(props.row.asset_id);

  const decisionClass = () =>
    props.row.decision === "added"
      ? "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30"
      : "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600";

  const showPreview = (e: MouseEvent) => {
    if (broken()) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Anchor to the right of the thumbnail, clamped into the viewport so a row
    // near the bottom of the table doesn't push the preview off-screen.
    const x = Math.min(rect.right + 8, window.innerWidth - 208);
    const y = Math.min(rect.top, window.innerHeight - 208);
    props.onPreview({ src: src(), x: Math.max(8, x), y: Math.max(8, y) });
  };

  return (
    <tr>
      <td class="px-4 py-2">
        <div class="flex items-center gap-3">
          <span
            class="inline-flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-md bg-slate-100 dark:bg-gray-800"
            data-testid="thumb"
            onMouseEnter={showPreview}
            onMouseLeave={() => props.onPreviewEnd()}
          >
            <Show
              when={!broken()}
              fallback={
                <span class="text-[10px] text-ui-muted" aria-hidden="true">
                  ?
                </span>
              }
            >
              <img
                src={src()}
                alt={label()}
                class="h-9 w-9 object-cover"
                loading="lazy"
                onError={() => setBroken(true)}
              />
            </Show>
          </span>
          <span
            class="max-w-[16rem] truncate text-immich-fg dark:text-immich-dark-fg"
            title={props.row.asset_id}
          >
            {label()}
          </span>
        </div>
      </td>
      <td class="px-4 py-2">
        <span
          class={`inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${decisionClass()}`}
        >
          {props.row.decision === "added" ? "matched" : "skipped"}
        </span>
      </td>
      <td class="px-4 py-2 text-ui-muted">{reasonLabel(props.row.reason)}</td>
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

export default RuleActivity;
