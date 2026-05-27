import {
  createMemo,
  createResource,
  createSignal,
  Show,
  Suspense,
  type Component,
} from "solid-js";
import { A, useNavigate } from "@solidjs/router";
import {
  deleteImmichKey,
  fetchImmichKey,
  getMe,
  pasteImmichKey,
  postLogout,
  type ApiResult,
  type ImmichKeyInfo,
  type Me,
} from "../lib/api";
import ConfirmDialog from "../components/ConfirmDialog";

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

  const onLogout = async () => {
    await postLogout();
    navigate("/login", { replace: true });
  };

  return (
    <main class="min-h-screen bg-slate-50">
      <header class="bg-white border-b border-slate-200">
        <div class="max-w-3xl mx-auto px-4 py-3 flex items-center justify-between">
          <div class="flex items-center gap-4">
            <A
              href="/"
              class="text-sm text-slate-500 hover:text-slate-700"
              aria-label="Back to dashboard"
            >
              ← Dashboard
            </A>
            <h1 class="text-lg font-semibold text-slate-900">Settings</h1>
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

      <section class="max-w-3xl mx-auto px-4 py-8 space-y-6">
        <section
          aria-labelledby="account-heading"
          class="rounded-lg border border-slate-200 bg-white p-5 shadow-sm"
        >
          <h2
            id="account-heading"
            class="text-sm font-semibold uppercase tracking-wide text-slate-500"
          >
            Account
          </h2>
          <Suspense fallback={<p class="mt-2 text-slate-500">Loading…</p>}>
            <Show
              when={me()}
              fallback={<p class="mt-2 text-slate-500">Not signed in.</p>}
            >
              {(user) => (
                <p class="mt-2 text-slate-700">
                  Signed in as{" "}
                  <span class="font-medium">{user().email}</span>
                  <Show when={user().display_name}>
                    {(name) => (
                      <span class="text-slate-500"> ({name()})</span>
                    )}
                  </Show>
                  .
                </p>
              )}
            </Show>
          </Suspense>
        </section>

        <section
          aria-labelledby="immich-heading"
          class="rounded-lg border border-slate-200 bg-white p-5 shadow-sm"
        >
          <h2
            id="immich-heading"
            class="text-sm font-semibold uppercase tracking-wide text-slate-500"
          >
            Immich account
          </h2>
          <p class="mt-1 text-sm text-slate-500">
            Connect your personal Immich API key so this account can browse its
            own people, albums, and assets.
          </p>

          <Suspense fallback={<p class="mt-4 text-slate-500">Loading…</p>}>
            <Show
              when={keyResource() !== undefined}
              fallback={<p class="mt-4 text-slate-500">Loading…</p>}
            >
              <Show when={connectedInfo()} keyed>
                {(info) => (
                  <div class="mt-4 space-y-3">
                    <dl class="text-sm text-slate-700 space-y-1">
                      <div class="flex gap-2">
                        <dt class="font-medium text-slate-500 w-40">
                          Immich URL
                        </dt>
                        <dd class="break-all">{info.base_url}</dd>
                      </div>
                      <div class="flex gap-2">
                        <dt class="font-medium text-slate-500 w-40">
                          Immich user
                        </dt>
                        <dd class="break-all">
                          {info.immich_user_id ?? "—"}
                        </dd>
                      </div>
                      <div class="flex gap-2">
                        <dt class="font-medium text-slate-500 w-40">
                          Last validated
                        </dt>
                        <dd>{formatTimestamp(info.last_validated_at)}</dd>
                      </div>
                    </dl>
                    <div class="flex flex-wrap gap-2 pt-2">
                      <button
                        type="button"
                        onClick={openReplaceForm}
                        class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
                      >
                        Replace key
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirmingDisconnect(true)}
                        class="rounded-md border border-red-300 bg-white px-3 py-1.5 text-sm text-red-700 hover:bg-red-50"
                      >
                        Disconnect
                      </button>
                    </div>
                  </div>
                )}
              </Show>

              <Show when={showEmptyForm()}>
                <form class="mt-4 space-y-4" onSubmit={onSubmit}>
                  <p class="text-sm text-slate-600">
                    Not connected to Immich. Paste an Immich API key minted in
                    Immich → Account settings → API Keys.
                  </p>
                  <div>
                    <label
                      class="block text-sm font-medium text-slate-700"
                      for="me-immich-url"
                    >
                      Immich base URL
                    </label>
                    <input
                      id="me-immich-url"
                      type="url"
                      required
                      placeholder="https://immich.example.com"
                      autocomplete="url"
                      class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                      value={baseUrl()}
                      onInput={(e) => setBaseUrl(e.currentTarget.value)}
                    />
                  </div>
                  <div>
                    <label
                      class="block text-sm font-medium text-slate-700"
                      for="me-immich-key"
                    >
                      Immich API key
                    </label>
                    <textarea
                      id="me-immich-key"
                      required
                      rows="3"
                      autocomplete="off"
                      spellcheck={false}
                      class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 font-mono text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                      value={apiKey()}
                      onInput={(e) => setApiKey(e.currentTarget.value)}
                    />
                    <p class="mt-1 text-xs text-slate-500">
                      Keys are encrypted at rest with AES-256-GCM. We never
                      display the key back to you after submission.
                    </p>
                  </div>

                  <Show when={formError()}>
                    <p class="text-sm text-red-600" role="alert">
                      {formError()}
                    </p>
                  </Show>

                  <button
                    type="submit"
                    disabled={submitting()}
                    class="rounded-md bg-indigo-600 px-4 py-2 text-sm font-medium text-white shadow hover:bg-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-1 disabled:opacity-60"
                  >
                    {submitting() ? "Connecting…" : "Connect Immich"}
                  </button>
                </form>
              </Show>

              <Show when={errorState()} keyed>
                {(state) => (
                  <div class="mt-4 space-y-3" role="alert">
                    <p class="text-sm text-red-600">
                      {humanKeyError(state.code)}
                    </p>
                    <button
                      type="button"
                      onClick={() => refetch()}
                      class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
                    >
                      Retry
                    </button>
                  </div>
                )}
              </Show>
            </Show>
          </Suspense>
        </section>
      </section>

      <ConfirmDialog
        open={confirmingDisconnect()}
        title="Disconnect Immich?"
        message="Your stored API key will be deleted. People, albums, and any rules referencing them will stop working until you reconnect."
        confirmLabel={disconnecting() ? "Disconnecting…" : "Disconnect Immich"}
        destructive
        onConfirm={onDisconnectConfirm}
        onCancel={() => setConfirmingDisconnect(false)}
      />
    </main>
  );
};

export default MeSettings;
