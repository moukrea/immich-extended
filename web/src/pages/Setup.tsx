import { createMemo, createSignal, Show, type Component } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { postSetupInitial } from "../lib/api";
import { Button, Card, Field, Input } from "../components/ui";

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
    <main class="min-h-screen flex items-center justify-center bg-immich-bg text-immich-fg dark:bg-immich-dark-bg dark:text-immich-dark-fg px-4 py-10">
      <div class="w-full max-w-md">
        <div class="mb-6 text-center">
          <h1 class="text-2xl font-semibold tracking-tight">
            First-time setup
          </h1>
          <p class="mt-1 text-sm text-ui-muted">
            Create the initial admin account.
          </p>
        </div>
        <Card padding="lg">
          <form class="space-y-4" onSubmit={onSubmit}>
            <Field label="Email" for_="setup-email" required>
              <Input
                id="setup-email"
                type="email"
                required
                autocomplete="username"
                value={email()}
                onInput={(e) => setEmail(e.currentTarget.value)}
              />
            </Field>
            <Field label="Password" for_="setup-password" required>
              <Input
                id="setup-password"
                type="password"
                required
                autocomplete="new-password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
              />
            </Field>
            <Field
              label="Display name"
              for_="setup-display-name"
              help="Optional — shown next to your email."
            >
              <Input
                id="setup-display-name"
                type="text"
                value={displayName()}
                onInput={(e) => setDisplayName(e.currentTarget.value)}
              />
            </Field>

            <fieldset class="border-t border-ui-border pt-4 dark:border-gray-700">
              <legend class="px-1 text-sm font-medium text-immich-fg dark:text-immich-dark-fg">
                Connect Immich{" "}
                <span class="text-ui-muted">(optional)</span>
              </legend>
              <p class="mt-1 text-xs text-ui-muted">
                Provide both fields, or leave both blank.
              </p>
              <div class="mt-3 space-y-4">
                <Field label="Immich base URL" for_="setup-immich-url">
                  <Input
                    id="setup-immich-url"
                    type="url"
                    placeholder="https://immich.example.com"
                    required={immichPartial() === "url"}
                    value={immichUrl()}
                    onInput={(e) => setImmichUrl(e.currentTarget.value)}
                  />
                </Field>
                <Field label="Immich API key" for_="setup-immich-key">
                  <Input
                    id="setup-immich-key"
                    type="password"
                    autocomplete="off"
                    required={immichPartial() === "key"}
                    value={immichKey()}
                    onInput={(e) => setImmichKey(e.currentTarget.value)}
                  />
                </Field>
              </div>
            </fieldset>

            <Show when={error()}>
              <p class="text-sm text-ui-danger" role="alert">
                {error()}
              </p>
            </Show>

            <Button
              type="submit"
              class="w-full"
              loading={submitting()}
              disabled={submitting()}
            >
              {submitting() ? "Creating account…" : "Create admin account"}
            </Button>
          </form>
        </Card>
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
