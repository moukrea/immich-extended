import { onCleanup, onMount } from "solid-js";

export interface LivePollOptions {
  intervalMs: number;
  fetcher: () => Promise<void> | void;
}

/**
 * Schedules `fetcher` on `intervalMs` while the document is visible.
 *
 * The first call happens immediately (caller doesn't have to also bootstrap
 * the resource), subsequent calls follow the interval. Polling pauses when
 * `document.visibilityState !== 'visible'` and resumes on the next visibility
 * change — saves a steady drip of fetches while the tab is in the background.
 */
export function useLivePoll(options: LivePollOptions): void {
  let timer: ReturnType<typeof setInterval> | null = null;

  const start = () => {
    if (timer !== null) return;
    timer = setInterval(() => {
      void options.fetcher();
    }, options.intervalMs);
  };

  const stop = () => {
    if (timer === null) return;
    clearInterval(timer);
    timer = null;
  };

  const onVisibility = () => {
    if (typeof document === "undefined") return;
    if (document.visibilityState === "visible") {
      // Catch up immediately on resume — operators expect freshness on focus.
      void options.fetcher();
      start();
    } else {
      stop();
    }
  };

  onMount(() => {
    void options.fetcher();
    if (typeof document === "undefined" || document.visibilityState === "visible") {
      start();
    }
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", onVisibility);
    }
  });

  onCleanup(() => {
    stop();
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", onVisibility);
    }
  });
}
