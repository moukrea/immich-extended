import {
  createSignal,
  onCleanup,
  onMount,
  Show,
  type Component,
} from "solid-js";
import { A } from "@solidjs/router";
import ThemeToggle from "./ThemeToggle";
import type { Me } from "../lib/api";

export interface AccountMenuProps {
  me: Me | null;
  onSignOut?: () => void | Promise<void>;
}

function displayName(me: Me): string {
  return me.display_name?.trim() || me.email;
}

function initials(me: Me): string {
  return displayName(me).slice(0, 2).toUpperCase();
}

const AccountMenu: Component<AccountMenuProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;

  const close = () => setOpen(false);
  const toggle = () => setOpen((v) => !v);

  const onDocPointer = (event: Event) => {
    if (!containerRef) return;
    const target = event.target;
    if (target instanceof Node && containerRef.contains(target)) return;
    close();
  };
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") close();
  };

  onMount(() => {
    document.addEventListener("pointerdown", onDocPointer, true);
    document.addEventListener("keydown", onKeyDown, true);
  });
  onCleanup(() => {
    document.removeEventListener("pointerdown", onDocPointer, true);
    document.removeEventListener("keydown", onKeyDown, true);
  });

  const signOut = () => {
    close();
    void props.onSignOut?.();
  };

  return (
    <div ref={containerRef} class="relative">
      <button
        type="button"
        data-testid="account-menu-button"
        aria-haspopup="menu"
        aria-expanded={open()}
        aria-label="Account menu"
        onClick={toggle}
        class={[
          "inline-flex h-9 w-9 items-center justify-center rounded-full",
          "bg-immich-primary/15 text-immich-primary",
          "dark:bg-immich-dark-primary/15 dark:text-immich-dark-primary",
          "ring-1 ring-inset ring-immich-primary/20 dark:ring-immich-dark-primary/20",
          "text-xs font-semibold uppercase",
          "hover:bg-immich-primary/25 dark:hover:bg-immich-dark-primary/25",
          "transition ease-immich duration-150",
          "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
          "dark:focus-visible:ring-immich-dark-primary",
        ].join(" ")}
      >
        <Show
          when={props.me}
          fallback={
            <svg
              class="h-5 w-5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              aria-hidden="true"
            >
              <circle cx="12" cy="8" r="4" />
              <path d="M4 21a8 8 0 0 1 16 0" stroke-linecap="round" />
            </svg>
          }
        >
          {(me) => <span aria-hidden="true">{initials(me())}</span>}
        </Show>
      </button>

      <Show when={open()}>
        <div
          data-testid="account-menu-popup"
          role="menu"
          aria-label="Account"
          class={[
            "absolute right-0 mt-2 z-50 w-72 origin-top-right",
            "rounded-3xl border border-ui-border dark:border-gray-700",
            "bg-white dark:bg-immich-dark-gray",
            "shadow-xl",
          ].join(" ")}
        >
          <div class="flex flex-col items-center gap-3 px-5 pt-5 pb-4">
            <div
              aria-hidden="true"
              class={[
                "flex h-16 w-16 items-center justify-center rounded-full",
                "bg-immich-primary/15 text-immich-primary",
                "dark:bg-immich-dark-primary/15 dark:text-immich-dark-primary",
                "text-xl font-semibold uppercase",
              ].join(" ")}
            >
              <Show when={props.me} fallback="??">
                {(me) => initials(me())}
              </Show>
            </div>
            <div class="text-center leading-tight">
              <p
                data-testid="account-menu-name"
                class="text-base font-semibold text-immich-fg dark:text-immich-dark-fg"
              >
                <Show when={props.me} fallback="Account">
                  {(me) => displayName(me())}
                </Show>
              </p>
              <Show
                when={props.me && props.me.email !== displayName(props.me)}
              >
                <p
                  data-testid="account-menu-email"
                  class="text-sm text-ui-muted dark:text-gray-400"
                >
                  {props.me?.email}
                </p>
              </Show>
            </div>
            <A
              href="/me"
              data-testid="account-menu-settings"
              role="menuitem"
              onClick={close}
              class={[
                "mt-1 inline-flex w-full items-center justify-center gap-2",
                "rounded-2xl border border-ui-border dark:border-gray-600",
                "px-4 py-2 text-sm font-medium",
                "text-immich-fg dark:text-immich-dark-fg",
                "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
                "transition ease-immich duration-150",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
              ].join(" ")}
            >
              <svg
                class="h-4 w-4"
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
              Settings
            </A>
          </div>

          <div class="flex items-center justify-between px-5 pb-4">
            <span class="text-sm font-medium text-gray-700 dark:text-gray-300">
              Theme
            </span>
            <ThemeToggle />
          </div>

          <div class="border-t border-ui-border dark:border-gray-700 p-2">
            <button
              type="button"
              data-testid="account-menu-signout"
              role="menuitem"
              onClick={signOut}
              class={[
                "flex w-full items-center gap-3 rounded-2xl px-3 py-2.5",
                "text-sm font-medium text-gray-700 dark:text-gray-300",
                "hover:bg-ui-danger/10 hover:text-ui-danger",
                "dark:hover:bg-ui-danger/10 dark:hover:text-ui-danger",
                "transition ease-immich duration-150",
                "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
              ].join(" ")}
            >
              <span
                class="flex h-5 w-5 items-center justify-center"
                aria-hidden="true"
              >
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
        </div>
      </Show>
    </div>
  );
};

export default AccountMenu;
