# Switchyard: design plan

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

Dark is the default look, whatever the OS setting: a warm charcoal close to Claude's dark mode, not blue-black.
Light (paper) and System stay available in Settings.

Interactive color (focus, selection, primary button) is a soft periwinkle, deliberately outside the quota scale,
so "periwinkle" always means "you can act here" and green-to-red always means "quota left". It is also far from
Anthropic's terracotta, so the app never looks like it borrows their branding.

| Name | Role | Dark (default) | Light |
|---|---|---|---|
| Charcoal / Paper | App ground, terminal background | `#1A1918` | `#F5F3EE` |
| Panel | Sidebar, panels, pane headers | `#222120` | `#FBFAF7` |
| Raised | Hover, selected rows | `#2A2826` | `#EEEBE4` |
| Line | Borders, dividers | `#35322F` | `#DCD7CD` |
| Text | Primary text | `#ECEAE4` | `#1F1D1A` |
| Muted | Secondary text | `#A3A097` | `#605C54` |
| Periwinkle | Actions, focus ring, selection | `#9AA5FF` | `#4556D6` |
| Quota scale | Full / Half / Low / Empty (left) | `#5EC98C` `#E2BE5C` `#EC8D4A` `#F06868` | `#16804A` `#926800` `#B6500E` `#C42E34` |

Muted text passes 4.5:1 on its ground in both themes. The quota colors were re-tuned to sit on warm charcoal
(slightly softer and warmer than the old blue-ink set).
Thresholds for the quota scale (percent left): Full ≥ 50, Half 20–49, Low 5–19, Empty < 5.
Every color state also carries a word ("Limit reached", "Low") so color is never the only signal.

Account identity colors come from a validated categorical set (dataviz reference palette, dark steps):
`#3987E5 #D95926 #199E70 #C98500 #D55181 #9085E9 #E66767 #008300`.

## Type

Claude's interface pairs a book serif for headings and answers with a plain sans for controls. Switchyard does the same:

- **Source Serif 4** (variable, bundled with @fontsource): screen titles, section headings, the account name and
  the big percentage in "Best account right now", the app name. Semibold, tabular figures for numbers that tick.
- **Hanken Grotesk** (variable): everything you click or scan: buttons, rows, labels, small numbers. It stays
  because its tabular figures keep countdowns from jittering, and it is neutral enough next to the serif.
- **IBM Plex Mono**: terminals and anything you could paste into one: command lines, paths, env vars.

Sizes: 13px base (desktop density), 12px secondary, 17px serif section titles, 24px serif screen titles,
44px serif hero figure. Sentence case everywhere. No all-caps eyebrows.

## Icon

A rail switch on a gradient charcoal tile: one periwinkle track enters from the left and is switched up onto the
branch whose signal is green (quota left), while the amber and red branches stay idle. Reads at 16px. Source: `app-icon.svg`
(`npx tauri icon app-icon.svg` regenerates the platform icons), mirrored in `src/components/Logo.tsx`.

## Terminal grid

Panes are flat panels on the ground color with a 36px header: account dot, name, provider, a 10-cell meter for
the binding window with its reset countdown, status, then icon buttons. The focused pane gets a periwinkle
border. Headers drop the provider, then the meter and status, as a pane gets narrower (container queries, not
window breakpoints). A pane whose account hits its limit shows one calm banner with a single "Continue on X"
button; there is no red alarm. The xterm theme comes from the same tokens.

## Layout

Fixed 208px sidebar (collapses to icons below 1000px), content scrolls. Comfortable at 1240×820,
usable at 900×600, never scrolls horizontally.

```
┌────────────┬──────────────────────────────────────────────────────────────┐
│ ◆ Switchyard    │ Overview                                    Refresh  Ctrl K   │
│            │ ┌ Use now ───────────────────────────────────────────────────┐│
│ Overview   │ │ For: [Coding] Long refactor  Quick question  Research  …    ││
│ Accounts   │ │                                                              ││
│ Alerts  3  │ │ ● Work Max 20x        ▮▮▮▮▮▮▮▯▯▯▯▯▯▯▯▯▯▯▯▯  36% left        ││
│ Settings   │ │ Claude · Max 20x      5-hour session · resets in 2h 14m      ││
│            │ │ "59% of weekly left, resets in 3 days"   [Launch Claude]     ││
│            │ │ Next best: Personal Pro (Launch) · ChatGPT Pro (Launch)      ││
│            │ └──────────────────────────────────────────────────────────────┘│
│            │ ⚠ Use it or lose it: H2 Max has 70% weekly left, resets in 9h  │
│            │ All accounts                                                    │
│            │ ● Work Max 20x   5-hour ▮▮▮▮▯▯▯  Weekly ▮▮▮▮▮▯  Opus ▮▯▯  [▶]   │
│            │ ● Personal Pro   5-hour ▮▮▮▮▮▮▮  Weekly ▯▯▯▯▯▯ Low          [▶] │
│            │ …                                                               │
│ Updated 2m │ Usage over time [chart]          Most used (30 days) [bars]    │
│ ago        │                                                                 │
└────────────┴──────────────────────────────────────────────────────────────┘
```

Accounts screen is master/detail: provider-grouped list on the left, the selected account on the right.
Adding an account is a two-step dialog (find on this computer, or create a new isolated profile).

## Principles

1. **Answer first.** The recommendation and its launch button sit at the top of the default screen.
2. **Structure encodes data.** Meter length and color are the remaining quota. Row order is the recommendation order.
3. **Quiet chrome.** Flat surfaces, 1px lines, one radius (6px), no gradients, no shadows except on overlays.
4. **Show what ran.** Every launch toasts the exact command line in mono, with a copy button.
5. **Motion only for change.** Meters animate when a value changes; nothing else moves. `prefers-reduced-motion` disables it.
6. **Keyboard first.** Visible violet focus ring, `Ctrl/Cmd+K` palette, `1–7` jump between screens, Enter launches.

## Review against the brief (revisions made)

- First draft used a near-black ground with a single green accent: that is a known template look. Moved the
  ground to a blue-ink tone and made the accent violet, keeping green exclusively for "plenty left".
- First draft had a card per account in a grid. Replaced with dense rows so ten accounts fit without scrolling
  and the windows line up in columns for comparison.
- Dropped a planned uppercase "PROVIDER" eyebrow over groups; groups use the provider name in normal case
  with the vendor's small identity mark.
