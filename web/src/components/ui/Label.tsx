import { splitProps, type Component, type JSX } from "solid-js";

export interface LabelProps extends JSX.LabelHTMLAttributes<HTMLLabelElement> {
  required?: boolean;
}

const base = [
  "block text-sm font-medium",
  "text-gray-600 dark:text-gray-300",
].join(" ");

const Label: Component<LabelProps> = (props) => {
  const [local, rest] = splitProps(props, ["class", "required", "children"]);
  return (
    <label
      data-testid="label"
      {...rest}
      class={[base, local.class ?? ""].filter(Boolean).join(" ")}
    >
      {local.children}
      {local.required ? (
        <span aria-hidden="true" class="ml-1 text-ui-danger">
          *
        </span>
      ) : null}
    </label>
  );
};

export default Label;
