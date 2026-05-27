import {
  createSignal,
  onMount,
  Show,
  type Component,
  type JSX,
} from "solid-js";
import { useNavigate } from "@solidjs/router";
import { getMe, postLogout, type Me } from "../lib/api";
import SidebarNav, { type SidebarItem } from "./SidebarNav";
import TopBar from "./TopBar";

const ICONS: Record<string, JSX.Element> = {
  rules: (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      aria-hidden="true"
    >
      <path
        d="M4 6h12M4 12h16M4 18h8"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  ),
  activity: (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      aria-hidden="true"
    >
      <path
        d="M3 12h4l3-8 4 16 3-8h4"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  ),
  settings: (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      aria-hidden="true"
    >
      <circle cx="12" cy="12" r="3" />
      <path
        d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    </svg>
  ),
};

const DEFAULT_ITEMS: SidebarItem[] = [
  {
    href: "/rules",
    label: "Rules",
    icon: ICONS.rules,
    matchPrefix: true,
  },
  {
    href: "/",
    label: "Activity",
    icon: ICONS.activity,
    matchPrefix: false,
  },
  {
    href: "/me",
    label: "Settings",
    icon: ICONS.settings,
    matchPrefix: true,
  },
];

export interface AppShellProps {
  children?: JSX.Element;
  initialMe?: Me | null;
  navItems?: SidebarItem[];
}

const AppShell: Component<AppShellProps> = (props) => {
  const navigate = useNavigate();
  const [me, setMe] = createSignal<Me | null>(props.initialMe ?? null);
  const [mobileOpen, setMobileOpen] = createSignal(false);

  onMount(async () => {
    if (props.initialMe !== undefined) return;
    const res = await getMe();
    if (res.ok) {
      setMe(res.data);
    }
  });

  const signOut = async () => {
    await postLogout();
    setMe(null);
    navigate("/login", { replace: true });
  };

  const navItems = () => props.navItems ?? DEFAULT_ITEMS;

  return (
    <div
      data-testid="app-shell"
      class="min-h-screen bg-immich-bg text-immich-fg dark:bg-immich-dark-bg dark:text-immich-dark-fg"
    >
      <TopBar
        me={me()}
        onMenuClick={() => setMobileOpen(true)}
        onSignOut={signOut}
      />
      <div class="flex">
        <aside
          data-testid="sidebar"
          class={[
            "hidden md:flex md:flex-col",
            "w-64 shrink-0",
            "h-[calc(100vh-theme(spacing.navbar))]",
            "sticky top-navbar",
            "border-r border-ui-border dark:border-immich-dark-gray",
            "bg-immich-bg dark:bg-immich-dark-bg",
            "p-4",
          ].join(" ")}
        >
          <SidebarNav items={navItems()} />
          <div class="mt-auto pt-4">
            <button
              type="button"
              data-testid="sidebar-signout"
              onClick={signOut}
              class={[
                "flex w-full items-center gap-3 rounded-xl px-3 py-2",
                "text-sm font-medium text-gray-700 dark:text-gray-300",
                "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
                "hover:text-immich-primary dark:hover:text-immich-dark-primary",
                "transition ease-immich duration-150",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
              ].join(" ")}
            >
              <span class="flex h-5 w-5 items-center justify-center" aria-hidden="true">
                <svg
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                >
                  <path
                    d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4M16 17l5-5-5-5M21 12H9"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                  />
                </svg>
              </span>
              Sign out
            </button>
          </div>
        </aside>
        <Show when={mobileOpen()}>
          <div
            data-testid="mobile-drawer"
            class="fixed inset-0 z-40 flex md:hidden"
            role="dialog"
            aria-modal="true"
            aria-label="Navigation"
          >
            <div
              class="absolute inset-0 bg-black/50"
              onClick={() => setMobileOpen(false)}
            />
            <div
              class={[
                "relative z-10 flex h-full w-64 flex-col",
                "bg-immich-bg dark:bg-immich-dark-bg",
                "border-r border-ui-border dark:border-immich-dark-gray",
                "p-4",
              ].join(" ")}
            >
              <div class="mb-4 flex items-center justify-between">
                <span class="text-base font-semibold tracking-tight">
                  immich-extended
                </span>
                <button
                  type="button"
                  aria-label="Close navigation"
                  onClick={() => setMobileOpen(false)}
                  class={[
                    "inline-flex h-8 w-8 items-center justify-center rounded-lg",
                    "text-gray-700 dark:text-gray-300",
                    "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
                  ].join(" ")}
                >
                  <svg
                    class="h-5 w-5"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    aria-hidden="true"
                  >
                    <path
                      d="M6 6l12 12M18 6L6 18"
                      stroke-linecap="round"
                    />
                  </svg>
                </button>
              </div>
              <SidebarNav
                items={navItems()}
                onNavigate={() => setMobileOpen(false)}
              />
              <button
                type="button"
                onClick={() => {
                  setMobileOpen(false);
                  void signOut();
                }}
                class={[
                  "mt-auto flex w-full items-center gap-3 rounded-xl px-3 py-2",
                  "text-sm font-medium text-gray-700 dark:text-gray-300",
                  "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
                ].join(" ")}
              >
                Sign out
              </button>
            </div>
          </div>
        </Show>
        <main
          data-testid="app-main"
          class="flex-1 min-w-0 px-4 sm:px-6 lg:px-8 py-6"
        >
          {props.children}
        </main>
      </div>
    </div>
  );
};

export default AppShell;
