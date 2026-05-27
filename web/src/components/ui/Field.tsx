import { Show, type Component, type JSX } from "solid-js";
import Label from "./Label";

export interface FieldProps {
  label: string;
  for_?: string;
  required?: boolean;
  help?: string;
  error?: string;
  children: JSX.Element;
  class?: string;
}

const Field: Component<FieldProps> = (props) => {
  return (
    <div
      data-testid="field"
      class={["space-y-1.5", props.class ?? ""].filter(Boolean).join(" ")}
    >
      <Label for={props.for_} required={props.required}>
        {props.label}
      </Label>
      {props.children}
      <Show when={props.error}>
        {(msg) => (
          <p class="text-xs text-ui-danger" role="alert">
            {msg()}
          </p>
        )}
      </Show>
      <Show when={!props.error && props.help}>
        {(msg) => (
          <p class="text-xs text-ui-muted dark:text-gray-400">{msg()}</p>
        )}
      </Show>
    </div>
  );
};

export default Field;
