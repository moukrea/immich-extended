import { createResource, Show, type Component } from "solid-js";
import { useNavigate } from "@solidjs/router";
import { getMe, postLogout } from "../lib/api";

const Dashboard: Component = () => {
  const navigate = useNavigate();
  const [me] = createResource(async () => {
    const result = await getMe();
    if (!result.ok) {
      if (result.status === 401) {
        navigate("/login", { replace: true });
      }
      return null;
    }
    return result.data;
  });

  const onLogout = async () => {
    await postLogout();
    navigate("/login", { replace: true });
  };

  return (
    <main class="min-h-screen bg-slate-50">
      <header class="bg-white border-b border-slate-200">
        <div class="max-w-5xl mx-auto px-4 py-3 flex items-center justify-between">
          <h1 class="text-lg font-semibold text-slate-900">immich-extended</h1>
          <button
            type="button"
            onClick={onLogout}
            class="rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-700 hover:bg-slate-100"
          >
            Sign out
          </button>
        </div>
      </header>

      <section class="max-w-5xl mx-auto px-4 py-8">
        <Show when={me()} fallback={<p class="text-slate-500">Loading…</p>}>
          {(user) => (
            <p class="text-slate-700">
              Logged in as{" "}
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
      </section>
    </main>
  );
};

export default Dashboard;
