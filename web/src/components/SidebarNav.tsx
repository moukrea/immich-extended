import { For, type Component, type JSX } from "solid-js";
import { A, useLocation } from "@solidjs/router";

export interface SidebarItem {
  href: string;
  label: string;
  icon: JSX.Element;
  matchPrefix?: boolean;
  // Extra path prefixes that also mark this item active. Lets "Rules" (href
  // "/") stay highlighted on its `/rules/:id` sub-pages without matching "/"
  // against every route.
  matchPrefixes?: string[];
}

export interface SidebarNavProps {
  items: SidebarItem[];
  onNavigate?: () => void;
  collapsed?: boolean;
}

const baseItem = [
  "flex items-center gap-3 rounded-xl",
  "px-3 py-2 text-sm font-medium",
  "transition ease-immich duration-150",
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
  "dark:focus-visible:ring-immich-dark-primary",
].join(" ");

const inactive = [
  "text-gray-700 dark:text-gray-300",
  "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
  "hover:text-immich-primary dark:hover:text-immich-dark-primary",
].join(" ");

const active = [
  "bg-immich-primary/10 dark:bg-immich-dark-primary/15",
  "text-immich-primary dark:text-immich-dark-primary",
].join(" ");

function underPrefix(currentPath: string, prefix: string) {
  return currentPath === prefix || currentPath.startsWith(prefix + "/");
}

function isActive(
  currentPath: string,
  href: string,
  matchPrefix?: boolean,
  matchPrefixes?: string[],
) {
  if (matchPrefixes?.some((p) => underPrefix(currentPath, p))) {
    return true;
  }
  if (matchPrefix) {
    if (href === "/") return currentPath === "/";
    return underPrefix(currentPath, href);
  }
  return currentPath === href;
}

const SidebarNav: Component<SidebarNavProps> = (props) => {
  const location = useLocation();
  return (
    <nav data-testid="sidebar-nav" aria-label="Primary" class="space-y-1">
      <For each={props.items}>
        {(item) => {
          const itemActive = () =>
            isActive(
              location.pathname,
              item.href,
              item.matchPrefix,
              item.matchPrefixes,
            );
          return (
            <A
              href={item.href}
              data-testid={`sidebar-item-${item.label.toLowerCase().replace(/\s+/g, "-")}`}
              onClick={() => props.onNavigate?.()}
              aria-current={itemActive() ? "page" : undefined}
              class={[
                baseItem,
                itemActive() ? active : inactive,
                props.collapsed ? "justify-center" : "",
              ]
                .filter(Boolean)
                .join(" ")}
            >
              <span class="flex h-5 w-5 items-center justify-center" aria-hidden="true">
                {item.icon}
              </span>
              {props.collapsed ? (
                <span class="sr-only">{item.label}</span>
              ) : (
                <span>{item.label}</span>
              )}
            </A>
          );
        }}
      </For>
    </nav>
  );
};

export default SidebarNav;
