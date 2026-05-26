import { createMemo, createSignal, Show, type Component } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { postSetupInitial } from "../lib/api";

const Setup: Component = () => {
  const navigate = useNavigate();
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [displayName, setDisplayName] = createSignal("");
  const [immichUrl, setImmichUrl] = createSignal("");
  const [immichKey, setImmichKey] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  const immichPartial = createMemo(() => {
    const hasUrl = immichUrl().trim().length > 0;
    const hasKey = immichKey().trim().length > 0;
    if (hasUrl && !hasKey) return "key";
    if (hasKey && !hasUrl) return "url";
    return null;
  });

  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (submitting()) return;
    if (immichPartial() !== null) {
      setError("Immich URL and API key must both be provided, or neither.");
      return;
    }
    setSubmitting(true);
    setError(null);

    const payload: Parameters<typeof postSetupInitial>[0] = {
      email: email().trim(),
      password: password(),
    };
    const dn = displayName().trim();
    if (dn.length > 0) payload.display_name = dn;
    const url = immichUrl().trim();
    const key = immichKey().trim();
    if (url.length > 0 && key.length > 0) {
      payload.immich_base_url = url;
      payload.immich_api_key = key;
    }

    const result = await postSetupInitial(payload);
    setSubmitting(false);
    if (result.ok) {
      navigate("/", { replace: true });
      return;
    }
    if (result.status === 409) {
      navigate("/login", { replace: true });
      return;
    }
    setError(humanError(result.error.error, result.error.field));
  };

  return (
    <main class="min-h-screen flex items-center justify-center bg-slate-50 px-4 py-10">
      <div class="w-full max-w-md bg-white shadow rounded-lg p-6">
        <h1 class="text-xl font-semibold text-slate-900">First-time setup</h1>
        <p class="mt-1 text-sm text-slate-500">
          Create the initial admin account.
        </p>

        <form class="mt-5 space-y-4" onSubmit={onSubmit}>
          <div>
            <label
              class="block text-sm font-medium text-slate-700"
              for="setup-email"
            >
              Email
            </label>
            <input
              id="setup-email"
              type="email"
              required
              autocomplete="username"
              class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              value={email()}
              onInput={(e) => setEmail(e.currentTarget.value)}
            />
          </div>
          <div>
            <label
              class="block text-sm font-medium text-slate-700"
              for="setup-password"
            >
              Password
            </label>
            <input
              id="setup-password"
              type="password"
              required
              autocomplete="new-password"
              class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
            />
          </div>
          <div>
            <label
              class="block text-sm font-medium text-slate-700"
              for="setup-display-name"
            >
              Display name <span class="text-slate-400">(optional)</span>
            </label>
            <input
              id="setup-display-name"
              type="text"
              class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              value={displayName()}
              onInput={(e) => setDisplayName(e.currentTarget.value)}
            />
          </div>

          <fieldset class="border-t border-slate-200 pt-4">
            <legend class="text-sm font-medium text-slate-700">
              Connect Immich <span class="text-slate-400">(optional)</span>
            </legend>
            <p class="mt-1 text-xs text-slate-500">
              Provide both fields, or leave both blank.
            </p>
            <div class="mt-3 space-y-3">
              <div>
                <label
                  class="block text-sm font-medium text-slate-700"
                  for="setup-immich-url"
                >
                  Immich base URL
                </label>
                <input
                  id="setup-immich-url"
                  type="url"
                  placeholder="https://immich.example.com"
                  required={immichPartial() === "url"}
                  class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                  value={immichUrl()}
                  onInput={(e) => setImmichUrl(e.currentTarget.value)}
                />
              </div>
              <div>
                <label
                  class="block text-sm font-medium text-slate-700"
                  for="setup-immich-key"
                >
                  Immich API key
                </label>
                <input
                  id="setup-immich-key"
                  type="password"
                  autocomplete="off"
                  required={immichPartial() === "key"}
                  class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
                  value={immichKey()}
                  onInput={(e) => setImmichKey(e.currentTarget.value)}
                />
              </div>
            </div>
          </fieldset>

          <Show when={error()}>
            <p class="text-red-600 text-sm" role="alert">
              {error()}
            </p>
          </Show>

          <button
            type="submit"
            disabled={submitting()}
            class="w-full rounded-md bg-indigo-600 px-3 py-2 text-sm font-medium text-white shadow hover:bg-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-1 disabled:opacity-60"
          >
            {submitting() ? "Creating account…" : "Create admin account"}
          </button>
        </form>
      </div>
    </main>
  );
};

function humanError(code: string, field: string | undefined): string {
  switch (code) {
    case "invalid_request":
      return field
        ? `Missing or invalid field: ${field}.`
        : "Invalid request.";
    case "invalid_base_url":
      return "Immich base URL is not a valid URL.";
    case "invalid_immich_key":
      return "Immich rejected the API key.";
    case "upstream_unreachable":
      return "Could not reach the Immich server.";
    default:
      return `Setup failed (${code}).`;
  }
}

export default Setup;
