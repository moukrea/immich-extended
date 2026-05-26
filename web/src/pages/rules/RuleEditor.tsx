import {
  createMemo,
  createResource,
  createSignal,
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
  getRule,
  postLogout,
  updateRule,
  type Rule,
  type RuleStatus,
} from "../../lib/api";
import {
  DEFAULT_LOCATION,
  readLocation,
  writeLocation,
} from "../../lib/yamlLocation";
import ConfirmDialog from "../../components/ConfirmDialog";
import { humanRuleError } from "./errors";

const MapPicker = lazy(() => import("../../components/MapPicker"));

type PendingLifecycle = "archive" | "delete";

const PLACEHOLDER_YAML = `# Paste a rule definition. Example:
name: Vacation 2024
status: active
match:
  date:
    from: 2024-06-01
    to: 2024-09-15
target_album:
  type: managed
  name: Vacation 2024
`;

const RuleEditor: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id?: string }>();
  const mode = createMemo<"new" | "edit">(() => (params.id ? "edit" : "new"));

  const [yamlSource, setYamlSource] = createSignal("");
  const [status, setStatus] = createSignal<RuleStatus>("active");
  const [error, setError] = createSignal<string | null>(null);
  const [saving, setSaving] = createSignal(false);
  const [lifecycleBusy, setLifecycleBusy] = createSignal(false);
  const [loaded, setLoaded] = createSignal(untrack(() => mode() === "new"));
  const [originalName, setOriginalName] = createSignal<string | null>(null);
  const [showMap, setShowMap] = createSignal(false);
  const [pending, setPending] = createSignal<PendingLifecycle | null>(null);

  const pickerLocation = createMemo(
    () => readLocation(yamlSource()) ?? DEFAULT_LOCATION,
  );

  const onPickerChange = (center: [number, number], radiusKm: number) => {
    setYamlSource(writeLocation(yamlSource(), { center, radiusKm }));
  };

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
      setYamlSource(result.data.yaml_source);
      setStatus(result.data.status);
      setOriginalName(result.data.name);
      setLoaded(true);
      return result.data;
    },
  );

  const heading = createMemo(() => {
    if (mode() === "new") return "New rule";
    const name = originalName();
    return name ? `Edit "${name}"` : "Edit rule";
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
    setStatus(next);
  };

  const onTogglePause = () => {
    const next: RuleStatus = status() === "paused" ? "active" : "paused";
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
    const yaml = yamlSource();
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
    const original = existing();
    const payload: { yaml_source?: string; status?: RuleStatus } = {
      yaml_source: yaml,
    };
    if (original && status() !== original.status) {
      payload.status = status();
    }
    const result = await updateRule(id, payload);
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
            <A
              href="/rules"
              class="text-sm text-slate-500 hover:text-slate-700"
            >
              ← Rules
            </A>
            <h1 class="text-lg font-semibold text-slate-900">{heading()}</h1>
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

      <section class="max-w-5xl mx-auto px-4 py-8">
        <Show
          when={loaded()}
          fallback={<p class="text-slate-500">Loading rule…</p>}
        >
          <form class="space-y-4" onSubmit={onSubmit}>
            <div>
              <label
                class="block text-sm font-medium text-slate-700"
                for="rule-yaml"
              >
                Rule YAML
              </label>
              <p class="mt-1 text-xs text-slate-500">
                See the documentation for the full schema. At minimum, a rule
                needs a <code>name</code>, a non-empty <code>match</code>{" "}
                section, and a <code>target_album</code>.
              </p>
              <textarea
                id="rule-yaml"
                class="mt-2 block h-96 w-full rounded-md border border-slate-300 px-3 py-2 font-mono text-sm leading-relaxed focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                spellcheck={false}
                placeholder={PLACEHOLDER_YAML}
                value={yamlSource()}
                onInput={(e) => setYamlSource(e.currentTarget.value)}
              />
            </div>

            <div>
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
                The picker writes the <code>match.location</code> block in
                the YAML above. The YAML stays the source of truth.
              </p>
              <Show when={showMap()}>
                <div id="rule-map-picker" class="mt-3">
                  <Suspense
                    fallback={
                      <p class="text-sm text-slate-500">Loading map…</p>
                    }
                  >
                    <MapPicker
                      center={pickerLocation().center}
                      radiusKm={pickerLocation().radiusKm}
                      onChange={onPickerChange}
                    />
                  </Suspense>
                </div>
              </Show>
            </div>

            <Show when={mode() === "edit"}>
              <div>
                <label
                  class="block text-sm font-medium text-slate-700"
                  for="rule-status"
                >
                  Status
                </label>
                <select
                  id="rule-status"
                  class="mt-1 w-full max-w-xs rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                  value={status()}
                  onChange={(e) =>
                    setStatus(e.currentTarget.value as RuleStatus)
                  }
                >
                  <option value="active">active</option>
                  <option value="paused">paused</option>
                  <option value="archived">archived</option>
                </select>
              </div>

              <div class="flex flex-wrap items-center gap-2 border-t border-slate-200 pt-4">
                <span class="text-sm font-medium text-slate-700">Actions:</span>
                <Show when={status() !== "archived"}>
                  <button
                    type="button"
                    disabled={lifecycleBusy()}
                    onClick={onTogglePause}
                    class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                  >
                    {status() === "paused" ? "Resume" : "Pause"}
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
                disabled={saving() || existing.loading}
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
        message={`Archive "${originalName() ?? "this rule"}"? It will stop running until you reactivate it.`}
        confirmLabel="Archive"
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
      <ConfirmDialog
        open={pending() === "delete"}
        title="Delete rule"
        message={`Delete "${originalName() ?? "this rule"}"? Decisions and run history will be removed. This cannot be undone.`}
        confirmLabel="Delete"
        destructive
        onConfirm={onConfirmPending}
        onCancel={onCancelPending}
      />
    </main>
  );
};

export default RuleEditor;
