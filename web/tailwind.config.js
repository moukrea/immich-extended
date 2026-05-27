/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        "immich-primary": "rgb(var(--immich-primary) / <alpha-value>)",
        "immich-dark-primary": "rgb(var(--immich-dark-primary) / <alpha-value>)",
        "immich-bg": "rgb(var(--immich-bg) / <alpha-value>)",
        "immich-fg": "rgb(var(--immich-fg) / <alpha-value>)",
        "immich-dark-bg": "rgb(var(--immich-dark-bg) / <alpha-value>)",
        "immich-dark-fg": "rgb(var(--immich-dark-fg) / <alpha-value>)",
        "immich-dark-gray": "rgb(var(--immich-dark-gray) / <alpha-value>)",
        "ui-primary": "rgb(var(--immich-ui-primary) / <alpha-value>)",
        "ui-success": "rgb(var(--immich-ui-success) / <alpha-value>)",
        "ui-danger": "rgb(var(--immich-ui-danger) / <alpha-value>)",
        "ui-warning": "rgb(var(--immich-ui-warning) / <alpha-value>)",
        "ui-info": "rgb(var(--immich-ui-info) / <alpha-value>)",
        "ui-muted": "rgb(var(--immich-ui-muted) / <alpha-value>)",
        "ui-gray": "rgb(var(--immich-ui-gray) / <alpha-value>)",
        "ui-dark": "rgb(var(--immich-ui-dark) / <alpha-value>)",
        "ui-light": "rgb(var(--immich-ui-light) / <alpha-value>)",
        "ui-border": "rgb(var(--immich-ui-default-border) / <alpha-value>)",
      },
      fontFamily: {
        sans: [
          "Overpass",
          "ui-sans-serif",
          "system-ui",
          "-apple-system",
          "Segoe UI",
          "Roboto",
          "sans-serif",
        ],
        mono: [
          "Overpass Mono",
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "monospace",
        ],
      },
      boxShadow: {
        "primary-glow":
          "0 4px 6px -1px rgb(var(--immich-ui-primary) / 0.2), 0 2px 4px -2px rgb(var(--immich-ui-primary) / 0.2)",
      },
      transitionTimingFunction: {
        immich: "cubic-bezier(.4, 0, .2, 1)",
      },
      spacing: {
        navbar: "calc(4.5rem + 4px)",
        "navbar-md": "calc(4.5rem - 10px)",
      },
    },
  },
  plugins: [],
};
