// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";

import Button from "../ui/Button";
import Card from "../ui/Card";
import Input from "../ui/Input";
import Label from "../ui/Label";
import Select from "../ui/Select";
import Field from "../ui/Field";

afterEach(() => {
  cleanup();
});

describe("Button", () => {
  it("renders primary by default with Immich brand classes", () => {
    const { getByTestId } = render(() => <Button>Save</Button>);
    const btn = getByTestId("button");
    expect(btn.getAttribute("data-variant")).toBe("primary");
    expect(btn.className).toContain("bg-immich-primary");
    expect(btn.className).toContain("rounded-lg");
    expect(btn.className).toContain("focus-visible:ring-immich-primary");
    expect(btn.getAttribute("type")).toBe("button");
  });

  it("renders each variant with distinct base styling", () => {
    const variants = ["primary", "secondary", "destructive", "ghost"] as const;
    for (const v of variants) {
      const { getByTestId, unmount } = render(() => (
        <Button variant={v}>x</Button>
      ));
      const btn = getByTestId("button");
      expect(btn.getAttribute("data-variant")).toBe(v);
      if (v === "destructive") expect(btn.className).toContain("bg-ui-danger");
      if (v === "secondary")
        expect(btn.className).toContain("bg-slate-200");
      if (v === "ghost") expect(btn.className).toContain("bg-transparent");
      unmount();
    }
  });

  it("shows a spinner and disables when loading", () => {
    const onClick = vi.fn();
    const { getByTestId } = render(() => (
      <Button loading onClick={onClick}>
        Saving
      </Button>
    ));
    const btn = getByTestId("button") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute("aria-busy")).toBe("true");
    expect(btn.querySelector("svg")).not.toBeNull();
    fireEvent.click(btn);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("propagates onClick when enabled", () => {
    const onClick = vi.fn();
    const { getByTestId } = render(() => (
      <Button onClick={onClick}>Go</Button>
    ));
    fireEvent.click(getByTestId("button"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});

describe("Card", () => {
  it("renders with default md padding and dark-mode surface", () => {
    const { getByTestId } = render(() => (
      <Card>
        <span>body</span>
      </Card>
    ));
    const card = getByTestId("card");
    expect(card.className).toContain("rounded-2xl");
    expect(card.className).toContain("dark:bg-immich-dark-gray");
    expect(card.className).toContain("p-6");
  });

  it("supports padding sm/none/lg", () => {
    const { getByTestId: a, unmount: ua } = render(() => (
      <Card padding="sm">x</Card>
    ));
    expect(a("card").className).toContain("p-4");
    ua();
    const { getByTestId: b, unmount: ub } = render(() => (
      <Card padding="lg">x</Card>
    ));
    expect(b("card").className).toContain("p-8");
    ub();
    const { getByTestId: c } = render(() => <Card padding="none">x</Card>);
    expect(c("card").className).not.toContain(" p-4");
    expect(c("card").className).not.toContain(" p-6");
    expect(c("card").className).not.toContain(" p-8");
  });
});

describe("Input", () => {
  it("applies the Immich form-input shape", () => {
    const { getByTestId } = render(() => <Input placeholder="email" />);
    const input = getByTestId("input");
    expect(input.className).toContain("rounded-xl");
    expect(input.className).toContain("bg-slate-200");
    expect(input.className).toContain("focus-visible:ring-immich-primary");
  });

  it("marks invalid with aria-invalid and danger ring", () => {
    const { getByTestId } = render(() => <Input invalid />);
    const input = getByTestId("input");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.className).toContain("ring-ui-danger");
  });
});

describe("Label", () => {
  it("renders gray label text and required marker", () => {
    const { getByTestId, container } = render(() => (
      <Label required>Email</Label>
    ));
    const lbl = getByTestId("label");
    expect(lbl.className).toContain("text-sm");
    expect(lbl.className).toContain("font-medium");
    expect(container.textContent).toContain("Email");
    expect(container.textContent).toContain("*");
  });

  it("omits the asterisk when not required", () => {
    const { container } = render(() => <Label>Email</Label>);
    expect(container.querySelector("span[aria-hidden='true']")).toBeNull();
  });
});

describe("Select", () => {
  it("styles the select like an Input with a chevron background", () => {
    const { getByTestId } = render(() => (
      <Select>
        <option>a</option>
      </Select>
    ));
    const sel = getByTestId("select");
    expect(sel.className).toContain("rounded-xl");
    expect(sel.className).toContain("appearance-none");
    expect(sel.className).toContain("bg-[url('data:image/svg+xml");
  });
});

describe("Field", () => {
  it("renders a label + child and a help line", () => {
    const { getByTestId, container } = render(() => (
      <Field label="Email" for_="email-input" help="we email you decisions">
        <Input id="email-input" />
      </Field>
    ));
    const field = getByTestId("field");
    expect(field).not.toBeNull();
    const label = getByTestId("label") as HTMLLabelElement;
    expect(label.getAttribute("for")).toBe("email-input");
    expect(container.textContent).toContain("we email you decisions");
  });

  it("prefers error over help when both provided", () => {
    const { container, queryByRole } = render(() => (
      <Field
        label="Email"
        help="optional"
        error="invalid format"
      >
        <Input />
      </Field>
    ));
    expect(queryByRole("alert")?.textContent).toBe("invalid format");
    expect(container.textContent).not.toContain("optional");
  });
});
