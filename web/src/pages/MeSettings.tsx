import {
  createMemo,
  createResource,
  createSignal,
  Show,
  Suspense,
  type Component,
} from "solid-js";
import { useNavigate } from "@solidjs/router";
import {
  deleteImmichKey,
  fetchImmichKey,
  getMe,
  pasteImmichKey,
  type ApiResult,
  type ImmichKeyInfo,
  type Me,
} from "../lib/api";
import ConfirmDialog from "../components/ConfirmDialog";
import { Button, Card, Field, Input, Label } from "../components/ui";

type KeyState =
  | { kind: "loading" }
  | { kind: "connected"; info: ImmichKeyInfo }
  | { kind: "empty" }
  | { kind: "error"; status: number; code: string };

function resultToKeyState(
  result: ApiResult<ImmichKeyInfo>,
  fallbackCode = "internal_error",
): KeyState {
  if (result.ok) return { kind: "connected", info: result.data };
  if (result.status === 404) return { kind: "empty" };
  return {
    kind: "error",
    status: result.status,
    code: result.error.error ?? fallbackCode,
  };
}

function formatTimestamp(unixSeconds: number): string {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) return "—";
  return new Date(unixSeconds * 1000).toLocaleString();
}

function humanKeyError(code: string): string {
  switch (code) {
    case "invalid_request":
      return "Both fields are required.";
    case "invalid_base_url":
      return "Immich base URL is not a valid URL.";
    case "invalid_immich_key":
      return "Immich rejected the API key. Double-check that you pasted the full key for the right Immich account.";
    case "upstream_unreachable":
      return "Could not reach the Immich server. Check the base URL and that Immich is online.";
    case "internal_error":
      return "Something went wrong on our side. Please retry; if it keeps failing, contact your administrator.";
    case "network_error":
      return "Network error reaching immich-extended. Check your connection and retry.";
    default:
      return `Connection failed (${code}).`;
  }
}

const MeSettings: Component = () => {
  const navigate = useNavigate();

  const [me] = createResource<Me | null>(async () => {
    const result = await getMe();
    if (!result.ok) {
      if (result.status === 401) navigate("/login", { replace: true });
      return null;
    }
    return result.data;
  });

  const [keyState, setKeyState] = createSignal<KeyState>({ kind: "loading" });
  const [keyResource, { refetch }] = createResource<KeyState>(async () => {
    const result = await fetchImmichKey();
    const next = resultToKeyState(result);
    setKeyState(next);
    return next;
  });

  const [baseUrl, setBaseUrl] = createSignal("");
  const [apiKey, setApiKey] = createSignal("");
  const [formError, setFormError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);
  const [confirmingDisconnect, setConfirmingDisconnect] = createSignal(false);
  const [disconnecting, setDisconnecting] = createSignal(false);

  const connectedInfo = createMemo(() => {
    const s = keyState();
    return s.kind === "connected" ? s.info : null;
  });
  const errorState = createMemo(() => {
    const s = keyState();
    return s.kind === "error" ? s : null;
  });
  const showEmptyForm = createMemo(() => keyState().kind === "empty");

  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (submitting()) return;
    const base = baseUrl().trim();
    const key = apiKey();
    if (base.length === 0 || key.length === 0) {
      setFormError("Both fields are required.");
      return;
    }
    setSubmitting(true);
    setFormError(null);
    const result = await pasteImmichKey({ base_url: base, api_key: key });
    setSubmitting(false);
    if (result.ok) {
      setApiKey("");
      setKeyState({ kind: "connected", info: result.data });
      await refetch();
      return;
    }
    setFormError(humanKeyError(result.error.error ?? "internal_error"));
  };

  const openReplaceForm = () => {
    const current = keyState();
    setBaseUrl(current.kind === "connected" ? current.info.base_url : "");
    setApiKey("");
    setFormError(null);
    setKeyState({ kind: "empty" });
  };

  const onDisconnectConfirm = async () => {
    if (disconnecting()) return;
    setDisconnecting(true);
    const result = await deleteImmichKey();
    setDisconnecting(false);
    setConfirmingDisconnect(false);
    if (result.ok) {
      setBaseUrl("");
      setApiKey("");
      setFormError(null);
      setKeyState({ kind: "empty" });
      await refetch();
      return;
    }
    setFormError(humanKeyError(result.error.error ?? "internal_error"));
  };

  return (
    <section class="max-w-3xl mx-auto space-y-6">
      <header class="mb-2">
        <h1 class="text-2xl font-semibold tracking-tight">Settings</h1>
        <p class="mt-1 text-sm text-ui-muted">
          Account info and your Immich connection.
        </p>
      </header>

      <Card padding="lg" aria-labelledby="account-heading">
        <h2
          id="account-heading"
          class="text-xs font-semibold uppercase tracking-wide text-ui-muted"
        >
          Account
        </h2>
        <Suspense fallback={<p class="mt-2 text-ui-muted">Loading…</p>}>
          <Show
            when={me()}
            fallback={<p class="mt-2 text-ui-muted">Not signed in.</p>}
          >
            {(user) => (
              <p class="mt-2 text-immich-fg dark:text-immich-dark-fg">
                Signed in as{" "}
                <span class="font-medium">{user().email}</span>
                <Show when={user().display_name}>
                  {(name) => (
                    <span class="text-ui-muted"> ({name()})</span>
                  )}
                </Show>
                .
              </p>
            )}
          </Show>
        </Suspense>
      </Card>

      <Card padding="lg" aria-labelledby="immich-heading">
        <h2
          id="immich-heading"
          class="text-xs font-semibold uppercase tracking-wide text-ui-muted"
        >
          Immich account
        </h2>
        <p class="mt-1 text-sm text-ui-muted">
          Connect your personal Immich API key so this account can browse its
          own people, albums, and assets.
        </p>

        <Suspense fallback={<p class="mt-4 text-ui-muted">Loading…</p>}>
          <Show
            when={keyResource() !== undefined}
            fallback={<p class="mt-4 text-ui-muted">Loading…</p>}
          >
            <Show when={connectedInfo()} keyed>
              {(info) => (
                <div class="mt-4 space-y-3">
                  <dl class="text-sm text-immich-fg dark:text-immich-dark-fg space-y-1">
                    <div class="flex gap-2">
                      <dt class="font-medium text-ui-muted w-40">
                        Immich URL
                      </dt>
                      <dd class="break-all">{info.base_url}</dd>
                    </div>
                    <div class="flex gap-2">
                      <dt class="font-medium text-ui-muted w-40">
                        Immich user
                      </dt>
                      <dd class="break-all">
                        {info.immich_user_id ?? "—"}
                      </dd>
                    </div>
                    <div class="flex gap-2">
                      <dt class="font-medium text-ui-muted w-40">
                        Last validated
                      </dt>
                      <dd>{formatTimestamp(info.last_validated_at)}</dd>
                    </div>
                  </dl>
                  <div class="flex flex-wrap gap-2 pt-2">
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      onClick={openReplaceForm}
                    >
                      Replace key
                    </Button>
                    <Button
                      type="button"
                      variant="destructive"
                      size="sm"
                      onClick={() => setConfirmingDisconnect(true)}
                    >
                      Disconnect
                    </Button>
                  </div>
                </div>
              )}
            </Show>

            <Show when={showEmptyForm()}>
              <form class="mt-4 space-y-4" onSubmit={onSubmit}>
                <p class="text-sm text-immich-fg dark:text-immich-dark-fg">
                  Not connected to Immich. Paste an Immich API key minted in
                  Immich → Account settings → API Keys.
                </p>
                <Field label="Immich base URL" for_="me-immich-url">
                  <Input
                    id="me-immich-url"
                    type="url"
                    required
                    placeholder="https://immich.example.com"
                    autocomplete="url"
                    value={baseUrl()}
                    onInput={(e) => setBaseUrl(e.currentTarget.value)}
                  />
                </Field>
                <div class="space-y-1.5">
                  <Label for="me-immich-key">Immich API key</Label>
                  <textarea
                    id="me-immich-key"
                    required
                    rows="3"
                    autocomplete="off"
                    spellcheck={false}
                    class="w-full rounded-xl bg-slate-200 dark:bg-gray-600 text-sm text-immich-fg dark:text-immich-dark-fg placeholder:text-gray-500 dark:placeholder:text-gray-300 px-3 py-3 border border-transparent font-mono transition ease-immich duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary dark:focus-visible:ring-immich-dark-primary"
                    value={apiKey()}
                    onInput={(e) => setApiKey(e.currentTarget.value)}
                  />
                  <p class="text-xs text-ui-muted">
                    Keys are encrypted at rest with AES-256-GCM. We never
                    display the key back to you after submission.
                  </p>
                </div>

                <Show when={formError()}>
                  <p class="text-sm text-ui-danger" role="alert">
                    {formError()}
                  </p>
                </Show>

                <Button
                  type="submit"
                  loading={submitting()}
                  disabled={submitting()}
                >
                  {submitting() ? "Connecting…" : "Connect Immich"}
                </Button>
              </form>
            </Show>

            <Show when={errorState()} keyed>
              {(state) => (
                <div class="mt-4 space-y-3" role="alert">
                  <p class="text-sm text-ui-danger">
                    {humanKeyError(state.code)}
                  </p>
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    onClick={() => refetch()}
                  >
                    Retry
                  </Button>
                </div>
              )}
            </Show>
          </Show>
        </Suspense>
      </Card>

      <ConfirmDialog
        open={confirmingDisconnect()}
        title="Disconnect Immich?"
        message="Your stored API key will be deleted. People, albums, and any rules referencing them will stop working until you reconnect."
        confirmLabel={disconnecting() ? "Disconnecting…" : "Disconnect Immich"}
        destructive
        onConfirm={onDisconnectConfirm}
        onCancel={() => setConfirmingDisconnect(false)}
      />
    </section>
  );
};

export default MeSettings;
