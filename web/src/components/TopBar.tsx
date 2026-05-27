import { Show, type Component } from "solid-js";
import ThemeToggle from "./ThemeToggle";
import type { Me } from "../lib/api";

export interface TopBarProps {
  me: Me | null;
  onMenuClick?: () => void;
  onSignOut?: () => void | Promise<void>;
}

const TopBar: Component<TopBarProps> = (props) => {
  return (
    <header
      data-testid="topbar"
      class={[
        "sticky top-0 z-30",
        "h-navbar md:h-navbar-md lg:h-navbar",
        "border-b border-ui-border dark:border-immich-dark-gray",
        "bg-immich-bg/95 dark:bg-immich-dark-bg/95 backdrop-blur",
        "px-4 lg:px-6",
        "flex items-center justify-between",
      ].join(" ")}
    >
      <div class="flex items-center gap-3">
        <button
          type="button"
          data-testid="topbar-menu"
          aria-label="Open navigation"
          onClick={() => props.onMenuClick?.()}
          class={[
            "md:hidden inline-flex h-9 w-9 items-center justify-center rounded-lg",
            "text-gray-700 dark:text-gray-300",
            "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
            "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
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
            <path d="M4 6h16M4 12h16M4 18h16" stroke-linecap="round" />
          </svg>
        </button>
        <span class="text-base font-semibold tracking-tight text-immich-fg dark:text-immich-dark-fg">
          immich-extended
        </span>
      </div>
      <div class="flex items-center gap-2">
        <ThemeToggle />
        <Show when={props.me}>
          {(me) => (
            <div class="hidden sm:flex items-center gap-2 pl-2">
              <div
                aria-hidden="true"
                class={[
                  "flex h-8 w-8 items-center justify-center rounded-full",
                  "bg-immich-primary/15 text-immich-primary",
                  "dark:bg-immich-dark-primary/15 dark:text-immich-dark-primary",
                  "text-xs font-semibold uppercase",
                ].join(" ")}
              >
                {(me().display_name ?? me().email).slice(0, 2)}
              </div>
              <div class="flex flex-col leading-tight">
                <span
                  data-testid="topbar-user-name"
                  class="text-sm font-medium text-immich-fg dark:text-immich-dark-fg"
                >
                  {me().display_name ?? me().email}
                </span>
                <Show when={me().display_name && me().display_name !== me().email}>
                  <span class="text-xs text-ui-muted dark:text-gray-400">
                    {me().email}
                  </span>
                </Show>
              </div>
            </div>
          )}
        </Show>
        <Show when={props.onSignOut}>
          <button
            type="button"
            data-testid="topbar-signout"
            onClick={() => props.onSignOut?.()}
            class={[
              "ml-1 hidden md:inline-flex items-center rounded-lg",
              "px-3 py-1.5 text-sm font-medium",
              "text-gray-700 dark:text-gray-300",
              "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
              "transition ease-immich duration-150",
              "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
            ].join(" ")}
          >
            Sign out
          </button>
        </Show>
      </div>
    </header>
  );
};

export default TopBar;
