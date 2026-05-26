import { createResource, Show, type Component } from "solid-js";
import { A, useNavigate } from "@solidjs/router";
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

      <section class="max-w-5xl mx-auto px-4 py-8 space-y-6">
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

        <div class="grid grid-cols-1 gap-4 sm:grid-cols-2">
          <A
            href="/rules"
            class="block rounded-lg border border-slate-200 bg-white p-5 shadow-sm hover:border-indigo-300 hover:shadow-md transition-shadow"
          >
            <h2 class="text-base font-semibold text-slate-900">Rules</h2>
            <p class="mt-1 text-sm text-slate-500">
              Author and manage automation rules that decide which assets land
              in which Immich albums.
            </p>
            <span class="mt-3 inline-block text-sm font-medium text-indigo-600">
              Open rules →
            </span>
          </A>
        </div>
      </section>
    </main>
  );
};

export default Dashboard;
