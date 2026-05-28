// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { JSX } from "solid-js";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

vi.mock("@solidjs/router", () => {
  return {
    A: (props: {
      href: string;
      class?: string;
      "data-testid"?: string;
      onClick?: (e: MouseEvent) => void;
      children?: unknown;
    }) => (
      <a
        href={props.href}
        class={props.class}
        data-testid={props["data-testid"]}
        onClick={(e) => props.onClick?.(e)}
      >
        {props.children as JSX.Element}
      </a>
    ),
  };
});

import AccountMenu from "../AccountMenu";

const ALICE = {
  user_id: "u1",
  email: "alice@example.com",
  display_name: "Alice Example",
};

beforeEach(() => {
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

describe("AccountMenu", () => {
  it("renders only the avatar button while closed", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    const button = getByTestId("account-menu-button");
    expect(button).not.toBeNull();
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(queryByTestId("account-menu-popup")).toBeNull();
  });

  it("opens on click and shows identity, settings, theme, and sign out", () => {
    const { getByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    expect(getByTestId("account-menu-popup")).not.toBeNull();
    expect(getByTestId("account-menu-button").getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(getByTestId("account-menu-name").textContent).toBe("Alice Example");
    expect(getByTestId("account-menu-email").textContent).toBe(
      "alice@example.com",
    );
    const settings = getByTestId("account-menu-settings");
    expect(settings.getAttribute("href")).toBe("/me");
    expect(getByTestId("theme-toggle")).not.toBeNull();
    expect(getByTestId("account-menu-signout")).not.toBeNull();
  });

  it("falls back to email as the name when display_name is null", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu
        me={{ user_id: "u2", email: "bob@example.com", display_name: null }}
        onSignOut={() => {}}
      />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    expect(getByTestId("account-menu-name").textContent).toBe("bob@example.com");
    // No redundant duplicate email line when name already is the email.
    expect(queryByTestId("account-menu-email")).toBeNull();
  });

  it("calls onSignOut and closes when Sign out is clicked", () => {
    const onSignOut = vi.fn();
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={onSignOut} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    fireEvent.click(getByTestId("account-menu-signout"));
    expect(onSignOut).toHaveBeenCalledTimes(1);
    expect(queryByTestId("account-menu-popup")).toBeNull();
  });

  it("toggles the theme from inside the popup", () => {
    const { getByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    fireEvent.click(getByTestId("theme-toggle"));
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.classList.contains("light")).toBe(true);
    expect(localStorage.getItem("theme")).toBe("light");
  });

  it("closes the popup on Escape", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    expect(getByTestId("account-menu-popup")).not.toBeNull();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(queryByTestId("account-menu-popup")).toBeNull();
  });

  it("closes the popup when clicking outside", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    expect(getByTestId("account-menu-popup")).not.toBeNull();
    fireEvent.pointerDown(document.body);
    expect(queryByTestId("account-menu-popup")).toBeNull();
  });

  it("closes the popup when the Settings link is followed", () => {
    const { getByTestId, queryByTestId } = render(() => (
      <AccountMenu me={ALICE} onSignOut={() => {}} />
    ));
    fireEvent.click(getByTestId("account-menu-button"));
    fireEvent.click(getByTestId("account-menu-settings"));
    expect(queryByTestId("account-menu-popup")).toBeNull();
  });
});
