import { type Component } from "solid-js";

// Global activity live log. POSTSHIP-T33 wires this to the indexer + rule-cycle
// event stream (`/api/v1/me/activity/stream`); until then it shows an honest
// placeholder so the nav destination exists and reads intentionally.
const Activity: Component = () => {
  return (
    <section class="max-w-5xl mx-auto">
      <div class="mb-6 flex flex-wrap items-baseline gap-3">
        <h1 class="text-2xl font-semibold tracking-tight">Activity</h1>
        <p class="text-sm text-ui-muted">
          A live log of what the background indexer and rule cycles are doing.
        </p>
      </div>

      <div class="rounded-2xl border border-dashed border-ui-border bg-white px-6 py-12 text-center dark:border-gray-700 dark:bg-immich-dark-gray">
        <span
          class="mx-auto mb-3 flex h-3 w-3 items-center justify-center"
          aria-hidden="true"
        >
          <span class="absolute inline-flex h-3 w-3 animate-ping rounded-full bg-immich-primary opacity-60 dark:bg-immich-dark-primary" />
          <span class="relative inline-flex h-2.5 w-2.5 rounded-full bg-immich-primary dark:bg-immich-dark-primary" />
        </span>
        <h2 class="text-base font-medium text-immich-fg dark:text-immich-dark-fg">
          Live activity is on its way
        </h2>
        <p class="mx-auto mt-1 max-w-md text-sm text-ui-muted">
          This view will stream the asset currently being processed — what was
          retrieved (people, faces, location) and the decision each rule made.
          Per-rule history is available now from each rule's Activity link.
        </p>
      </div>
    </section>
  );
};

export default Activity;
