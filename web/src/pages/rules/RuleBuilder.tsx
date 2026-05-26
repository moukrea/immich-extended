import {
  batch,
  createMemo,
  createResource,
  createSignal,
  For,
  lazy,
  Show,
  Suspense,
  untrack,
  type Component,
} from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import {
  createRule,
  deleteRule,
  fetchAlbums,
  getRule,
  postLogout,
  updateRule,
  type MeAlbum,
  type Rule,
  type RuleStatus,
} from "../../lib/api";
import {
  defaultBuilderState,
  formStateToYaml,
  peopleValueToYaml,
  peopleYamlToValue,
  yamlToFormState,
  type RuleBuilderState,
  type TargetAlbumState,
} from "../../lib/ruleYaml";
import ConfirmDialog from "../../components/ConfirmDialog";
import { humanRuleError } from "./errors";

const MapPicker = lazy(() => import("../../components/MapPicker"));

const MANAGED_OPTION_VALUE = "__managed__";
type PendingLifecycle = "archive" | "delete";

function deriveStateFromRule(rule: Rule): RuleBuilderState {
  const parsed = yamlToFormState(rule.yaml_source);
  // Server-side status is the lifecycle-button authoritative source; pin it
  // over whatever the YAML happens to declare.
  return { ...parsed.state, id: rule.id, status: rule.status };
}

const RuleBuilder: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id?: string }>();
  const mode = createMemo<"new" | "edit">(() => (params.id ? "edit" : "new"));

  const [state, setState] = createSignal<RuleBuilderState>(
    defaultBuilderState(),
  );
  const [yamlText, setYamlText] = createSignal<string>(
    formStateToYaml(defaultBuilderState()),
  );
  const [yamlError, setYamlError] = createSignal<string | null>(null);
  const [untouched, setUntouched] = createSignal<string[]>([]);
  const [peopleYaml, setPeopleYaml] = createSignal<string>("");
  const [peopleError, setPeopleError] = createSignal<string | null>(null);
  const [showMap, setShowMap] = createSignal(false);
  const [showAdvanced, setShowAdvanced] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [lifecycleBusy, setLifecycleBusy] = createSignal(false);
  const [pending, setPending] = createSignal<PendingLifecycle | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [originalName, setOriginalName] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(untrack(() => mode() === "new"));

  const [albumsResource] = createResource<MeAlbum[]>(async () => {
    const result = await fetchAlbums();
    if (!result.ok) return [];
    return result.data;
  });

  const writableAlbums = createMemo<MeAlbum[]>(() => {
    const list = albumsResource();
    if (!list) return [];
    return list.filter((a) => a.is_writable);
  });

  const [existing] = createResource<Rule | null, string | undefined>(
    () => (mode() === "edit" ? params.id : undefined),
    async (id) => {
      if (!id) return null;
      const result = await getRule(id);
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
        setLoaded(true);
        return null;
      }
      const next = deriveStateFromRule(result.data);
      batch(() => {
        setState(next);
        setYamlText(formStateToYaml(next));
        setPeopleYaml(peopleValueToYaml(next.people_raw));
        setOriginalName(result.data.name);
        setLoaded(true);
      });
      return result.data;
    },
  );

  const heading = createMemo(() => {
    if (mode() === "new") return "New rule";
    const name = originalName();
    return name ? `Edit "${name}"` : "Edit rule";
  });

  // Single mutator that keeps `state` and `yamlText` in lock-step when the
  // edit originates from the visual form. The Advanced-YAML textarea has its
  // own path (`onYamlInput`) so a user typing partial YAML doesn't lose their
  // place to a round-trip.
  const mutateForm = (updater: (prev: RuleBuilderState) => RuleBuilderState) => {
    const next = updater(state());
    const newYaml = formStateToYaml(next);
    batch(() => {
      setState(next);
      setYamlText(newYaml);
      setYamlError(null);
    });
  };

  const onYamlInput = (text: string) => {
    setYamlText(text);
    const parsed = yamlToFormState(text);
    setYamlError(parsed.error);
    if (parsed.error !== null) return;
    batch(() => {
      // Preserve form-only fields that the YAML doesn't carry: nothing today,
      // but keep the merge pattern in place for future extensions.
      setState({ ...parsed.state, id: state().id ?? parsed.state.id });
      setUntouched(parsed.untouched);
      setPeopleYaml(peopleValueToYaml(parsed.state.people_raw));
      setPeopleError(null);
    });
  };

  const onNameInput = (value: string) =>
    mutateForm((s) => ({ ...s, name: value }));

  const onAlbumSelect = (selected: string) => {
    mutateForm((s) => {
      if (selected === MANAGED_OPTION_VALUE) {
        if (s.target.kind === "managed") return s;
        return {
          ...s,
          target: { kind: "managed", name: "", shared_with: [] },
        };
      }
      if (s.target.kind === "existing" && s.target.album_id === selected) {
        return s;
      }
      return { ...s, target: { kind: "existing", album_id: selected } };
    });
  };

  const onManagedNameInput = (name: string) =>
    mutateForm((s) => {
      if (s.target.kind !== "managed") return s;
      return { ...s, target: { ...s.target, name } };
    });

  const onToggleDate = (enabled: boolean) =>
    mutateForm((s) => ({ ...s, date_enabled: enabled }));
  const onDateFromInput = (v: string) =>
    mutateForm((s) => ({ ...s, date_from: v }));
  const onDateToInput = (v: string) =>
    mutateForm((s) => ({ ...s, date_to: v }));

  const onToggleLocation = (enabled: boolean) =>
    mutateForm((s) => ({ ...s, location_enabled: enabled }));
  const onMapChange = (center: [number, number], radiusKm: number) =>
    mutateForm((s) => ({
      ...s,
      location_center: center,
      location_radius_km: radiusKm,
    }));

  const onTogglePeople = (enabled: boolean) =>
    mutateForm((s) => ({ ...s, people_enabled: enabled }));
  const onPeopleTextareaInput = (text: string) => {
    setPeopleYaml(text);
    const { value, error: peErr } = peopleYamlToValue(text);
    setPeopleError(peErr);
    if (peErr !== null) return;
    mutateForm((s) => ({ ...s, people_raw: value }));
  };

  const onToggleMedia = (enabled: boolean) =>
    mutateForm((s) => ({ ...s, media_enabled: enabled }));
  const onTogglePhoto = (v: boolean) =>
    mutateForm((s) => ({ ...s, media_photo: v }));
  const onToggleVideo = (v: boolean) =>
    mutateForm((s) => ({ ...s, media_video: v }));

  const selectedAlbumValue = createMemo(() => {
    const t = state().target;
    return t.kind === "managed" ? MANAGED_OPTION_VALUE : t.album_id;
  });

  const orphanAlbumId = createMemo<string>(() => {
    const t = state().target;
    return t.kind === "existing" ? t.album_id : "";
  });

  const managedAlbumName = createMemo<string>(() => {
    const t = state().target;
    return t.kind === "managed" ? t.name : "";
  });

  // When editing a rule whose existing album_id isn't in the user's writable
  // list (lost write access, deleted, etc.), surface a non-functional option
  // so the value stays in the select rather than silently flipping to the
  // first writable album.
  const showOrphanAlbumOption = createMemo(() => {
    const t = state().target;
    if (t.kind !== "existing") return false;
    return !writableAlbums().some((a) => a.id === t.album_id);
  });

  const onLogout = async () => {
    await postLogout();
    navigate("/login", { replace: true });
  };

  const applyStatus = async (next: RuleStatus) => {
    const id = params.id;
    if (!id) return;
    setLifecycleBusy(true);
    setError(null);
    const result = await updateRule(id, { status: next });
    setLifecycleBusy(false);
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError(humanRuleError(result.error));
      return;
    }
    mutateForm((s) => ({ ...s, status: next }));
  };

  const onTogglePause = () => {
    const current = state().status;
    const next: RuleStatus = current === "paused" ? "active" : "paused";
    void applyStatus(next);
  };
  const onArchive = () => setPending("archive");
  const onDelete = () => setPending("delete");
  const onCancelPending = () => setPending(null);

  const onConfirmPending = async () => {
    const action = pending();
    if (!action) return;
    setPending(null);
    if (action === "archive") {
      await applyStatus("archived");
      return;
    }
    const id = params.id;
    if (!id) return;
    setLifecycleBusy(true);
    setError(null);
    const result = await deleteRule(id);
    setLifecycleBusy(false);
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError(humanRuleError(result.error));
      return;
    }
    navigate("/rules", { replace: true });
  };

  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (saving()) return;
    const yaml = yamlText();
    if (yaml.trim().length === 0) {
      setError("YAML cannot be empty.");
      return;
    }
    setSaving(true);
    setError(null);

    if (mode() === "new") {
      const result = await createRule(yaml);
      setSaving(false);
      if (!result.ok) {
        if (result.status === 401) {
          navigate("/login", { replace: true });
          return;
        }
        setError(humanRuleError(result.error));
        return;
      }
      navigate("/rules", { replace: true });
      return;
    }

    const id = params.id;
    if (!id) {
      setError("Missing rule id.");
      setSaving(false);
      return;
    }
    const result = await updateRule(id, { yaml_source: yaml });
    setSaving(false);
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
        return;
      }
      setError(humanRuleError(result.error));
      return;
    }
    navigate("/rules", { replace: true });
  };

  return (
    <main class="min-h-screen bg-slate-50">
      <header class="bg-white border-b border-slate-200">
        <div class="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <div class="flex items-center gap-4">
            <A href="/rules" class="text-sm text-slate-500 hover:text-slate-700">
              ← Rules
            </A>
            <h1 class="text-lg font-semibold text-slate-900">{heading()}</h1>
            <Show when={mode() === "edit"}>
              <span
                class="rounded-full px-2 py-0.5 text-xs font-medium uppercase tracking-wide text-slate-700 bg-slate-100"
                aria-label="Rule status"
              >
                {state().status}
              </span>
            </Show>
          </div>
          <div class="flex items-center gap-2">
            <Show when={mode() === "edit" && params.id}>
              <A
                href={`/rules/${params.id}/decisions`}
                class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
              >
                Decisions
              </A>
            </Show>
            <button
              type="button"
              onClick={onLogout}
              class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
            >
              Sign out
            </button>
          </div>
        </div>
      </header>

      <section class="max-w-3xl mx-auto px-4 py-8">
        <Show
          when={loaded()}
          fallback={<p class="text-slate-500">Loading rule…</p>}
        >
          <form class="space-y-6" onSubmit={onSubmit}>
            <div>
              <label
                class="block text-sm font-medium text-slate-700"
                for="rule-name"
              >
                Name
              </label>
              <input
                id="rule-name"
                type="text"
                value={state().name}
                onInput={(e) => onNameInput(e.currentTarget.value)}
                placeholder="Vacation 2024"
                class="mt-1 block w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              />
            </div>

            <fieldset class="rounded-md border border-slate-200 bg-white p-4">
              <legend class="px-1 text-sm font-semibold text-slate-700">
                Target album
              </legend>
              <label
                class="block text-xs font-medium text-slate-600"
                for="rule-album"
              >
                Where matching assets are added
              </label>
              <select
                id="rule-album"
                value={selectedAlbumValue()}
                onChange={(e) => onAlbumSelect(e.currentTarget.value)}
                class="mt-1 block w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              >
                <option value={MANAGED_OPTION_VALUE}>
                  Create / manage a new album
                </option>
                <Show when={showOrphanAlbumOption()}>
                  <option value={orphanAlbumId()} disabled>
                    Currently selected (no write access)
                  </option>
                </Show>
                <For each={writableAlbums()}>
                  {(a) => (
                    <option value={a.id}>
                      {a.name}
                      {a.asset_count > 0 ? ` (${a.asset_count})` : ""}
                    </option>
                  )}
                </For>
              </select>
              <Show when={state().target.kind === "managed"}>
                <label
                  class="mt-3 block text-xs font-medium text-slate-600"
                  for="rule-album-name"
                >
                  Managed album name
                </label>
                <input
                  id="rule-album-name"
                  type="text"
                  value={managedAlbumName()}
                  onInput={(e) => onManagedNameInput(e.currentTarget.value)}
                  placeholder="Vacation 2024"
                  class="mt-1 block w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                />
              </Show>
            </fieldset>

            <fieldset class="rounded-md border border-slate-200 bg-white p-4">
              <legend class="px-1 text-sm font-semibold text-slate-700">
                Date range
              </legend>
              <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                <input
                  type="checkbox"
                  checked={state().date_enabled}
                  onChange={(e) => onToggleDate(e.currentTarget.checked)}
                  aria-label="Enable date filter"
                />
                Restrict to a date range
              </label>
              <Show when={state().date_enabled}>
                <div class="mt-3 grid grid-cols-1 sm:grid-cols-2 gap-3">
                  <div>
                    <label
                      class="block text-xs font-medium text-slate-600"
                      for="rule-date-from"
                    >
                      From
                    </label>
                    <input
                      id="rule-date-from"
                      type="date"
                      value={state().date_from}
                      onInput={(e) => onDateFromInput(e.currentTarget.value)}
                      class="mt-1 block w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                    />
                  </div>
                  <div>
                    <label
                      class="block text-xs font-medium text-slate-600"
                      for="rule-date-to"
                    >
                      To
                    </label>
                    <input
                      id="rule-date-to"
                      type="date"
                      value={state().date_to}
                      onInput={(e) => onDateToInput(e.currentTarget.value)}
                      class="mt-1 block w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                    />
                  </div>
                </div>
              </Show>
            </fieldset>

            <fieldset class="rounded-md border border-slate-200 bg-white p-4">
              <legend class="px-1 text-sm font-semibold text-slate-700">
                Location
              </legend>
              <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                <input
                  type="checkbox"
                  checked={state().location_enabled}
                  onChange={(e) => onToggleLocation(e.currentTarget.checked)}
                  aria-label="Enable location filter"
                />
                Restrict to a geo radius
              </label>
              <Show when={state().location_enabled}>
                <div class="mt-3">
                  <button
                    type="button"
                    onClick={() => setShowMap((s) => !s)}
                    class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
                    aria-expanded={showMap()}
                    aria-controls="rule-map-picker"
                  >
                    {showMap() ? "Hide map picker" : "Open map picker"}
                  </button>
                  <p class="mt-1 text-xs text-slate-500">
                    Click the map to set the center; the slider sets the
                    radius.
                  </p>
                  <Show when={showMap()}>
                    <div id="rule-map-picker" class="mt-3">
                      <Suspense
                        fallback={
                          <p class="text-sm text-slate-500">Loading map…</p>
                        }
                      >
                        <MapPicker
                          center={state().location_center}
                          radiusKm={state().location_radius_km}
                          onChange={onMapChange}
                        />
                      </Suspense>
                    </div>
                  </Show>
                </div>
              </Show>
            </fieldset>

            <fieldset class="rounded-md border border-slate-200 bg-white p-4">
              <legend class="px-1 text-sm font-semibold text-slate-700">
                People
              </legend>
              <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                <input
                  type="checkbox"
                  checked={state().people_enabled}
                  onChange={(e) => onTogglePeople(e.currentTarget.checked)}
                  aria-label="Enable people filter"
                />
                Restrict by people
              </label>
              <Show when={state().people_enabled}>
                <p class="mt-2 text-xs text-slate-500">
                  The structured multi-select with thumbnails ships in the
                  next iteration (M6-T6). For now, paste the{" "}
                  <code>people:</code> YAML body directly.
                </p>
                <textarea
                  value={peopleYaml()}
                  onInput={(e) => onPeopleTextareaInput(e.currentTarget.value)}
                  placeholder={"must_include: [paloma-id]\nmust_exclude_other_identifiable: true"}
                  class="mt-2 block h-32 w-full rounded-md border border-slate-300 px-3 py-2 font-mono text-xs leading-relaxed focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                  spellcheck={false}
                  aria-label="People predicate YAML"
                />
                <Show when={peopleError()}>
                  <p class="mt-1 text-xs text-red-700">{peopleError()}</p>
                </Show>
              </Show>
            </fieldset>

            <fieldset class="rounded-md border border-slate-200 bg-white p-4">
              <legend class="px-1 text-sm font-semibold text-slate-700">
                Media types
              </legend>
              <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                <input
                  type="checkbox"
                  checked={state().media_enabled}
                  onChange={(e) => onToggleMedia(e.currentTarget.checked)}
                  aria-label="Enable media filter"
                />
                Restrict to specific media types
              </label>
              <Show when={state().media_enabled}>
                <div class="mt-3 flex gap-4">
                  <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                    <input
                      type="checkbox"
                      checked={state().media_photo}
                      onChange={(e) => onTogglePhoto(e.currentTarget.checked)}
                      aria-label="Photo media type"
                    />
                    Photo
                  </label>
                  <label class="inline-flex items-center gap-2 text-sm text-slate-700">
                    <input
                      type="checkbox"
                      checked={state().media_video}
                      onChange={(e) => onToggleVideo(e.currentTarget.checked)}
                      aria-label="Video media type"
                    />
                    Video
                  </label>
                </div>
              </Show>
            </fieldset>

            <div class="rounded-md border border-slate-200 bg-white">
              <button
                type="button"
                onClick={() => setShowAdvanced((s) => !s)}
                aria-expanded={showAdvanced()}
                aria-controls="rule-yaml-panel"
                class="w-full px-4 py-3 text-left text-sm font-semibold text-slate-700 hover:bg-slate-50"
              >
                {showAdvanced() ? "▾" : "▸"} Advanced (YAML)
              </button>
              <Show when={showAdvanced()}>
                <div id="rule-yaml-panel" class="border-t border-slate-200 p-4">
                  <p class="text-xs text-slate-500">
                    The YAML below is the live serialization of the form. Edits
                    here re-populate the visual fields when the YAML parses.
                  </p>
                  <textarea
                    id="rule-yaml"
                    value={yamlText()}
                    onInput={(e) => onYamlInput(e.currentTarget.value)}
                    spellcheck={false}
                    class="mt-2 block h-96 w-full rounded-md border border-slate-300 px-3 py-2 font-mono text-xs leading-relaxed focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                    aria-label="Rule YAML"
                  />
                  <Show when={yamlError()}>
                    <p class="mt-1 text-xs text-red-700">{yamlError()}</p>
                  </Show>
                  <Show when={untouched().length > 0 && yamlError() === null}>
                    <p class="mt-1 text-xs text-amber-700">
                      Some YAML keys have no visual control yet and will be
                      preserved on save: {untouched().join(", ")}.
                    </p>
                  </Show>
                </div>
              </Show>
            </div>

            <Show when={mode() === "edit"}>
              <div class="flex flex-wrap items-center gap-2 border-t border-slate-200 pt-4">
                <span class="text-sm font-medium text-slate-700">Actions:</span>
                <Show when={state().status !== "archived"}>
                  <button
                    type="button"
                    disabled={lifecycleBusy()}
                    onClick={onTogglePause}
                    class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                  >
                    {state().status === "paused" ? "Resume" : "Pause"}
                  </button>
                  <button
                    type="button"
                    disabled={lifecycleBusy()}
                    onClick={onArchive}
                    class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                  >
                    Archive
                  </button>
                </Show>
                <button
                  type="button"
                  disabled={lifecycleBusy()}
                  onClick={onDelete}
                  class="rounded-md border border-red-300 bg-white px-3 py-1.5 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-60"
                >
                  Delete
                </button>
              </div>
            </Show>

            <Show when={error()}>
              <div
                class="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700 whitespace-pre-wrap"
                role="alert"
              >
                {error()}
              </div>
            </Show>

            <div class="flex items-center justify-end gap-2 pt-2">
              <A
                href="/rules"
                class="rounded-md border border-slate-300 bg-white px-3 py-2 text-sm text-slate-700 hover:bg-slate-100"
              >
                Cancel
              </A>
              <button
                type="submit"
                disabled={
                  saving() || existing.loading || yamlError() !== null
                }
                class="rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white shadow hover:bg-indigo-500 disabled:opacity-60"
              >
                {saving() ? "Saving…" : "Save"}
              </button>
            </div>
          </form>
        </Show>
      </section>

      <ConfirmDialog
        open={pending() === "archive"}
        title="Archive rule"
        message={`Archive "${
          originalName() ?? "this rule"
        }"? It will stop running until you reactivate it.`}
        confirmLabel="Archive"
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
      <ConfirmDialog
        open={pending() === "delete"}
        title="Delete rule"
        message={`Delete "${
          originalName() ?? "this rule"
        }"? Decisions and run history will be removed. This cannot be undone.`}
        confirmLabel="Delete"
        destructive
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
    </main>
  );
};

export default RuleBuilder;

// Expose helpers for unit tests / future hooks.
export { deriveStateFromRule, MANAGED_OPTION_VALUE };
export type { TargetAlbumState };
