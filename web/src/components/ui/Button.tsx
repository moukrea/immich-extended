import { Show, splitProps, type Component, type JSX } from "solid-js";

export type ButtonVariant = "primary" | "secondary" | "destructive" | "ghost";
export type ButtonSize = "sm" | "md";

export interface ButtonProps
  extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
}

const variantClass: Record<ButtonVariant, string> = {
  primary: [
    "bg-immich-primary text-white",
    "shadow-md shadow-ui-primary/20",
    "hover:bg-immich-primary/90 active:bg-immich-primary/80",
    "disabled:bg-gray-400 disabled:text-gray-100 disabled:shadow-none disabled:cursor-not-allowed",
    "dark:bg-immich-dark-primary dark:text-immich-dark-bg",
    "dark:hover:bg-immich-dark-primary/90 dark:active:bg-immich-dark-primary/80",
  ].join(" "),
  secondary: [
    "bg-slate-200 dark:bg-gray-600 text-immich-fg dark:text-immich-dark-fg",
    "hover:bg-slate-300 dark:hover:bg-gray-500",
    "active:bg-slate-400 dark:active:bg-gray-400",
    "disabled:bg-gray-300 disabled:text-gray-500 disabled:cursor-not-allowed",
    "dark:disabled:bg-gray-700 dark:disabled:text-gray-500",
  ].join(" "),
  destructive: [
    "bg-ui-danger text-white",
    "hover:bg-ui-danger/90 active:bg-ui-danger/80",
    "disabled:bg-gray-400 disabled:text-gray-100 disabled:cursor-not-allowed",
  ].join(" "),
  ghost: [
    "bg-transparent text-immich-primary dark:text-immich-dark-primary",
    "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
    "active:bg-immich-primary/20 dark:active:bg-immich-dark-primary/20",
    "disabled:opacity-50 disabled:cursor-not-allowed",
  ].join(" "),
};

const sizeClass: Record<ButtonSize, string> = {
  sm: "px-3 py-1.5 text-xs",
  md: "px-4 py-2 text-sm",
};

const base = [
  "inline-flex items-center justify-center gap-2",
  "rounded-lg font-medium",
  "transition ease-immich duration-150",
  "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
  "dark:focus-visible:ring-immich-dark-primary",
].join(" ");

const Spinner: Component = () => (
  <svg
    class="h-4 w-4 animate-spin"
    viewBox="0 0 24 24"
    fill="none"
    aria-hidden="true"
  >
    <circle
      cx="12"
      cy="12"
      r="10"
      stroke="currentColor"
      stroke-width="3"
      class="opacity-25"
    />
    <path
      d="M22 12a10 10 0 0 1-10 10"
      stroke="currentColor"
      stroke-width="3"
      stroke-linecap="round"
      class="opacity-75"
    />
  </svg>
);

const Button: Component<ButtonProps> = (props) => {
  const [local, rest] = splitProps(props, [
    "variant",
    "size",
    "loading",
    "class",
    "children",
    "type",
    "disabled",
  ]);
  const variant = (): ButtonVariant => local.variant ?? "primary";
  const size = (): ButtonSize => local.size ?? "md";
  return (
    <button
      data-testid="button"
      data-variant={variant()}
      type={local.type ?? "button"}
      disabled={local.disabled || local.loading}
      aria-busy={local.loading ? "true" : undefined}
      {...rest}
      class={[base, variantClass[variant()], sizeClass[size()], local.class ?? ""]
        .filter(Boolean)
        .join(" ")}
    >
      <Show when={local.loading}>
        <Spinner />
      </Show>
      {local.children}
    </button>
  );
};

export default Button;
