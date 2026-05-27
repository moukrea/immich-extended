import { createSignal, Show, type Accessor, type Component } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { postLogin } from "../lib/api";

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
    <main class="min-h-screen flex items-center justify-center bg-slate-50 px-4">
      <div class="w-full max-w-sm bg-white shadow rounded-lg p-6">
        <h1 class="text-xl font-semibold text-slate-900">Sign in</h1>
        <p class="mt-1 text-sm text-slate-500">
          Local account, or SSO if enabled.
        </p>

        <form class="mt-5 space-y-4" onSubmit={onSubmit}>
          <div>
            <label
              class="block text-sm font-medium text-slate-700"
              for="login-email"
            >
              Email
            </label>
            <input
              id="login-email"
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
              for="login-password"
            >
              Password
            </label>
            <input
              id="login-password"
              type="password"
              required
              autocomplete="current-password"
              class="mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm focus:border-indigo-500 focus:outline-none focus:ring-1 focus:ring-indigo-500"
              value={password()}
              onInput={(e) => setPassword(e.currentTarget.value)}
            />
          </div>

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
            {submitting() ? "Signing in…" : "Sign in"}
          </button>
        </form>

        <Show when={props.oidcEnabled()}>
          <div class="mt-4 border-t border-slate-200 pt-4">
            <a
              href="/api/v1/auth/oidc/login"
              rel="external"
              class="block w-full rounded-md bg-slate-900 px-3 py-2 text-sm font-medium text-white text-center hover:bg-slate-800"
            >
              Sign in with SSO
            </a>
          </div>
        </Show>
      </div>
    </main>
  );
};

export default Login;
