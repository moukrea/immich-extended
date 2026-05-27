# Immich UI style mirror

> **Purpose.** This document captures Immich's design language — colors, type, spacing, radii, motion, layout patterns — so that POSTSHIP-T17 (theme primitives + `<AppShell>`) and POSTSHIP-T21 (page polish) can mirror it without guessing.
>
> **Method.** Tokens were extracted from the deployed Immich at `https://immich.${DOMAIN}/` on 2026-05-27 by fetching the served stylesheet (`/_app/immutable/assets/0.HmSofUKY.css`, 128 135 bytes, the SvelteKit-compiled Tailwind v4 build) and grepping for CSS custom properties + utility patterns. Cross-referenced where useful against `github.com/immich-app/immich` (the upstream `web/` package).
>
> **Anti-goals (per task spec §6).** We do *not* mirror Immich's photo grid, map view, asset detail viewer, or timeline scrubber. Those are out of scope for immich-extended — we only need shell + form + card + button + nav patterns.
>
> **Source-of-truth status.** Everything below is a snapshot of what Immich ships today. Any value can be tweaked later, but T17 should land them verbatim first so the look matches; deviations should be deliberate, not accidental.

---

## 1. Brand colors (the only ones that matter)

Immich exposes **two distinct token families** in its CSS:

- `--immich-*` — the brand palette. Two variants only: light-mode (no class) and explicit dark (`.dark` ancestor). Mostly used for brand accents.
- `--immich-ui-*` — the semantic / functional palette (primary, danger, success, warning, info, muted, gray, dark, light, default-border). Every name has *both* a light and dark RGB triplet, scoped by `:root,.light` vs `.dark`.

All triplets are stored as raw `R G B` (space-separated) so opacity modifiers compose via `rgb(var(--immich-ui-primary) / 50%)`. This is the Tailwind v4 / OKLCH-era pattern; we should follow it.

### 1.1 Brand palette (`--immich-*`)

| Token | RGB | Hex | Where used |
|---|---|---|---|
| `--immich-primary` | `66 80 175` | `#4250af` | Brand anchor — buttons, links, focus rings in light mode |
| `--immich-dark-primary` | `172 203 250` | `#accbfa` | Lighter blue used as brand anchor in dark mode (better contrast on near-black bg) |
| `--immich-bg` | `255 255 255` | `#ffffff` | Body background, light mode |
| `--immich-fg` | `0 0 0` | `#000000` | Body text, light mode |
| `--immich-dark-bg` | `10 10 10` | `#0a0a0a` | Body background, dark mode (Immich's signature near-black) |
| `--immich-dark-fg` | `229 231 235` | `#e5e7eb` | Body text, dark mode |
| `--immich-dark-gray` | `33 33 33` | `#212121` | Card / panel surface in dark mode; alternates with `--immich-dark-bg` for zebra rows |

**Notable:** Immich dark mode is *near-black* `#0a0a0a`, not a lifted gray. Cards are `#212121` (`--immich-dark-gray`). That contrast is what gives Immich its photographer-app feel.

### 1.2 Semantic palette (`--immich-ui-*`)

Each row has a light-mode value (applied via `:root,.light`) and a dark-mode value (applied via `.dark`). Bring both into our `:root`/`.dark` blocks.

| Semantic | Light RGB | Light hex | Dark RGB | Dark hex | Notes |
|---|---|---|---|---|---|
| `primary` | `66 80 175` | `#4250af` | `172 203 250` | `#accbfa` | Mirror of `--immich-primary` / `--immich-dark-primary` |
| `success` | `16 188 99` | `#10bc63` | `72 237 152` | `#48ed98` | Pill / toast / status dot |
| `danger` | `200 60 60` | `#c83c3c` | `246 125 125` | `#f67d7d` | Destructive button text/bg, error toast |
| `warning` | `216 143 64` | `#d88f40` | `254 197 132` | `#fec584` | Pending state, soft warnings |
| `info` | `8 111 230` | `#086fe6` | `121 183 254` | `#79b7fe` | Tooltips, informational banners |
| `muted` | `161 161 161` | `#a1a1a1` | `212 212 212` | `#d4d4d4` | Subdued text (subtitles, captions) |
| `gray` | `246 246 246` | `#f6f6f6` | `33 33 33` | `#212121` | Neutral surface — chips, secondary fills |
| `light` | `255 255 255` | `#ffffff` | `0 0 0` | `#000000` | Used as "opposite of dark" sentinel (e.g. button text on `primary`) |
| `dark` | `20 22 26` | `#14161a` | `229 231 235` | `#e5e7eb` | Opposite of `light` — high-contrast text on neutrals |
| `default-border` | `209 213 219` | `#d1d5db` | `33 33 33` | `#212121` | Default `*` border (set on `box-sizing: border-box` globals) |

**One implication for us:** in dark mode, `default-border` collapses into `--immich-dark-gray` so a card next to a card visually merges unless you add a subtle `bg-immich-dark-gray` on the card surface. That's intentional — Immich's dark UI is mostly *separation by surface tone*, not by hairlines.

### 1.3 Neutral utility scale (Tailwind v4 defaults)

Immich relies on the default Tailwind palette for `gray-50..900`, `slate-*`, `red-*`, `blue-*`, `green-*`, `amber-*`, `orange-*`, `indigo-*`, `purple-*`, `pink-*`, `yellow-*`, `neutral-*`, `zinc-*` — all stored as **OKLCH** values in v4 (e.g. `--color-gray-900: oklch(21% .034 264.665)`). We get these for free by upgrading to Tailwind v4 (or by listing them in our v3 config; see §6). We do not need to redefine them.

The most-used neutrals in Immich's components:
- `--color-gray-200` (`oklch(92.8% .006 264.531)`) — light-mode card surface alternate
- `--color-gray-500` (`oklch(55.1% .027 264.364)`) — secondary label text in light mode
- `--color-gray-600..900` — dark-mode surfaces (`gray-600` = button-disabled bg in dark, `gray-800` = disabled bg, `gray-900` = optional card surface)
- `--color-slate-200` — `immich-form-input` background in light mode

---

## 2. Typography

Immich preloads **Overpass** (sans) and **Overpass Mono** as the entire UI font stack. Both are local TTF assets:

```html
<link rel="preload" as="font" type="font/ttf" href="/_app/immutable/assets/Overpass.DCP28BvT.ttf" crossorigin="anonymous" />
<link rel="preload" as="font" type="font/ttf" href="/_app/immutable/assets/OverpassMono.XkUhFDDw.ttf" crossorigin="anonymous" />
```

CSS:
```css
@font-face { font-family: Overpass; src: url(...) format("truetype"); font-display: swap; }
/* Body and most text */
:root { --font-immich-mono: Overpass Mono, monospace; }
body { font-family: Overpass, sans-serif; }
.font-mono, code, pre { font-family: var(--font-immich-mono); }
```

### 2.1 Type scale (Tailwind v4 defaults that Immich consumes)

| Class | Size | Line-height |
|---|---|---|
| `text-xs`  | `0.75rem` (12px)  | `1/0.75` (16px) |
| `text-sm`  | `0.875rem` (14px) | `1.25/0.875` (20px) |
| `text-base`| `1rem` (16px)     | `1.5` (24px) |
| `text-lg`  | `1.125rem` (18px) | `1.75/1.125` (28px) |
| `text-xl`  | `1.25rem` (20px)  | `1.75/1.25` (28px) |
| `text-2xl` | `1.5rem` (24px)   | `2/1.5` (32px) |
| `text-3xl` | `1.875rem` (30px) | `1.2` (36px) |
| `text-4xl` | `2.25rem` (36px)  | `2.5/2.25` (40px) |
| `text-5xl` | `3rem` (48px)     | `1` (48px) |
| `text-6xl` | `3.75rem` (60px)  | `1` (60px) |

### 2.2 Weights

`300 / 400 / 500 / 600 / 700 / 800` — `--font-weight-{light,normal,medium,semibold,bold,extrabold}`.

Form labels use `medium` (500). Headings tend to use `semibold` or `bold`. Body is `normal`.

### 2.3 Tracking / leading utilities Immich actually uses

- `--tracking-tight: -0.025em` — used on large headings
- `--tracking-wider: 0.05em`  — used on uppercase eyebrows
- `--leading-relaxed: 1.625`  — used on body paragraphs in setting cards

---

## 3. Spacing, radii, shadows

### 3.1 Spacing

Base unit: `--spacing: 0.25rem` (4px). Every Tailwind spacing utility (`p-4`, `gap-6`, `space-y-3`) multiplies that. Immich relies on the default 0..12 + 16, 24, 32 ramp; their own components are dense at `p-2 / p-3 / p-4` for buttons and `p-6 / p-8` for cards.

### 3.2 Border radius

| Token | Value | Where used |
|---|---|---|
| `--radius-md`  | `0.375rem` (6px)  | Compact UI: chips, small buttons |
| `--radius-lg`  | `0.5rem` (8px)    | Default button + select |
| `--radius-xl`  | `0.75rem` (12px)  | **`.immich-form-input` and most inputs** — Immich's most distinctive radius |
| `--radius-2xl` | `1rem` (16px)     | Cards, modal dialogs |
| `--radius-3xl` | `1.5rem` (24px)   | Hero / login card on `/setup`, big tiles |

**Tell.** Immich form inputs use `rounded-xl` (12px) — *not* `rounded-md` (which most UIs default to). Mirror that.

### 3.3 Shadows

Tailwind v4 default `shadow-{sm,md,lg,xl,2xl}` token set is what Immich uses (`box-shadow` utilities), plus colored shadows for primary buttons:

```css
.shadow-primary\/20 { --tw-shadow-color: rgb(var(--immich-ui-primary)); /* + opacity */ }
```

i.e. **primary buttons get a soft blue shadow** (`shadow-md shadow-immich-primary/20`) — that's the second visual signature, after the dark-mode near-black.

### 3.4 Motion

```css
--default-transition-duration: 0.15s;
--default-transition-timing-function: cubic-bezier(.4, 0, .2, 1);   /* "ease-in-out" */
--ease-out: cubic-bezier(0, 0, .2, 1);
--ease-in-out: cubic-bezier(.4, 0, .2, 1);
```

Immich uses these defaults; we should not override. Tailwind v4 ships them as v4 defaults — v3 needs them re-declared.

---

## 4. Layout

### 4.1 Top-bar height

```css
--navbar-height: calc(4.5rem + 4px);    /* 76px desktop */
--navbar-height-md: calc(4.5rem - 10px); /* 62px tablet */
```

Pages reserve top padding with `pt-(--navbar-height)`. We'll mirror this for `<AppShell>`'s top bar.

### 4.2 Sidebar pattern

Immich does **not** use a left sidebar on every page — its sidebar is the album/library nav inside `/photos`, accessed via a hamburger on mobile and a slide-out drawer with icon-then-label items. On settings pages, layout switches to a centered single-column with the navbar still present.

For immich-extended, our needs are simpler (Rules / Activity / Settings / Sign out), so we adopt a **persistent left sidebar on desktop, top-bar burger on mobile**, with the same visual style as Immich's drawer:

- Sidebar width: `w-64` (256px) desktop; collapses to icon-only `w-16` (64px) under `lg:`, fully hidden under `md:` with burger.
- Item: `flex items-center gap-3 px-3 py-2 rounded-xl hover:bg-immich-dark-primary/10` (or `hover:bg-gray-100` in light mode). Active = `bg-immich-dark-primary/20 text-immich-dark-primary`.
- Icon size: `h-5 w-5`, paired with `text-sm font-medium`.

### 4.3 Card pattern

```html
<div class="rounded-2xl bg-white dark:bg-immich-dark-gray
            p-6 shadow-sm
            border border-immich-ui-default-border dark:border-immich-dark-gray">
  …
</div>
```

Cards in dark mode rely on surface contrast (`#212121` card on `#0a0a0a` body), not on borders.

### 4.4 Modal pattern

`rounded-3xl` (24px), full-bleed backdrop with `bg-black/50`, content max-width `max-w-md`, top-bar with title + dismiss `×`, body padded `p-6`, action row `gap-2 justify-end pt-4 border-t`.

### 4.5 Form input (the most-cited Immich-style element)

```html
<input class="w-full rounded-xl bg-slate-200 dark:bg-gray-600
              text-sm
              px-3 py-3
              focus:outline-none focus:ring-2 focus:ring-immich-primary
              disabled:cursor-not-allowed disabled:bg-gray-400 disabled:text-gray-100" />
```

Notes for our wrapper:
- `rounded-xl`, not `rounded-md`.
- Filled, **not bordered**, in resting state. Border only appears on `focus:` via the ring.
- Focus ring color: `immich-primary` (light) / `immich-dark-primary` (dark, via `dark:focus:ring-immich-dark-primary`).
- Padding is generous: `px-3 py-3` (12px / 12px).

### 4.6 Label pattern

```html
<label class="text-sm font-medium text-gray-500 dark:text-gray-300">
```

— gray label above the input, never floating, never overlaid.

### 4.7 Button hierarchy

Immich defines variants implicitly via Tailwind utility classes; their `Button.svelte` component picks the right combo. Mapping to our planned `<Button variant>`:

| Variant | Resting | Hover | Active / Pressed | Disabled |
|---|---|---|---|---|
| `primary`     | `bg-immich-primary text-white shadow-md shadow-immich-primary/20` | `bg-immich-primary/90` | `bg-immich-primary/80` | `bg-gray-400 text-gray-100 cursor-not-allowed` |
| `secondary`   | `bg-slate-200 dark:bg-gray-600 text-immich-fg dark:text-immich-dark-fg` | `bg-slate-300 dark:bg-gray-500` | `bg-slate-400 dark:bg-gray-400` | same disabled as above |
| `destructive` | `bg-immich-ui-danger text-white` | `bg-immich-ui-danger/90` | `bg-immich-ui-danger/80` | same disabled |
| `ghost`       | `bg-transparent text-immich-primary dark:text-immich-dark-primary` | `bg-immich-primary/10 dark:bg-immich-dark-primary/10` | `bg-immich-primary/20 dark:bg-immich-dark-primary/20` | `opacity-50 cursor-not-allowed` |

All variants share: `rounded-lg px-4 py-2 text-sm font-medium transition focus:outline-none focus:ring-2 focus:ring-immich-primary dark:focus:ring-immich-dark-primary`.

### 4.8 Toast pattern

Immich uses a top-right stack, `rounded-2xl`, semantic background tint (e.g. `bg-immich-ui-success/10` border-left `border-l-4 border-immich-ui-success`), 250 ms fade. We don't need toasts for cycle 4 — record this for later.

---

## 5. Dark-mode-first

Immich's `<html>` ships with class `dark` by default at SSR time; the toggle writes to `localStorage` and applies/removes the class on `<html>`. We will mirror that:

- `<html class="dark">` in `web/index.html` so the first paint is dark.
- `<AppShell>` reads/writes `localStorage.theme` (`"dark" | "light"`), and toggles the class on `document.documentElement`.
- Tailwind config: `darkMode: 'class'`.
- All components must declare their dark variant *and* light variant explicitly; never assume the inverse.

---

## 6. `tailwind.config.js` snippet (drop-in for T17)

We're currently on Tailwind v3 (see `web/tailwind.config.js`). The most cautious path is to **stay on v3** for T17 and replicate Immich's v4 tokens manually. Upgrading to v4 is a separate decision (out of scope for cycle 4).

Below is the complete `theme.extend` block that T17 should land. Colors are declared as `rgb(var(--name) / <alpha-value>)` so opacity modifiers like `bg-immich-primary/50` work.

```js
// web/tailwind.config.js — REPLACEMENT
/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        // Brand
        "immich-primary": "rgb(var(--immich-primary) / <alpha-value>)",
        "immich-dark-primary": "rgb(var(--immich-dark-primary) / <alpha-value>)",
        "immich-bg": "rgb(var(--immich-bg) / <alpha-value>)",
        "immich-fg": "rgb(var(--immich-fg) / <alpha-value>)",
        "immich-dark-bg": "rgb(var(--immich-dark-bg) / <alpha-value>)",
        "immich-dark-fg": "rgb(var(--immich-dark-fg) / <alpha-value>)",
        "immich-dark-gray": "rgb(var(--immich-dark-gray) / <alpha-value>)",
        // Semantic
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
        sans: ["Overpass", "ui-sans-serif", "system-ui", "sans-serif"],
        mono: ["Overpass Mono", "ui-monospace", "SFMono-Regular", "Menlo", "monospace"],
      },
      borderRadius: {
        // Tailwind v3 already has md/lg/xl/2xl/3xl matching Immich's values;
        // declare only if drift is noticed. Leave defaults.
      },
      boxShadow: {
        "primary-glow": "0 4px 6px -1px rgb(var(--immich-ui-primary) / 0.2), 0 2px 4px -2px rgb(var(--immich-ui-primary) / 0.2)",
      },
      transitionTimingFunction: {
        "immich": "cubic-bezier(.4, 0, .2, 1)",
      },
      spacing: {
        "navbar": "calc(4.5rem + 4px)",
        "navbar-md": "calc(4.5rem - 10px)",
      },
    },
  },
  plugins: [],
};
```

### 6.1 Global CSS additions for `web/src/index.css`

```css
@tailwind base;
@tailwind components;
@tailwind utilities;

@layer base {
  :root, .light {
    --immich-primary: 66 80 175;
    --immich-bg: 255 255 255;
    --immich-fg: 0 0 0;
    --immich-dark-primary: 172 203 250;
    --immich-dark-bg: 10 10 10;
    --immich-dark-fg: 229 231 235;
    --immich-dark-gray: 33 33 33;
    /* semantic, light */
    --immich-ui-primary: 66 80 175;
    --immich-ui-success: 16 188 99;
    --immich-ui-danger: 200 60 60;
    --immich-ui-warning: 216 143 64;
    --immich-ui-info: 8 111 230;
    --immich-ui-muted: 161 161 161;
    --immich-ui-gray: 246 246 246;
    --immich-ui-dark: 20 22 26;
    --immich-ui-light: 255 255 255;
    --immich-ui-default-border: 209 213 219;
  }
  .dark {
    --immich-ui-primary: 172 203 250;
    --immich-ui-success: 72 237 152;
    --immich-ui-danger: 246 125 125;
    --immich-ui-warning: 254 197 132;
    --immich-ui-info: 121 183 254;
    --immich-ui-muted: 212 212 212;
    --immich-ui-gray: 33 33 33;
    --immich-ui-dark: 229 231 235;
    --immich-ui-light: 0 0 0;
    --immich-ui-default-border: 33 33 33;
  }
  html, body {
    background-color: rgb(var(--immich-bg));
    color: rgb(var(--immich-fg));
    font-family: "Overpass", ui-sans-serif, system-ui, sans-serif;
  }
  .dark html, .dark body, html.dark, html.dark body {
    background-color: rgb(var(--immich-dark-bg));
    color: rgb(var(--immich-dark-fg));
  }
}
```

Note: we'll bundle Overpass as a webfont in T17 (download from Google Fonts or Immich's own `_app/immutable/assets/Overpass.DCP28BvT.ttf` — re-host locally for offline use; do not hotlink). System sans is the fallback.

---

## 7. Component inventory for T17

Each entry: **status** (`new` = build from scratch in T17 ; `keep` = exists, leave for T21 to restyle ; `restyle` = touch in T17 to align), then short note.

| Component | Status | Where |
|---|---|---|
| `<AppShell>`           | **new** | `web/src/components/AppShell.tsx` — sidebar + topbar + `<Outlet />` (or `{props.children}` in Solid) |
| `<SidebarNav>`         | **new** | child of `<AppShell>`; collapsible on `lg:`, drawer on `md:` |
| `<TopBar>`             | **new** | child of `<AppShell>`; brand mark left, user menu + dark-mode toggle right |
| `<ThemeToggle>`        | **new** | small `<button>` inside `<TopBar>` that toggles `document.documentElement.classList` and writes `localStorage.theme` |
| `<Card>`               | **new** | `web/src/components/ui/Card.tsx`; props: `padding?: "sm" | "md" | "lg"`, default `md` (`p-6`) |
| `<Button>`             | **new** | `web/src/components/ui/Button.tsx`; variants per §4.7; `loading?: boolean` spinner + disabled |
| `<Input>`              | **new** | `web/src/components/ui/Input.tsx`; matches `.immich-form-input` spec |
| `<Label>`              | **new** | `web/src/components/ui/Label.tsx`; just `<label class="text-sm font-medium text-gray-500 dark:text-gray-300">` |
| `<Select>`             | **new** | `web/src/components/ui/Select.tsx`; wraps native `<select>` with same styling as `<Input>` |
| `<Field>`              | **new** | `web/src/components/ui/Field.tsx`; lays out label + input + help/error |
| `<ConfirmDialog>`      | **restyle** | exists at `web/src/components/ConfirmDialog.tsx`; update to use `<Card>` / `<Button>` and `rounded-3xl` |
| `<PeopleMultiSelect>`  | **keep** | exists, T21 will polish |
| `<MapPicker>`          | **keep** | exists, T21 will polish (only minor color-token replacements) |
| `<PeopleContext>`      | **keep** | logic-only, no UI |
| Login page             | **restyle** | exists at `web/src/pages/Login.tsx`; T21 will polish to match Immich login (centered card `rounded-3xl`, two-input form, `<Button variant="primary">`) |
| Setup page             | **restyle** | exists; T21 |
| MeSettings page        | **restyle** | exists; T21 |
| Dashboard / rules list | **restyle** | exists; T21 |
| RuleBuilder            | **restyle** | exists; T20 will overwrite parts of this for block-builder UI |

**T17 deliverable** = everything marked **new** above + `tailwind.config.js` swap + `index.css` swap + `<html class="dark">` default. T17 deliberately leaves restyle work for T21 so the shell can land independently and existing tests stay green.

---

## 8. Page wireframes (ASCII)

Renderings below assume dark mode (the default). Light mode swaps backgrounds but keeps the structure.

### 8.1 Logged-in shell — `/rules`

```
┌───────────────────────────────────────────────────────────────────────────────┐
│ ┃ immich-extended           [search]              [☀/☾]  [user ▾]            │ <- TopBar (76px)
│ ┃                                                                              │
├──┴─────────────────────────────────────────────────────────────────────────────┤
│        │                                                                       │
│ Rules• │   Rules                                                  [+ New rule] │ <- Page header
│ Activ. │                                                                       │
│ Settgs │   ┌─────────────────────────────────────────────────────────────┐   │
│ ──     │   │ ● Paloma (partage)                              poll 5m  ⋯ │   │ <- Card
│ Logout │   │   Last run 19:51 · 360 added · 890 skipped · 0 errors      │   │
│        │   └─────────────────────────────────────────────────────────────┘   │
│        │   ┌─────────────────────────────────────────────────────────────┐   │
│        │   │ ⊘ Paloma (partage Maman)                         paused  ⋯ │   │
│        │   │   No runs yet                                              │   │
│        │   └─────────────────────────────────────────────────────────────┘   │
│        │                                                                       │
│  256px │   (cards: rounded-2xl, bg-immich-dark-gray, p-6, hover lifts ring)    │
└────────┴──────────────────────────────────────────────────────────────────────┘
```

### 8.2 Login — `/login`

```
                       ┌────────────────────────────────┐
                       │                                │
                       │       immich-extended          │  <- text-3xl font-semibold
                       │                                │
                       │   ┌──────────────────────┐    │
                       │   │ email                 │    │  <- <Input> (rounded-xl, bg-slate-200/gray-600)
                       │   └──────────────────────┘    │
                       │   ┌──────────────────────┐    │
                       │   │ password              │    │
                       │   └──────────────────────┘    │
                       │   [    Sign in   ]            │  <- <Button variant=primary> rounded-lg
                       │                                │
                       │   ── or ──                     │
                       │   [   Continue with SSO   ]   │  <- <Button variant=secondary> w-full
                       │                                │
                       └────────────────────────────────┘
                          (Card: rounded-3xl, max-w-md, p-10, shadow-xl)
```

### 8.3 Settings — `/me`

```
┌─ AppShell ──────────────────────────────────────────────────────────────────┐
│  Sidebar | Settings                                                           │
│          | ─────────                                                          │
│          | ┌─ Immich connection ─────────────────────────────────────────┐  │
│          | │  Base URL                                                    │  │
│          | │  [ https://immich.example.com         ]                      │  │
│          | │                                                              │  │
│          | │  API key                                                     │  │
│          | │  [ ••••••••••••                       ]   [ Validate ]      │  │
│          | │                                                              │  │
│          | │  Last validated: 2026-05-27 19:55 UTC                        │  │
│          | └──────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 8.4 Activity (sketch only — full page is T23)

```
┌─ Rules / Paloma (partage) / Activity ───────────────────────────────────────┐
│  Recent runs                                                                 │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │ ▶ 19:51 · 10m 46s · 1250 evaluated · 310 added · 940 skipped · ok   │  │
│  │ ▶ 19:46 · 10m 12s · 1250 evaluated · 0 added · 1250 skipped · ok    │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
│                                                                              │
│  Recent decisions                                                            │
│  ┌─────────────────────────────────────────────────────────────────────┐  │
│  │ IMG_2942.jpg     added     19:51:48                                  │  │
│  │ IMG_2941.jpg     skipped   19:51:48   reason: people-mismatch        │  │
│  └─────────────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────────┘
```

---

## 9. Open questions / nice-to-haves for later

1. **Webfont licensing.** Overpass is OFL-licensed (open source, free to redistribute). Bundle a self-hosted copy under `web/public/fonts/`, not a Google Fonts link, so the app works air-gapped.
2. **OKLCH vs RGB.** Immich's neutral palette is OKLCH (Tailwind v4 default). We're on v3 which exports `rgb()` defaults. The visual difference at this scale is invisible to the eye; keep v3 RGB for now.
3. **System dark-mode detection.** Immich's toggle is *explicit* — no `prefers-color-scheme` auto-follow. T17 will mirror that: dark by default, manual toggle, persisted in `localStorage.theme`.
4. **Logo asset.** Immich uses a custom SVG mark + wordmark. We can ship a minimal monogram (e.g. "⊞ ie") until/unless someone designs a real one.
5. **Accessibility.** Immich's contrast ratios meet WCAG AA for both modes on text; `--immich-dark-fg` (#e5e7eb) on `--immich-dark-bg` (#0a0a0a) is ~14:1. We inherit that as long as we don't substitute lower-contrast neutrals.

---

## 10. Verification checklist for T17

When T17 lands, verify by:

- [ ] `web/index.html` has `<html class="dark">`.
- [ ] `web/tailwind.config.js` has `darkMode: 'class'` and the `colors` block from §6.
- [ ] `web/src/index.css` declares the `:root`/`.dark` blocks from §6.1.
- [ ] `web/src/components/AppShell.tsx`, `<SidebarNav>`, `<TopBar>`, `<ThemeToggle>` exist.
- [ ] `web/src/components/ui/{Card,Button,Input,Label,Select,Field}.tsx` exist.
- [ ] `npm run typecheck` clean.
- [ ] `npm test -- --run` — new vitest snapshot tests for each `<Button>` variant pass; `<AppShell>` renders sidebar+topbar+children; `<Card>` renders with correct radius class.
- [ ] `npm run build` succeeds; bundle size delta noted (expected ~+5..10 kB pre-gzip).
- [ ] Manual browser smoke: existing pages still render readably (unstyled is OK; T21 polishes).

---

*End of doc. Authored 2026-05-27 by POSTSHIP-T16. Snapshot sources: `https://immich.${DOMAIN}/_app/immutable/assets/0.HmSofUKY.css` (128 kB) and `https://immich.${DOMAIN}/` HTML (5 kB).*
