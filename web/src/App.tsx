import { Router, Route, useNavigate } from "@solidjs/router";
import {
  createContext,
  createSignal,
  onMount,
  useContext,
  type Accessor,
  type Component,
  type JSX,
} from "solid-js";
import { getMe, getSetupState, type SetupState } from "./lib/api";
import { decideBootstrapNavigation } from "./lib/route";
import Login from "./pages/Login";
import Setup from "./pages/Setup";
import Dashboard from "./pages/Dashboard";
import RulesList from "./pages/rules/RulesList";
import RuleEditor from "./pages/rules/RuleEditor";
import RuleDecisions from "./pages/rules/RuleDecisions";

interface BootstrapValue {
  setupState: Accessor<SetupState | null>;
  ready: Accessor<boolean>;
}

const BootstrapContext = createContext<BootstrapValue>({
  setupState: () => null,
  ready: () => false,
});

export function useBootstrap(): BootstrapValue {
  return useContext(BootstrapContext);
}

const Bootstrap: Component<{ children: JSX.Element }> = (props) => {
  const navigate = useNavigate();
  const [ready, setReady] = createSignal(false);
  const [setupState, setSetupState] = createSignal<SetupState | null>(null);

  onMount(async () => {
    const [stateRes, meRes] = await Promise.all([getSetupState(), getMe()]);
    const state: SetupState = stateRes.ok
      ? stateRes.data
      : { needs_setup: false, oidc_enabled: false };
    const authed = meRes.ok;
    setSetupState(state);
    const target = decideBootstrapNavigation(
      { needs_setup: state.needs_setup },
      { authed },
      window.location.pathname,
    );
    if (target !== null) {
      navigate(target, { replace: true });
    }
    setReady(true);
  });

  return (
    <BootstrapContext.Provider value={{ setupState, ready }}>
      {props.children}
    </BootstrapContext.Provider>
  );
};

const LoginRoute: Component = () => {
  const { setupState } = useBootstrap();
  return <Login oidcEnabled={() => setupState()?.oidc_enabled ?? false} />;
};

const NotFound: Component = () => (
  <main class="min-h-screen flex items-center justify-center">
    <div class="text-center">
      <h1 class="text-2xl font-semibold">Not found</h1>
      <p class="mt-2 text-slate-600">
        <a class="text-indigo-600 hover:underline" href="/">
          Back home
        </a>
      </p>
    </div>
  </main>
);

const App: Component = () => (
  <Router root={(props) => <Bootstrap>{props.children}</Bootstrap>}>
    <Route path="/login" component={LoginRoute} />
    <Route path="/setup" component={Setup} />
    <Route path="/" component={Dashboard} />
    <Route path="/rules" component={RulesList} />
    <Route path="/rules/new" component={RuleEditor} />
    <Route path="/rules/:id" component={RuleEditor} />
    <Route path="/rules/:id/decisions" component={RuleDecisions} />
    <Route path="*" component={NotFound} />
  </Router>
);

export default App;
