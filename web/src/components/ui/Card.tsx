import { splitProps, type Component, type JSX } from "solid-js";

export type CardPadding = "none" | "sm" | "md" | "lg";

export interface CardProps extends JSX.HTMLAttributes<HTMLDivElement> {
  padding?: CardPadding;
}

const paddingClass: Record<CardPadding, string> = {
  none: "",
  sm: "p-4",
  md: "p-6",
  lg: "p-8",
};

const Card: Component<CardProps> = (props) => {
  const [local, rest] = splitProps(props, ["padding", "class", "children"]);
  const pad = (): CardPadding => local.padding ?? "md";
  return (
    <div
      data-testid="card"
      {...rest}
      class={[
        "rounded-2xl bg-white dark:bg-immich-dark-gray",
        "border border-ui-border dark:border-immich-dark-gray",
        "shadow-sm",
        paddingClass[pad()],
        local.class ?? "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {local.children}
    </div>
  );
};

export default Card;
