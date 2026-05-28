import {
  createEffect,
  createMemo,
  createSignal,
  For,
  Match,
  Show,
  Switch,
  type Component,
} from "solid-js";
import { useNavigate } from "@solidjs/router";
import {
  fetchActivityStream,
  fetchIndexStatus,
  type ActivityEvent,
  type IndexStatus,
} from "../lib/api";
import {
  groupActivity,
  type AssetGroup,
  type RuleVerdict,
} from "../lib/activityGrouping";
import { reasonLabel } from "../lib/decisionReasons";
import { useLivePoll } from "../lib/livePoll";

const POLL_MS = 2000;
/// The status header hits Immich's statistics endpoint server-side, so poll it
/// far less often than the cheap local stream.
const STATUS_POLL_MS = 10000;
/// Client-side cap. The server ring buffer is bounded too; this just keeps the
/// rendered DOM small during a long-running session.
const MAX_EVENTS = 200;

type Preview = { src: string; x: number; y: number };

function assetThumbUrl(assetId: string): string {
  return `/api/v1/me/assets/${encodeURIComponent(assetId)}/thumbnail`;
}

function timeLabel(at: number): string {
  if (!Number.isFinite(at) || at <= 0) return "";
  return new Date(at * 1000).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function agoLabel(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds) || seconds <= 0)
    return "never";
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - seconds);
  if (diff < 5) return "just now";
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function shortHash(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 6)}…${id.slice(-4)}`;
}

/// The global live processing log (cycle-6 T44). A status header fed by
/// `/me/index/status` sits above a per-asset grouped narrative of the
/// `/me/activity/stream` events — each asset's indexed/matched/skipped lines
/// fold into one card, with rule-level/sweep summaries interleaved.
const Activity: Component = () => {
  const navigate = useNavigate();
  const [events, setEvents] = createSignal<ActivityEvent[]>([]);
  const [lastSeq, setLastSeq] = createSignal(0);
  const [paused, setPaused] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [started, setStarted] = createSignal(false);
  const [status, setStatus] = createSignal<IndexStatus | null>(null);
  const [preview, setPreview] = createSignal<Preview | null>(null);
  let scrollEl: HTMLDivElement | undefined;

  const rows = createMemo(() => groupActivity(events()));

  const pollStream = async () => {
    const res = await fetchActivityStream(lastSeq());
    setStarted(true);
    if (res.ok) {
      setError(null);
      if (res.data.last_seq > lastSeq()) setLastSeq(res.data.last_seq);
      if (res.data.events.length > 0) {
        setEvents((prev) => {
          // Dedup by seq — defensive against a re-sent tail or an overlapping
          // cursor; the stream is otherwise strictly seq-ordered.
          const seen = new Set(prev.map((e) => e.seq));
          const fresh = res.data.events.filter((e) => !seen.has(e.seq));
          if (fresh.length === 0) return prev;
          const next = [...prev, ...fresh];
          return next.length > MAX_EVENTS
            ? next.slice(next.length - MAX_EVENTS)
            : next;
        });
      }
    } else if (res.status === 401) {
      navigate("/login", { replace: true });
    } else {
      setError("Could not reach the activity stream. Retrying…");
    }
  };

  const pollStatus = async () => {
    const res = await fetchIndexStatus();
    if (res.ok) {
      setStatus(res.data);
    } else if (res.status === 401) {
      navigate("/login", { replace: true });
    }
  };

  useLivePoll({ intervalMs: POLL_MS, fetcher: pollStream });
  useLivePoll({ intervalMs: STATUS_POLL_MS, fetcher: pollStatus });

  // Tail-follow: jump to the newest event on each append, unless the operator
  // is hovering the log to read older entries (pause-on-hover).
  createEffect(() => {
    rows();
    if (!paused() && scrollEl) {
      scrollEl.scrollTop = scrollEl.scrollHeight;
    }
  });

  const isIdle = () => started() && events().length === 0;

  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-baseline gap-3">
        <h1 class="text-2xl font-semibold tracking-tight">Activity</h1>
        <p class="text-sm text-ui-muted">
          A live log of what the background indexer and rule cycles are doing.
        </p>
      </div>

      <StatusHeader status={status()} />

      <div class="mt-5 rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
        <header class="flex flex-wrap items-center gap-3 border-b border-ui-border px-5 py-3 dark:border-gray-700">
          <span class="relative flex h-2.5 w-2.5" aria-hidden="true">
            <span class="absolute inline-flex h-2.5 w-2.5 animate-ping rounded-full bg-immich-primary opacity-60 dark:bg-immich-dark-primary" />
            <span class="relative inline-flex h-2.5 w-2.5 rounded-full bg-immich-primary dark:bg-immich-dark-primary" />
          </span>
          <h2 class="text-base font-semibold">Live processing</h2>
          <span class="ml-auto text-xs text-ui-muted tabular-nums">
            <Show when={paused()} fallback={`${events().length} events`}>
              <span data-testid="activity-paused">
                Paused — move away to resume
              </span>
            </Show>
          </span>
        </header>

        <Show when={error()}>
          {(message) => (
            <p
              class="border-b border-ui-border px-5 py-2 text-xs text-ui-danger dark:border-gray-700"
              role="alert"
            >
              {message()}
            </p>
          )}
        </Show>

        <Show
          when={!isIdle()}
          fallback={
            <div class="px-6 py-12 text-center" data-testid="activity-empty">
              <h3 class="text-base font-medium text-immich-fg dark:text-immich-dark-fg">
                Nothing processing right now
              </h3>
              <p class="mx-auto mt-1 max-w-md text-sm text-ui-muted">
                As the indexer sweeps your library and rules run their cycles,
                each asset and decision will stream in here.
              </p>
            </div>
          }
        >
          <div
            ref={scrollEl}
            class="max-h-[32rem] overflow-y-auto"
            data-testid="activity-log"
            onMouseEnter={() => setPaused(true)}
            onMouseLeave={() => setPaused(false)}
          >
            <Show
              when={events().length > 0}
              fallback={
                <p class="px-5 py-6 text-sm text-ui-muted">Connecting…</p>
              }
            >
              <ul class="divide-y divide-ui-border dark:divide-gray-800">
                <For each={rows()}>
                  {(row) => (
                    <Switch>
                      <Match when={row.kind === "asset" && row}>
                        {(g) => (
                          <AssetCard
                            group={g()}
                            onPreview={setPreview}
                            onPreviewEnd={() => setPreview(null)}
                          />
                        )}
                      </Match>
                      <Match when={row.kind === "summary" && row}>
                        {(s) => <SummaryRow event={s().event} />}
                      </Match>
                    </Switch>
                  )}
                </For>
              </ul>
            </Show>
          </div>
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
            <img src={p().src} alt="" class="h-48 w-48 object-cover" loading="lazy" />
          </div>
        )}
      </Show>
    </section>
  );
};

const StatusHeader: Component<{ status: IndexStatus | null }> = (props) => {
  const indexedLabel = () => {
    const s = props.status;
    if (!s) return "—";
    const n = s.indexed.toLocaleString();
    return s.library_total === null
      ? n
      : `${n} / ${s.library_total.toLocaleString()}`;
  };
  const sweeping = () => props.status?.sweeping ?? false;

  return (
    <div
      class="flex flex-wrap items-center gap-x-6 gap-y-2 rounded-2xl border border-ui-border bg-white px-5 py-4 shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray"
      data-testid="activity-status"
    >
      <div class="flex items-baseline gap-2">
        <span class="text-xs uppercase tracking-wider text-ui-muted">
          Indexed
        </span>
        <span class="text-lg font-semibold tabular-nums text-immich-fg dark:text-immich-dark-fg">
          {indexedLabel()}
        </span>
      </div>
      <div class="flex items-baseline gap-2">
        <span class="text-xs uppercase tracking-wider text-ui-muted">
          Last sweep
        </span>
        <span class="text-sm text-immich-fg dark:text-immich-dark-fg">
          {agoLabel(props.status?.last_swept_at ?? null)}
        </span>
      </div>
      <span
        class={`ml-auto inline-flex items-center gap-2 rounded-full px-3 py-1 text-xs font-medium ring-1 ring-inset ${
          sweeping()
            ? "bg-immich-primary/10 text-immich-primary ring-immich-primary/20 dark:bg-immich-dark-primary/20 dark:text-immich-dark-primary dark:ring-immich-dark-primary/30"
            : "bg-slate-100 text-slate-600 ring-slate-200 dark:bg-gray-700 dark:text-gray-300 dark:ring-gray-600"
        }`}
        data-testid="activity-state"
      >
        <Show
          when={sweeping()}
          fallback={
            <>
              <span class="h-1.5 w-1.5 rounded-full bg-slate-400 dark:bg-gray-400" />
              idle
            </>
          }
        >
          <span class="relative flex h-1.5 w-1.5" aria-hidden="true">
            <span class="absolute inline-flex h-1.5 w-1.5 animate-ping rounded-full bg-immich-primary opacity-60 dark:bg-immich-dark-primary" />
            <span class="relative inline-flex h-1.5 w-1.5 rounded-full bg-immich-primary dark:bg-immich-dark-primary" />
          </span>
          indexing
        </Show>
      </span>
    </div>
  );
};

const AssetCard: Component<{
  group: AssetGroup;
  onPreview: (p: Preview) => void;
  onPreviewEnd: () => void;
}> = (props) => {
  const label = () =>
    props.group.filename ?? shortHash(props.group.asset_id);
  return (
    <li
      class="flex items-start gap-3 px-5 py-3 text-sm"
      data-testid="activity-asset"
    >
      <span class="w-16 shrink-0 pt-0.5 text-xs text-ui-muted tabular-nums">
        {timeLabel(props.group.at)}
      </span>
      <AssetThumb
        assetId={props.group.asset_id}
        onPreview={props.onPreview}
        onPreviewEnd={props.onPreviewEnd}
      />
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-baseline gap-x-2">
          <span class="truncate font-medium text-immich-fg dark:text-immich-dark-fg">
            {label()}
          </span>
          <Show when={props.group.indexed}>
            {(idx) => (
              <span class="shrink-0 text-xs text-ui-muted">
                indexed · {idx().person_count}{" "}
                {idx().person_count === 1 ? "person" : "people"}
                {idx().has_gps ? " · GPS" : ""}
              </span>
            )}
          </Show>
        </div>
        <Show when={props.group.verdicts.length > 0}>
          <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
            <For each={props.group.verdicts}>
              {(verdict) => <VerdictChip verdict={verdict} />}
            </For>
          </div>
        </Show>
      </div>
    </li>
  );
};

const VerdictChip: Component<{ verdict: RuleVerdict }> = (props) => {
  const matched = () => props.verdict.decision === "matched";
  return (
    <span
      class={`inline-flex max-w-full items-center gap-1 rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${
        matched()
          ? "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30"
          : "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600"
      }`}
      data-testid={matched() ? "verdict-matched" : "verdict-skipped"}
    >
      <span>{matched() ? "matched" : "skipped"}</span>
      <span class="truncate font-semibold">“{props.verdict.rule_name}”</span>
      <Show when={!matched() && props.verdict.reason}>
        {(reason) => (
          <span class="text-ui-muted">· {reasonLabel(reason())}</span>
        )}
      </Show>
    </span>
  );
};

const SummaryRow: Component<{
  event: Extract<ActivityEvent, { kind: "album_add" | "sweep_done" }>;
}> = (props) => {
  return (
    <li
      class="flex items-center gap-3 px-5 py-2.5 text-sm"
      data-testid="activity-summary"
    >
      <span class="w-16 shrink-0 text-xs text-ui-muted tabular-nums">
        {timeLabel(props.event.at)}
      </span>
      <Switch>
        <Match when={props.event.kind === "album_add" && props.event}>
          {(e) => (
            <span class="min-w-0 truncate">
              <span class="font-medium text-immich-fg dark:text-immich-dark-fg">
                {e().rule_name}
              </span>
              <span class="text-ui-muted">
                {" "}
                filed {e().added_count}{" "}
                {e().added_count === 1 ? "asset" : "assets"} into its album
              </span>
            </span>
          )}
        </Match>
        <Match when={props.event.kind === "sweep_done" && props.event}>
          {(e) => (
            <span class="text-ui-muted">
              <Show
                when={e().indexed > 0}
                fallback="Library sweep — nothing new"
              >
                Library sweep — indexed {e().indexed}{" "}
                {e().indexed === 1 ? "asset" : "assets"}
              </Show>
            </span>
          )}
        </Match>
      </Switch>
    </li>
  );
};

const AssetThumb: Component<{
  assetId: string;
  onPreview: (p: Preview) => void;
  onPreviewEnd: () => void;
}> = (props) => {
  const [broken, setBroken] = createSignal(false);
  const src = () => assetThumbUrl(props.assetId);

  const showPreview = (e: MouseEvent) => {
    if (broken()) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    // Anchor to the right of the thumbnail, clamped into the viewport so a row
    // near an edge doesn't push the preview off-screen.
    const x = Math.min(rect.right + 8, window.innerWidth - 208);
    const y = Math.min(rect.top, window.innerHeight - 208);
    props.onPreview({ src: src(), x: Math.max(8, x), y: Math.max(8, y) });
  };

  return (
    <span
      class="inline-flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-md bg-slate-100 dark:bg-gray-800"
      data-testid="activity-thumb"
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
          alt=""
          class="h-9 w-9 object-cover"
          loading="lazy"
          onError={() => setBroken(true)}
        />
      </Show>
    </span>
  );
};

export default Activity;
