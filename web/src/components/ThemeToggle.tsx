import { createSignal, onMount, type Component } from "solid-js";

type Theme = "dark" | "light";

function currentTheme(): Theme {
  if (typeof document === "undefined") {
    return "dark";
  }
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

function applyTheme(theme: Theme): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
    root.classList.remove("light");
  } else {
    root.classList.remove("dark");
    root.classList.add("light");
  }
  try {
    localStorage.setItem("theme", theme);
  } catch {
    /* localStorage might be unavailable (SSR, privacy mode) */
  }
}

const ThemeToggle: Component = () => {
  const [theme, setTheme] = createSignal<Theme>("dark");

  onMount(() => {
    setTheme(currentTheme());
  });

  const toggle = () => {
    const next: Theme = theme() === "dark" ? "light" : "dark";
    applyTheme(next);
    setTheme(next);
  };

  return (
    <button
      type="button"
      data-testid="theme-toggle"
      aria-label={
        theme() === "dark" ? "Switch to light mode" : "Switch to dark mode"
      }
      aria-pressed={theme() === "dark"}
      onClick={toggle}
      class={[
        "inline-flex items-center justify-center",
        "h-9 w-9 rounded-full",
        "text-gray-600 dark:text-gray-300",
        "hover:bg-immich-primary/10 dark:hover:bg-immich-dark-primary/10",
        "transition ease-immich duration-150",
        "focus:outline-none focus-visible:ring-2 focus-visible:ring-immich-primary",
        "dark:focus-visible:ring-immich-dark-primary",
      ].join(" ")}
    >
      {theme() === "dark" ? (
        <svg
          class="h-5 w-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          aria-hidden="true"
        >
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
        </svg>
      ) : (
        <svg
          class="h-5 w-5"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          aria-hidden="true"
        >
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
        </svg>
      )}
    </button>
  );
};

export default ThemeToggle;
