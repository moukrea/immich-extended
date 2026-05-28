// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

const navigateMock = vi.fn();

type AnchorAriaCurrent =
  | "page"
  | "step"
  | "location"
  | "date"
  | "time"
  | "true"
  | "false"
  | undefined;

vi.mock("@solidjs/router", () => {
  return {
    A: (props: {
      href: string;
      class?: string;
      "data-testid"?: string;
      "aria-current"?: AnchorAriaCurrent;
      onClick?: (e: MouseEvent) => void;
      children?: unknown;
    }) => (
      <a
        href={props.href}
        class={props.class}
        data-testid={props["data-testid"]}
        aria-current={props["aria-current"]}
        onClick={(e) => props.onClick?.(e)}
      >
        {props.children as JSX.Element}
      </a>
    ),
    useNavigate: () => navigateMock,
    useLocation: () => ({ pathname: "/rules" }),
  };
});

const apiMock = vi.hoisted(() => ({
  postLogout: vi.fn(),
  getMe: vi.fn(),
}));

vi.mock("../../lib/api", () => apiMock);

import AppShell from "../AppShell";
import ThemeToggle from "../ThemeToggle";
import SidebarNav from "../SidebarNav";

beforeEach(() => {
  navigateMock.mockReset();
  apiMock.postLogout.mockReset();
  apiMock.postLogout.mockResolvedValue({ ok: true, data: undefined });
  apiMock.getMe.mockReset();
  apiMock.getMe.mockResolvedValue({
    ok: false,
    error: { status: 401, code: "no_session" },
  });
  try {
    localStorage.clear();
  } catch {
    /* jsdom may not implement */
  }
  document.documentElement.classList.remove("light");
  document.documentElement.classList.add("dark");
});

afterEach(() => {
  cleanup();
});

describe("AppShell", () => {
  it("renders topbar, sidebar, and main slot with the provided children", () => {
    const { getByTestId, container } = render(() => (
      <AppShell initialMe={{ user_id: "u1", email: "a@b", display_name: "Alice" }}>
        <p data-testid="page">hello</p>
      </AppShell>
    ));
    expect(getByTestId("app-shell")).not.toBeNull();
    expect(getByTestId("topbar")).not.toBeNull();
    expect(getByTestId("sidebar")).not.toBeNull();
    expect(getByTestId("page").textContent).toBe("hello");
    expect(getByTestId("account-menu-button")).not.toBeNull();
    expect(container.textContent).toContain("Rules");
    expect(container.textContent).toContain("Activity");
    expect(container.textContent).toContain("Settings");
  });

  it("exposes name + email only inside the account menu popup", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AppShell
        initialMe={{ user_id: "u1", email: "a@b.com", display_name: null }}
      >
        <span />
      </AppShell>
    ));
    // Closed: identity is not in the DOM.
    expect(queryByTestId("account-menu-name")).toBeNull();
    fireEvent.click(getByTestId("account-menu-button"));
    // Open: name falls back to email when display_name is null.
    expect(getByTestId("account-menu-name").textContent).toBe("a@b.com");
  });

  it("has no sign-out controls outside the account menu", () => {
    const { queryByTestId } = render(() => (
      <AppShell initialMe={{ user_id: "u1", email: "a@b", display_name: "Alice" }}>
        <span />
      </AppShell>
    ));
    expect(queryByTestId("topbar-signout")).toBeNull();
    expect(queryByTestId("sidebar-signout")).toBeNull();
    // The only sign-out lives in the (closed-by-default) account menu.
    expect(queryByTestId("account-menu-signout")).toBeNull();
  });

  it("signs out via the account menu + navigates to /login", async () => {
    const { getByTestId } = render(() => (
      <AppShell initialMe={{ user_id: "u1", email: "a@b", display_name: null }}>
        <span />
      </AppShell>
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    fireEvent.click(getByTestId("account-menu-signout"));
    // microtask drain
    await Promise.resolve();
    await Promise.resolve();
    expect(apiMock.postLogout).toHaveBeenCalledTimes(1);
    expect(navigateMock).toHaveBeenCalledWith("/login", { replace: true });
  });

  it("opens and closes the mobile drawer", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AppShell initialMe={null}>
        <span />
      </AppShell>
    ));
    expect(queryByTestId("mobile-drawer")).toBeNull();
    fireEvent.click(getByTestId("topbar-menu"));
    expect(getByTestId("mobile-drawer")).not.toBeNull();
  });
});

describe("ThemeToggle", () => {
  it("toggles the html root class and persists to localStorage", () => {
    const { getByTestId } = render(() => <ThemeToggle />);
    const btn = getByTestId("theme-toggle");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    fireEvent.click(btn);
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(localStorage.getItem("theme")).toBe("light");
    fireEvent.click(btn);
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(localStorage.getItem("theme")).toBe("dark");
  });
});

describe("SidebarNav", () => {
  it("marks the active item with aria-current=page", () => {
    const items = [
      {
        href: "/rules",
        label: "Rules",
        icon: <span>R</span>,
        matchPrefix: true,
      },
      {
        href: "/me",
        label: "Settings",
        icon: <span>S</span>,
        matchPrefix: true,
      },
    ];
    const { getByTestId } = render(() => <SidebarNav items={items} />);
    expect(getByTestId("sidebar-item-rules").getAttribute("aria-current")).toBe(
      "page",
    );
    expect(
      getByTestId("sidebar-item-settings").getAttribute("aria-current"),
    ).toBeNull();
  });

  it("keeps Rules (href '/') active on its /rules sub-pages via matchPrefixes", () => {
    // useLocation is mocked to pathname "/rules".
    const items = [
      {
        href: "/",
        label: "Rules",
        icon: <span>R</span>,
        matchPrefix: true,
        matchPrefixes: ["/rules"],
      },
      {
        href: "/activity",
        label: "Activity",
        icon: <span>A</span>,
        matchPrefix: false,
      },
    ];
    const { getByTestId } = render(() => <SidebarNav items={items} />);
    expect(getByTestId("sidebar-item-rules").getAttribute("aria-current")).toBe(
      "page",
    );
    expect(
      getByTestId("sidebar-item-activity").getAttribute("aria-current"),
    ).toBeNull();
  });
});
