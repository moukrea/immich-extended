import { type Component } from "solid-js";
import AccountMenu from "./AccountMenu";
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
        <AccountMenu me={props.me} onSignOut={props.onSignOut} />
      </div>
    </header>
  );
};

export default TopBar;
