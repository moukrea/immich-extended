import {
  batch,
  createMemo,
  createResource,
  createSignal,
  For,
  Show,
  untrack,
  type Component,
} from "solid-js";
import { A, useNavigate, useParams } from "@solidjs/router";
import {
  createRule,
  deleteRule,
  fetchAlbums,
  getRule,
  MAX_POLL_INTERVAL_SECONDS,
  MIN_POLL_INTERVAL_SECONDS,
  postLogout,
  updateRule,
  type MeAlbum,
  type Rule,
  type RuleStatus,
} from "../../lib/api";
import { emptyMatch, type MatchExpr } from "../../lib/matchTree";
import {
  defaultRuleMeta,
  formStateToYamlV2,
  yamlToFormStateV2,
  type RuleMetaState,
} from "../../lib/ruleYamlV2";
import { BUILDER_DEFAULT_POLL_INTERVAL_SECONDS } from "../../lib/ruleYaml";
import BlockTreeEditor from "../../components/blocks/BlockTreeEditor";
import { PeopleProvider } from "../../components/PeopleContext";
import ConfirmDialog from "../../components/ConfirmDialog";
import { Button, Card, Field, Input, Select } from "../../components/ui";
import { humanRuleError } from "./errors";

const MANAGED_OPTION_VALUE = "__managed__";
type PendingLifecycle = "archive" | "delete";

function readFileAsText(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      if (typeof result === "string") resolve(result);
      else reject(new Error("FileReader returned a non-string result"));
    };
    reader.onerror = () =>
      reject(reader.error ?? new Error("Unknown FileReader error"));
    reader.readAsText(file);
  });
}

function deriveStateFromRule(rule: Rule): {
  meta: RuleMetaState;
  expr: MatchExpr;
} {
  const parsed = yamlToFormStateV2(rule.yaml_source);
  // Server payload is authoritative for status + poll_interval_seconds (the
  // latter is row-level, never round-tripped through YAML).
  return {
    meta: {
      ...parsed.meta,
      id: rule.id,
      status: rule.status,
      poll_interval_seconds: rule.poll_interval_seconds,
    },
    expr: parsed.expr,
  };
}

const RuleBuilderV2: Component = () => {
  const navigate = useNavigate();
  const params = useParams<{ id?: string }>();
  const mode = createMemo<"new" | "edit">(() => (params.id ? "edit" : "new"));

  const [meta, setMeta] = createSignal<RuleMetaState>(defaultRuleMeta());
  const [expr, setExpr] = createSignal<MatchExpr>(emptyMatch());
  const [yamlText, setYamlText] = createSignal<string>(
    formStateToYamlV2(defaultRuleMeta(), emptyMatch()),
  );
  const [yamlError, setYamlError] = createSignal<string | null>(null);
  const [untouched, setUntouched] = createSignal<string[]>([]);
  const [showAdvanced, setShowAdvanced] = createSignal(false);
  const [saving, setSaving] = createSignal(false);
  const [lifecycleBusy, setLifecycleBusy] = createSignal(false);
  const [pending, setPending] = createSignal<PendingLifecycle | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [originalName, setOriginalName] = createSignal<string | null>(null);
  const [loaded, setLoaded] = createSignal(untrack(() => mode() === "new"));
  const [copyStatus, setCopyStatus] = createSignal<"idle" | "copied" | "error">(
    "idle",
  );
  let fileInputRef: HTMLInputElement | undefined;

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
        setMeta(next.meta);
        setExpr(next.expr);
        setYamlText(formStateToYamlV2(next.meta, next.expr));
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

  const syncYaml = (nextMeta: RuleMetaState, nextExpr: MatchExpr) => {
    batch(() => {
      setMeta(nextMeta);
      setExpr(nextExpr);
      setYamlText(formStateToYamlV2(nextMeta, nextExpr));
      setYamlError(null);
    });
  };
  const mutateMeta = (
    updater: (prev: RuleMetaState) => RuleMetaState,
  ): void => {
    const next = updater(meta());
    syncYaml(next, expr());
  };
  const mutateExpr = (next: MatchExpr): void => {
    syncYaml(meta(), next);
  };

  const onYamlInput = (text: string) => {
    setYamlText(text);
    const parsed = yamlToFormStateV2(text);
    setYamlError(parsed.error);
    if (parsed.error !== null) return;
    batch(() => {
      // Preserve the id on edit (YAML may omit it).
      setMeta({ ...parsed.meta, id: meta().id ?? parsed.meta.id });
      setExpr(parsed.expr);
      setUntouched(parsed.untouched);
    });
  };

  const onNameInput = (value: string) =>
    mutateMeta((m) => ({ ...m, name: value }));

  const selectedAlbumValue = createMemo(() => {
    const t = meta().target;
    return t.kind === "managed" ? MANAGED_OPTION_VALUE : t.album_id;
  });
  const orphanAlbumId = createMemo<string>(() => {
    const t = meta().target;
    return t.kind === "existing" ? t.album_id : "";
  });
  const managedAlbumName = createMemo<string>(() => {
    const t = meta().target;
    return t.kind === "managed" ? t.name : "";
  });
  const showOrphanAlbumOption = createMemo(() => {
    const t = meta().target;
    if (t.kind !== "existing") return false;
    return !writableAlbums().some((a) => a.id === t.album_id);
  });

  const onAlbumSelect = (selected: string) =>
    mutateMeta((m) => {
      if (selected === MANAGED_OPTION_VALUE) {
        if (m.target.kind === "managed") return m;
        return { ...m, target: { kind: "managed", name: "", shared_with: [] } };
      }
      if (m.target.kind === "existing" && m.target.album_id === selected) {
        return m;
      }
      return { ...m, target: { kind: "existing", album_id: selected } };
    });
  const onManagedNameInput = (name: string) =>
    mutateMeta((m) => {
      if (m.target.kind !== "managed") return m;
      return { ...m, target: { ...m.target, name } };
    });

  const onPollIntervalInput = (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed === "") {
      mutateMeta((m) => ({
        ...m,
        poll_interval_seconds: BUILDER_DEFAULT_POLL_INTERVAL_SECONDS,
      }));
      return;
    }
    const parsed = Number(trimmed);
    if (!Number.isFinite(parsed)) return;
    mutateMeta((m) => ({ ...m, poll_interval_seconds: Math.round(parsed) }));
  };

  const exportSlug = createMemo(() => {
    const trimmed = meta().name.trim().toLowerCase();
    const slug = trimmed.replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
    return slug.length > 0 ? slug : "rule";
  });
  const exportHref = createMemo(
    () => `data:text/yaml;charset=utf-8,${encodeURIComponent(yamlText())}`,
  );
  const exportFilename = createMemo(() => `rule-${exportSlug()}.yaml`);
  const copyButtonLabel = createMemo(() => {
    const s = copyStatus();
    if (s === "copied") return "Copied!";
    if (s === "error") return "Copy failed";
    return "Copy YAML";
  });

  const onCopyYaml = async () => {
    const text = yamlText();
    try {
      const clip = navigator.clipboard;
      if (!clip || typeof clip.writeText !== "function") {
        throw new Error("Clipboard API unavailable");
      }
      await clip.writeText(text);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("error");
    }
    setTimeout(() => setCopyStatus("idle"), 1500);
  };

  const onImportClick = () => {
    if (!fileInputRef) return;
    fileInputRef.value = "";
    fileInputRef.click();
  };
  const onImportFile = async (event: Event) => {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = "";
    if (!file) return;
    let text: string;
    try {
      text = await readFileAsText(file);
    } catch (cause) {
      setError(
        `Failed to read file: ${
          cause instanceof Error ? cause.message : String(cause)
        }`,
      );
      return;
    }
    const isDirty = mode() === "edit" || meta().name.trim().length > 0;
    if (isDirty && typeof window.confirm === "function") {
      const ok = window.confirm(
        "Replace the current rule with the imported YAML?",
      );
      if (!ok) return;
    }
    setShowAdvanced(true);
    onYamlInput(text);
  };

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
    mutateMeta((m) => ({ ...m, status: next }));
  };

  const onTogglePause = () => {
    const current = meta().status;
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
    const pollInterval = meta().poll_interval_seconds;

    if (mode() === "new") {
      const result = await createRule({
        yaml_source: yaml,
        poll_interval_seconds: pollInterval,
      });
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
    const result = await updateRule(id, {
      yaml_source: yaml,
      poll_interval_seconds: pollInterval,
    });
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
    <main class="min-h-screen bg-immich-bg dark:bg-immich-dark-bg text-immich-fg dark:text-immich-dark-fg">
      <header class="border-b border-ui-border bg-white dark:bg-immich-dark-gray">
        <div class="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <div class="flex items-center gap-4">
            <A
              href="/rules"
              class="text-sm text-ui-muted dark:text-gray-400 hover:text-immich-primary"
            >
              ← Rules
            </A>
            <h1 class="text-lg font-semibold">{heading()}</h1>
            <Show when={mode() === "edit"}>
              <span
                class="rounded-full px-2 py-0.5 text-xs font-medium uppercase tracking-wide bg-slate-200 dark:bg-gray-700"
                aria-label="Rule status"
              >
                {meta().status}
              </span>
            </Show>
          </div>
          <div class="flex items-center gap-2">
            <Show when={mode() === "edit" && params.id}>
              <A
                href={`/rules/${params.id}/activity`}
                class="rounded-md border border-ui-border bg-white dark:bg-gray-700 px-3 py-1.5 text-sm hover:bg-slate-100 dark:hover:bg-gray-600"
              >
                Activity
              </A>
              <A
                href={`/rules/${params.id}/decisions`}
                class="rounded-md border border-ui-border bg-white dark:bg-gray-700 px-3 py-1.5 text-sm hover:bg-slate-100 dark:hover:bg-gray-600"
              >
                Decisions
              </A>
            </Show>
            <Button variant="secondary" size="sm" onClick={onLogout}>
              Sign out
            </Button>
          </div>
        </div>
      </header>

      <section class="max-w-3xl mx-auto px-4 py-8">
        <Show when={loaded()} fallback={<p class="text-ui-muted">Loading rule…</p>}>
          <form class="space-y-6" onSubmit={onSubmit}>
            <Card>
              <Field label="Name" for_="rule-name">
                <Input
                  id="rule-name"
                  type="text"
                  value={meta().name}
                  onInput={(e) => onNameInput(e.currentTarget.value)}
                  placeholder="Vacation 2024"
                />
              </Field>
            </Card>

            <Card>
              <Field
                label="Target album"
                for_="rule-album"
                help="Where matching assets are added."
              >
                <Select
                  id="rule-album"
                  value={selectedAlbumValue()}
                  onChange={(e) => onAlbumSelect(e.currentTarget.value)}
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
                </Select>
              </Field>
              <Show when={meta().target.kind === "managed"}>
                <div class="mt-3">
                  <Field label="Managed album name" for_="rule-album-name">
                    <Input
                      id="rule-album-name"
                      type="text"
                      value={managedAlbumName()}
                      onInput={(e) => onManagedNameInput(e.currentTarget.value)}
                      placeholder="Vacation 2024"
                    />
                  </Field>
                </div>
              </Show>
            </Card>

            <Card>
              <h2 class="text-sm font-semibold mb-2">Include media when</h2>
              <p class="text-xs text-ui-muted dark:text-gray-400 mb-3">
                Compose blocks with AND, OR, and NOT. Cheaper conditions are
                evaluated first; YOLO runs only when nothing cheaper rejects.
              </p>
              <PeopleProvider>
                <BlockTreeEditor expr={expr()} onChange={mutateExpr} />
              </PeopleProvider>
            </Card>

            <Card>
              <Field
                label="Poll interval (seconds)"
                for_="rule-poll-interval"
                help={`Minimum ${MIN_POLL_INTERVAL_SECONDS}s, maximum ${MAX_POLL_INTERVAL_SECONDS}s (1 day). 300s (5 min) suits most rules.`}
              >
                <Input
                  id="rule-poll-interval"
                  type="number"
                  min={MIN_POLL_INTERVAL_SECONDS}
                  max={MAX_POLL_INTERVAL_SECONDS}
                  step="1"
                  value={meta().poll_interval_seconds}
                  onInput={(e) => onPollIntervalInput(e.currentTarget.value)}
                  aria-label="Poll interval seconds"
                  class="w-40"
                />
              </Field>
            </Card>

            <Card padding="none">
              <button
                type="button"
                onClick={() => setShowAdvanced((s) => !s)}
                aria-expanded={showAdvanced()}
                aria-controls="rule-yaml-panel"
                class="w-full px-4 py-3 text-left text-sm font-semibold hover:bg-slate-50 dark:hover:bg-gray-700 rounded-2xl"
              >
                {showAdvanced() ? "▾" : "▸"} Advanced (YAML)
              </button>
              <Show when={showAdvanced()}>
                <div id="rule-yaml-panel" class="border-t border-ui-border p-4">
                  <div class="flex flex-wrap items-start justify-between gap-3">
                    <p class="text-xs text-ui-muted dark:text-gray-400 max-w-md">
                      The YAML below is the live serialization of the form.
                      Edits here re-populate the visual blocks when the YAML
                      parses.
                    </p>
                    <div class="flex flex-wrap items-center gap-2">
                      <a
                        href={exportHref()}
                        download={exportFilename()}
                        class="rounded-md border border-ui-border bg-white dark:bg-gray-700 px-3 py-1.5 text-xs font-medium hover:bg-slate-100 dark:hover:bg-gray-600"
                        aria-label="Export YAML as file"
                      >
                        Export
                      </a>
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={onCopyYaml}
                        aria-label="Copy YAML to clipboard"
                      >
                        {copyButtonLabel()}
                      </Button>
                      <Button
                        variant="secondary"
                        size="sm"
                        onClick={onImportClick}
                        aria-label="Import YAML from file"
                      >
                        Import
                      </Button>
                      <input
                        ref={fileInputRef}
                        type="file"
                        accept=".yaml,.yml,application/x-yaml,text/yaml,text/plain"
                        class="sr-only"
                        onChange={onImportFile}
                        aria-label="Import YAML file"
                        tabindex="-1"
                      />
                    </div>
                  </div>
                  <textarea
                    id="rule-yaml"
                    value={yamlText()}
                    onInput={(e) => onYamlInput(e.currentTarget.value)}
                    spellcheck={false}
                    class="mt-2 block h-96 w-full rounded-md border border-ui-border bg-white dark:bg-gray-800 px-3 py-2 font-mono text-xs leading-relaxed focus:border-immich-primary focus:outline-none focus:ring-1 focus:ring-immich-primary"
                    aria-label="Rule YAML"
                  />
                  <Show when={yamlError()}>
                    <p class="mt-1 text-xs text-ui-danger">{yamlError()}</p>
                  </Show>
                  <Show when={untouched().length > 0 && yamlError() === null}>
                    <p class="mt-1 text-xs text-amber-700 dark:text-amber-300">
                      Preserved-but-not-shown YAML keys: {untouched().join(", ")}.
                    </p>
                  </Show>
                </div>
              </Show>
            </Card>

            <Show when={mode() === "edit"}>
              <div class="flex flex-wrap items-center gap-2 border-t border-ui-border pt-4">
                <span class="text-sm font-medium">Actions:</span>
                <Show when={meta().status !== "archived"}>
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={lifecycleBusy()}
                    onClick={onTogglePause}
                  >
                    {meta().status === "paused" ? "Resume" : "Pause"}
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={lifecycleBusy()}
                    onClick={onArchive}
                  >
                    Archive
                  </Button>
                </Show>
                <Button
                  variant="destructive"
                  size="sm"
                  disabled={lifecycleBusy()}
                  onClick={onDelete}
                >
                  Delete
                </Button>
              </div>
            </Show>

            <Show when={error()}>
              <div
                class="rounded-md border border-ui-danger/30 bg-red-50 dark:bg-red-900/20 px-3 py-2 text-sm text-ui-danger whitespace-pre-wrap"
                role="alert"
              >
                {error()}
              </div>
            </Show>

            <div class="flex items-center justify-end gap-2 pt-2">
              <A
                href="/rules"
                class="rounded-md border border-ui-border bg-white dark:bg-gray-700 px-3 py-2 text-sm hover:bg-slate-100 dark:hover:bg-gray-600"
              >
                Cancel
              </A>
              <Button
                type="submit"
                disabled={saving() || existing.loading || yamlError() !== null}
                loading={saving()}
              >
                {saving() ? "Saving…" : "Save"}
              </Button>
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

export default RuleBuilderV2;
export { deriveStateFromRule, MANAGED_OPTION_VALUE };
