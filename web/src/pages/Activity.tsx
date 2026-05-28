import {
  createEffect,
  createSignal,
  For,
  Match,
  Show,
  Switch,
  type Component,
} from "solid-js";
import { useNavigate } from "@solidjs/router";
import { fetchActivityStream, type ActivityEvent } from "../lib/api";
import { reasonLabel } from "../lib/decisionReasons";
import { useLivePoll } from "../lib/livePoll";

const POLL_MS = 2000;
/// Client-side cap. The server ring buffer is bounded too; this just keeps the
/// rendered DOM small during a long-running session.
const MAX_EVENTS = 200;

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

function shortHash(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 6)}…${id.slice(-4)}`;
}

/// The global live processing log (POSTSHIP-T33). Polls
/// `/api/v1/me/activity/stream` every couple of seconds, appending whatever the
/// background indexer and rule cycles published since the last cursor.
const Activity: Component = () => {
  const navigate = useNavigate();
  const [events, setEvents] = createSignal<ActivityEvent[]>([]);
  const [lastSeq, setLastSeq] = createSignal(0);
  const [paused, setPaused] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [started, setStarted] = createSignal(false);
  let scrollEl: HTMLDivElement | undefined;

  const poll = async () => {
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

  useLivePoll({ intervalMs: POLL_MS, fetcher: poll });

  // Tail-follow: jump to the newest event on each append, unless the operator
  // is hovering the log to read older entries (pause-on-hover).
  createEffect(() => {
    events();
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

      <div class="rounded-2xl border border-ui-border bg-white shadow-sm dark:border-immich-dark-gray dark:bg-immich-dark-gray">
        <header class="flex flex-wrap items-center gap-3 border-b border-ui-border px-5 py-3 dark:border-gray-700">
          <span class="relative flex h-2.5 w-2.5" aria-hidden="true">
            <span class="absolute inline-flex h-2.5 w-2.5 animate-ping rounded-full bg-immich-primary opacity-60 dark:bg-immich-dark-primary" />
            <span class="relative inline-flex h-2.5 w-2.5 rounded-full bg-immich-primary dark:bg-immich-dark-primary" />
          </span>
          <h2 class="text-base font-semibold">Live processing</h2>
          <span class="ml-auto text-xs text-ui-muted tabular-nums">
            <Show when={paused()} fallback={`${events().length} events`}>
              <span data-testid="activity-paused">Paused — move away to resume</span>
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
            <div
              class="px-6 py-12 text-center"
              data-testid="activity-empty"
            >
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
                <For each={events()}>
                  {(event) => <EventRow event={event} />}
                </For>
              </ul>
            </Show>
          </div>
        </Show>
      </div>
    </section>
  );
};

const KIND_BADGE: Record<ActivityEvent["kind"], { label: string; cls: string }> =
  {
    indexed: {
      label: "indexed",
      cls: "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600",
    },
    matched: {
      label: "matched",
      cls: "bg-emerald-100 text-emerald-800 ring-emerald-200 dark:bg-emerald-500/20 dark:text-emerald-200 dark:ring-emerald-500/30",
    },
    skipped: {
      label: "skipped",
      cls: "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600",
    },
    album_add: {
      label: "album",
      cls: "bg-immich-primary/10 text-immich-primary ring-immich-primary/20 dark:bg-immich-dark-primary/20 dark:text-immich-dark-primary dark:ring-immich-dark-primary/30",
    },
    sweep_done: {
      label: "sweep",
      cls: "bg-slate-100 text-slate-700 ring-slate-200 dark:bg-gray-700 dark:text-gray-200 dark:ring-gray-600",
    },
  };

const EventRow: Component<{ event: ActivityEvent }> = (props) => {
  const badge = () => KIND_BADGE[props.event.kind];
  return (
    <li class="flex items-center gap-3 px-5 py-2.5 text-sm" data-testid="activity-event">
      <span class="w-16 shrink-0 text-xs text-ui-muted tabular-nums">
        {timeLabel(props.event.at)}
      </span>
      <span
        class={`inline-flex w-[4.5rem] shrink-0 items-center justify-center rounded-full px-2 py-0.5 text-xs font-medium ring-1 ring-inset ${badge().cls}`}
      >
        {badge().label}
      </span>
      <Switch>
        <Match when={props.event.kind === "indexed" && props.event}>
          {(e) => (
            <span class="flex min-w-0 items-baseline gap-2">
              <span class="truncate text-immich-fg dark:text-immich-dark-fg">
                {e().filename}
              </span>
              <span class="shrink-0 text-xs text-ui-muted">
                {e().person_count}{" "}
                {e().person_count === 1 ? "person" : "people"}
                {e().has_gps ? " · GPS" : ""}
              </span>
            </span>
          )}
        </Match>
        <Match when={props.event.kind === "matched" && props.event}>
          {(e) => (
            <span class="flex min-w-0 items-center gap-3">
              <AssetThumb assetId={e().asset_id} />
              <span class="min-w-0 truncate">
                <span class="font-medium text-immich-fg dark:text-immich-dark-fg">
                  {e().rule_name}
                </span>
                <span class="text-ui-muted">
                  {" "}
                  matched {e().filename ?? shortHash(e().asset_id)}
                </span>
              </span>
            </span>
          )}
        </Match>
        <Match when={props.event.kind === "skipped" && props.event}>
          {(e) => (
            <span class="flex min-w-0 items-center gap-3">
              <AssetThumb assetId={e().asset_id} />
              <span class="min-w-0 truncate">
                <span class="font-medium text-immich-fg dark:text-immich-dark-fg">
                  {e().rule_name}
                </span>
                <span class="text-ui-muted">
                  {" "}
                  skipped {e().filename ?? shortHash(e().asset_id)} ·{" "}
                  {reasonLabel(e().reason)}
                </span>
              </span>
            </span>
          )}
        </Match>
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

const AssetThumb: Component<{ assetId: string }> = (props) => {
  const [broken, setBroken] = createSignal(false);
  return (
    <span
      class="inline-flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-md bg-slate-100 dark:bg-gray-800"
      data-testid="activity-thumb"
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
          src={assetThumbUrl(props.assetId)}
          alt=""
          class="h-8 w-8 object-cover"
          loading="lazy"
          onError={() => setBroken(true)}
        />
      </Show>
    </span>
  );
};

export default Activity;
