import { createSignal, Show, type Accessor, type Component } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { postLogin } from "../lib/api";
import { Button, Card, Field, Input } from "../components/ui";

interface LoginProps {
  oidcEnabled: Accessor<boolean>;
}

const Login: Component<LoginProps> = (props) => {
  const navigate = useNavigate();
  const [email, setEmail] = createSignal("");
  const [password, setPassword] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  const [submitting, setSubmitting] = createSignal(false);

  const onSubmit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (submitting()) return;
    setSubmitting(true);
    setError(null);
    const result = await postLogin(email(), password());
    setSubmitting(false);
    if (result.ok) {
      navigate("/", { replace: true });
      return;
    }
    if (result.status === 401) {
      setError("Invalid credentials.");
    } else if (result.status === 0) {
      setError("Network error — is the server running?");
    } else {
      setError(`Login failed (${result.error.error}).`);
    }
  };

  return (
    <main class="min-h-screen flex items-center justify-center bg-immich-bg text-immich-fg dark:bg-immich-dark-bg dark:text-immich-dark-fg px-4">
      <div class="w-full max-w-sm">
        <div class="mb-6 text-center">
          <h1 class="text-2xl font-semibold tracking-tight">
            immich-extended
          </h1>
          <p class="mt-1 text-sm text-ui-muted">
            Sign in to manage your rules.
          </p>
        </div>
        <Card padding="lg">
          <form class="space-y-4" onSubmit={onSubmit}>
            <Field label="Email" for_="login-email" required>
              <Input
                id="login-email"
                type="email"
                required
                autocomplete="username"
                value={email()}
                onInput={(e) => setEmail(e.currentTarget.value)}
              />
            </Field>
            <Field label="Password" for_="login-password" required>
              <Input
                id="login-password"
                type="password"
                required
                autocomplete="current-password"
                value={password()}
                onInput={(e) => setPassword(e.currentTarget.value)}
              />
            </Field>

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
              {submitting() ? "Signing in…" : "Sign in"}
            </Button>
          </form>

          <Show when={props.oidcEnabled()}>
            <div class="mt-5 border-t border-ui-border pt-5 dark:border-gray-700">
              {/* rel="external" opts this link out of SolidJS Router interception
                  so the browser performs a real navigation to the server-rendered
                  /api/v1/auth/oidc/login redirect. Without it the click is
                  hijacked into client-side routing and lands on NotFound. */}
              <a
                href="/api/v1/auth/oidc/login"
                rel="external"
                class="inline-flex w-full items-center justify-center gap-2 rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-white shadow-md transition ease-immich duration-150 hover:bg-gray-700 focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary dark:bg-gray-700 dark:hover:bg-gray-600"
              >
                Sign in with SSO
              </a>
            </div>
          </Show>
        </Card>
      </div>
    </main>
  );
};

export default Login;
