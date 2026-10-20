# AI Hopper: design plan

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

Interactive color (focus, selection, primary button) is violet, deliberately outside the quota scale, so
"violet" always means "you can act here" and green-to-red always means "quota left".

| Name | Role | Dark (default) | Light |
|---|---|---|---|
| Night shift | App ground | `#0E141E` | `#EEF1F6` |
| Deck | Panels, sidebar, rows on hover | `#161E2B` | `#FFFFFF` |
| Rivet | Lines, empty meter cells | `#263245` | `#D5DCE6` |
| Paper / Ink | Primary text | `#E7EBF2` | `#121926` |
| Hop violet | Actions, focus ring, selection | `#A193FF` | `#5A44DB` |
| Quota scale | Full / Half / Low / Empty (left) | `#3DD39B` `#E6BE45` `#F28A3C` `#F25C6E` | `#0D8F62` `#A97C08` `#C25A12` `#CC2F49` |

Muted text: `#93A0B5` dark / `#556275` light (both pass 4.5:1 on their ground).
Thresholds for the quota scale (percent left): Full ≥ 50, Half 20–49, Low 5–19, Empty < 5.
Every color state also carries a word ("Limit reached", "Low") so color is never the only signal.

Account identity colors come from a validated categorical set (dataviz reference palette, dark steps):
`#3987E5 #D95926 #199E70 #C98500 #D55181 #9085E9 #E66767 #008300`.

## Type

- **Hanken Grotesk** (variable, bundled with @fontsource): all UI text and numbers. A compact, slightly
  technical grotesk with true tabular figures, so countdowns and percentages do not jitter as they tick.
  Big numbers in the "Use now" panel use weight 700 at 44px.
- **IBM Plex Mono**: only for things you could paste into a terminal: command lines, paths, env vars.
  Never for labels.

Sizes: 13px base (desktop density), 12px secondary, 15px section titles, 20px screen titles, 44px hero figure.
Sentence case everywhere. No all-caps eyebrows.

## Layout

Fixed 208px sidebar (collapses to icons below 1000px), content scrolls. Comfortable at 1240×820,
usable at 900×600, never scrolls horizontally.

```
┌────────────┬──────────────────────────────────────────────────────────────┐
│ ◆ AI Hopper│ Overview                                    Refresh  Ctrl K   │
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
6. **Keyboard first.** Visible violet focus ring, `Ctrl/Cmd+K` palette, `1–4` jump between screens, Enter launches.

## Review against the brief (revisions made)

- First draft used a near-black ground with a single green accent: that is a known template look. Moved the
  ground to a blue-ink tone and made the accent violet, keeping green exclusively for "plenty left".
- First draft had a card per account in a grid. Replaced with dense rows so ten accounts fit without scrolling
  and the windows line up in columns for comparison.
- Dropped a planned uppercase "PROVIDER" eyebrow over groups; groups use the provider name in normal case
  with the vendor's small identity mark.
