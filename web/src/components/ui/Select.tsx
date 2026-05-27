import { splitProps, type Component, type JSX } from "solid-js";

export interface SelectProps
  extends JSX.SelectHTMLAttributes<HTMLSelectElement> {
  invalid?: boolean;
}

const base = [
  "w-full rounded-xl appearance-none",
  "bg-slate-200 dark:bg-gray-600",
  "text-sm text-immich-fg dark:text-immich-dark-fg",
  "px-3 py-3 pr-9",
  "border border-transparent",
  "transition ease-immich duration-150",
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
  "dark:focus-visible:ring-immich-dark-primary",
  "disabled:cursor-not-allowed disabled:bg-gray-300 disabled:text-gray-500",
  "dark:disabled:bg-gray-700 dark:disabled:text-gray-400",
  // chevron via inline SVG background (Tailwind v3 friendly)
  "bg-no-repeat bg-[length:1rem] bg-[position:right_0.75rem_center]",
  "bg-[url('data:image/svg+xml;utf8,<svg%20xmlns=%22http://www.w3.org/2000/svg%22%20viewBox=%220%200%2020%2020%22%20fill=%22currentColor%22><path%20fill-rule=%22evenodd%22%20d=%22M5.23%207.21a.75.75%200%200%201%201.06.02L10%2011.06l3.71-3.83a.75.75%200%201%201%201.08%201.04l-4.25%204.4a.75.75%200%200%201-1.08%200l-4.25-4.4a.75.75%200%200%201%20.02-1.06z%22%20clip-rule=%22evenodd%22/></svg>')]",
].join(" ");

const invalidClass = [
  "ring-2 ring-ui-danger",
  "focus-visible:ring-ui-danger dark:focus-visible:ring-ui-danger",
].join(" ");

const Select: Component<SelectProps> = (props) => {
  const [local, rest] = splitProps(props, ["class", "invalid", "children"]);
  return (
    <select
      data-testid="select"
      {...rest}
      aria-invalid={local.invalid ? "true" : undefined}
      class={[base, local.invalid ? invalidClass : "", local.class ?? ""]
        .filter(Boolean)
        .join(" ")}
    >
      {local.children}
    </select>
  );
};

export default Select;
