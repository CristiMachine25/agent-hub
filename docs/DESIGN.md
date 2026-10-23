# Relay: design plan

> 2026-10-23: renamed to Relay. Name history: AI Hopper → Baton → Switchyard → Agent Hub → Relay.

## The one question

Every screen serves one question: **which account should I use right now, and how do I get into it?**
The answer must be readable in under two seconds, and the launch must be one click (or `Ctrl/Cmd+K`, type, Enter).

## Concept: the fuel board

Each account is a tank. Each rate-limit window (5-hour session, weekly, weekly Opus, monthly premium
requests) is a gauge on that tank. The app is the board you glance at before picking which tank to draw from.

- Meters show **what is left**, not what is used. "36% left" is the number people act on.
- Meters are **segmented** (20 cells of 5%), like a battery or a fuel strip. Segments make small differences
  countable at a glance and look like nothing else in a generic dashboard. This is where the boldness goes.
- **Color = how much is left.** The fill color of a meter comes from a four-step scale, and nothing else in the
  UI uses those colors. Accounts carry an identity color, but only as a small mark and as a chart series.
- The recommendation ("Use now") is the single loud moment: a big meter, the account name large, one primary
  button. Everything else is quiet rows on a flat surface separated by lines, not floating cards.

## Palette

Dark is the default look, whatever the OS setting: a slightly cool near-black taken from the app icon's charcoal
tile, not grey and not blue-black. Surfaces step up in a fixed order, well < ground < panel < raised, each step
far enough apart that a panel never merges into the ground. Light (paper) and System stay available in Settings.

Interactive color (focus, selection, primary button) is periwinkle, from the icon's route (`#7397FB` family),
deliberately outside the quota scale, so "periwinkle" always means "you can act here" and green-to-red always
means "quota left". It is also far from Anthropic's terracotta, so the app never looks like it borrows their branding.

| Name | Role | Dark (default) | Light |
|---|---|---|---|
| Ground / Paper | App ground, terminal background | `#0E0F12` | `#F5F3EE` |
| Panel | Sidebar, panels, pane headers, overlays | `#16181C` | `#FBFAF7` |
| Well | Side lists (accounts, terminal sessions), code previews | `#0A0B0D` | `#ECE9E2` |
| Raised | Hover, selected rows, tooltips | `#212329` | `#EEEBE4` |
| Line | Borders, dividers | `#2B2E35` | `#DCD7CD` |
| Cell off | Unlit meter cells (the meter well) | `#272A31` | `#E2DED5` |
| Text | Primary text | `#E9EAEF` | `#1F1D1A` |
| Muted | Secondary text | `#9DA1AC` | `#605C54` |
| Faint | Hints, tertiary text | `#868A95` | `#6D6960` |
| Periwinkle | Actions, focus ring, selection | `#86A0FB` | `#4556D6` |
| On periwinkle | Text on the primary button | `#0A0F28` | `#FFFFFF` |
| Quota scale | Full / Half / Low / Empty (left) | `#5ED6A2` `#E6C25F` `#EF904E` `#F36D6D` | `#16804A` `#926800` `#B6500E` `#C42E34` |

Full leans toward the icon's mint; the four quota colors were re-tuned for the near-black ground.
Thresholds for the quota scale (percent left): Full ≥ 50, Half 20–49, Low 5–19, Empty < 5.
Every color state also carries a word ("Limit reached", "Low") so color is never the only signal.

Contrast (WCAG 2.x ratios, computed from the hex values above):

| Pair | Dark | Light |
|---|---|---|
| Text on ground / panel / raised | 15.9 / 14.8 / 13.1 | 15.2 / 16.1 / 14.1 |
| Muted on ground / panel / well / raised | 7.4 / 6.9 / 7.6 / 6.1 | 6.0 / 6.4 / 5.5 / 5.6 |
| Faint on ground / panel / well / raised | 5.6 / 5.2 / 5.7 / 4.6 | 4.9 / 5.2 / 4.5 / 4.6 |
| On periwinkle text on periwinkle | 7.6 | 5.9 |
| Periwinkle focus ring on ground / panel | 7.7 / 7.1 | 5.3 / 5.7 |
| Full / Half / Low / Empty on cell off | 8.0 / 8.4 / 6.0 / 4.9 | 3.7 / 3.7 / 3.8 / 4.1 |
| Line on ground (non-text, kept subtle) | 1.4 | 1.3 |

The xterm theme reads ground, text and periwinkle from the same tokens (cursor in periwinkle, selection at 30%
periwinkle); its ANSI colors are tuned per theme in `src/terminals/registry.ts`. Charts read grid, axis and
tooltip colors from the tokens through `src/charts/theme.ts`.

Account identity colors come from a validated categorical set (dataviz reference palette, dark steps):
`#3987E5 #D95926 #199E70 #C98500 #D55181 #9085E9 #E66767 #008300`.

## Type

Claude's interface pairs a book serif for headings and answers with a plain sans for controls. Relay does the same:

- **Source Serif 4** (variable with the optical-size axis, bundled with @fontsource): screen titles, section
  headings, the account name and the big percentage in "Best account right now", the app name. Semibold. The
  optical-size build draws the 44px figure with the display cut and the 17–20px headings with the text cut.
- **Hanken Grotesk** (variable): everything you click or scan: buttons, rows, labels, small numbers. Its figures
  are tabular by default, so countdowns don't jitter, and it is neutral enough next to the serif.
- **IBM Plex Mono** (400 and 600): terminals and anything you could paste into one: command lines, paths, env
  vars. Terminal bold uses the real 600 weight instead of a synthesized one.

Neither bundled sans nor serif subset carries stylistic sets or character variants, so there are none to enable;
`tnum` stays on globally as a guard. Text is antialiased (grayscale) on every theme.

Sizes: 13.5px/19px base (desktop density), 12px secondary, 17px serif section and dialog titles, 20px serif screen
titles and the app name (-0.01em), 44px serif hero figure (-0.025em). Serif text elsewhere tracks -0.008em.
Sentence case everywhere. No all-caps eyebrows.

## Icon

Relay is the approved app mark: a periwinkle route hands work to a green route inside a rounded charcoal tile.
The source artwork is `app-icon.png`. Preserve that artwork when generating platform sizes; the in-app mark
imports a small generated copy through `src/components/Logo.tsx`. Run `npm run icons:build` to regenerate the
desktop icon formats, interface copy, and the website's favicon, navigation, and Apple touch icon.

## Terminal grid

Panes are flat panels on the ground color with a 36px header: account dot, name, provider, a 10-cell meter for
the binding window with its reset countdown, status, then icon buttons. The focused pane gets a periwinkle
border. Headers drop the provider, then the meter and status, as a pane gets narrower (container queries, not
window breakpoints). A pane whose account hits its limit shows one calm banner with a single "Continue on X"
button; there is no red alarm. The xterm theme comes from the same tokens.

## Layout

208px sidebar; the button beside Settings or Ctrl/Cmd+B collapses it to icons (60px, 92px on macOS to
clear the window buttons), and it starts collapsed below 1000px. Every screen has a fixed 56px toolbar with
its title (and actions such as Refresh on Knowledge); the content scrolls below it. Comfortable at 1240×820,
usable at 900×600, never scrolls horizontally.

```
┌─────────────────┬──────────────────────────────────────────────────────────────────────┐
│ ◆ Relay         │ Overview                                          Find and launch ⌘K │
│                 ├──────────────────────────────────────────────────────────────────────┤
│ Overview        │ ┌ Best account right now ──────── [Coding] Long refactor  Quick …  ┐ │
│ Accounts        │ ├────────────────────────────────────────────┬─────────────────────┤ │
│ Knowledge       │ │ ● Work Max 20x   Claude Code, Max 20x      │ If not that one     │ │
│ Skills          │ │ 36%  ▮▮▮▮▮▮▮▯▯▯▯▯▯▯▯▯▯▯▯▯                  │ ● Personal Pro   ▶  │ │
│ Terminals       │ │ left of 5-hour session, resets in 2h 14m   │ ● ChatGPT Pro    ▶  │ │
│ Notifications 3 │ │ Weekly ▮▮▮▮▮▮▯▯▯▯  Weekly Opus ▮▮▮▮▮▮▮▮▯▯  │                     │ │
│                 │ │ 59% of weekly left, resets in 3 days       │                     │ │
│                 │ │ [▶ Launch Claude]  [Resume last session]   │                     │ │
│                 │ └────────────────────────────────────────────┴─────────────────────┘ │
│                 │ Use it or lose it: H2 Max has 70% of its weekly left, resets in 9h   │
│                 │ All accounts                                     [+ Add account]     │
│                 │ ● Work Max 20x   5-hour ▮▮▮▮▯▯▯  Weekly ▮▮▮▮▮▯  Opus ▮▯▯  [▶]        │
│                 │ ● Personal Pro   5-hour ▮▮▮▮▮▮▮  Weekly ▯▯▯▯▯▯  Low       [▶]        │
│ ↻ Usage updated │ …                                                                    │
│   2m ago        │ Weekly and monthly limits used     Most used, last 30 days           │
│ Settings    [«] │ [chart]                            [bars]                            │
└─────────────────┴──────────────────────────────────────────────────────────────────────┘
```

Accounts screen is master/detail: provider-grouped list on the left, the selected account on the right.
Adding an account is a two-step dialog (find on this computer, or create a new isolated profile).

## Principles

1. **Answer first.** The recommendation and its launch button sit at the top of the default screen.
2. **Structure encodes data.** Meter length and color are the remaining quota. Row order is the recommendation order.
3. **Quiet chrome.** Flat surfaces, 1px lines, one radius (6px), no gradients, no shadows except on overlays.
4. **Show what ran.** Every launch toasts the exact command line in mono, with a copy button.
5. **Motion only for change.** Meters animate when a value changes; nothing else moves. `prefers-reduced-motion` disables it.
6. **Keyboard first.** Visible periwinkle focus ring, `Ctrl/Cmd+K` palette, `1–7` jump between screens, Enter launches.

## Review against the brief (revisions made)

- First draft used a near-black ground with a single green accent: that is a known template look. The accent
  is periwinkle, keeping green exclusively for "plenty left". The ground went from warm charcoal to the icon's
  cool near-black for launch; it stays a tinted black with stepped surfaces, never a single flat black.
- First draft had a card per account in a grid. Replaced with dense rows so ten accounts fit without scrolling
  and the windows line up in columns for comparison.
- Dropped a planned uppercase "PROVIDER" eyebrow over groups; groups use the provider name in normal case
  with the vendor's small identity mark.
