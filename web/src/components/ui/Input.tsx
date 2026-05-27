import { splitProps, type Component, type JSX } from "solid-js";

export interface InputProps
  extends JSX.InputHTMLAttributes<HTMLInputElement> {
  invalid?: boolean;
}

const base = [
  "w-full rounded-xl",
  "bg-slate-200 dark:bg-gray-600",
  "text-sm text-immich-fg dark:text-immich-dark-fg",
  "placeholder:text-gray-500 dark:placeholder:text-gray-300",
  "px-3 py-3",
  "border border-transparent",
  "transition ease-immich duration-150",
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
  "dark:focus-visible:ring-immich-dark-primary",
  "disabled:cursor-not-allowed disabled:bg-gray-300 disabled:text-gray-500",
  "dark:disabled:bg-gray-700 dark:disabled:text-gray-400",
].join(" ");

const invalidClass = [
  "ring-2 ring-ui-danger",
  "focus-visible:ring-ui-danger dark:focus-visible:ring-ui-danger",
].join(" ");

const Input: Component<InputProps> = (props) => {
  const [local, rest] = splitProps(props, ["class", "invalid"]);
  return (
    <input
      data-testid="input"
      {...rest}
      aria-invalid={local.invalid ? "true" : undefined}
      class={[base, local.invalid ? invalidClass : "", local.class ?? ""]
        .filter(Boolean)
        .join(" ")}
    />
  );
};

export default Input;
