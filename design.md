# Verio — Design Tokens

Reference document for any designer or AI producing UI for Verio.
This file contains only tokens. No layout, no components, no behavior.

## Brand direction

Warm, quiet, serious. Dark mode only. Not gamer RGB, not Discord, not a
corporate SaaS dashboard. Precision-instrument feel — closer to a piece of
hi-fi audio gear than to a chat app.

## Color

### Base surfaces

| Token           | Hex       | Use                                                    |
|-----------------|-----------|--------------------------------------------------------|
| `--bg`          | `#171412` | App background. Warm espresso, not pure black.         |
| `--surface`     | `#211c18` | Cards, panels, list rows.                              |
| `--surface-2`   | `#2a2420` | Hover states, inset wells, dropdown backgrounds.       |
| `--surface-3`   | `#332b25` | Deepest wells, active row backgrounds, pressed states. |

### Borders

| Token             | Hex       | Use                                     |
|-------------------|-----------|-----------------------------------------|
| `--border`        | `#2a241f` | Default hairline. Barely visible.       |
| `--border-2`      | `#3a322b` | Hovered or focused borders.             |

### Text

| Token           | Hex       | Use                                              |
|-----------------|-----------|--------------------------------------------------|
| `--text`        | `#e8dfd1` | Primary text. Warm bone, not pure white.         |
| `--text-dim`    | `#7d7166` | Labels, secondary info, timestamps.              |
| `--text-faint`  | `#57504a` | Placeholders, disabled text.                     |

### Semantic

| Token           | Hex       | Use                                                    |
|-----------------|-----------|--------------------------------------------------------|
| `--accent`      | `#c9873f` | Primary action + speaking indicator. Nothing else.     |
| `--accent-hi`   | `#d9994f` | Hover state for accent elements.                       |
| `--accent-lo`   | `#a86f2f` | Pressed state for accent elements.                     |
| `--idle`        | `#5b7a72` | Quiet / connected / at-rest. Mossy green-grey.         |
| `--warn`        | `#b86a4a` | Muted, deafened, non-critical warnings.                |
| `--danger`      | `#8a3a30` | Destructive actions only (leave call, remove peer).    |

### Color rules

- `--accent` is used for exactly two things: **primary actions** and the
  **speaking indicator**. Never decoration, never icons-that-just-look-nice.
- `--idle` is the resting state color for anything connected or quiet.
- `--warn` is for muted / deafened — states that are intentional but
  should be visible.
- `--danger` is only for genuinely destructive actions.
- No glows, no neon, no drop shadows on interactive elements.
- Gradients only on fixed bars (e.g. a bottom dock) and only subtly.

## Typography

### Families

- **Inter** — all UI text: labels, buttons, names, descriptions, headings.
  Weights: 400, 500, 600, 700.
- **JetBrains Mono** — every number, code, and address. This includes:
  room codes, timers, ping/latency, packet counters, hotkey chips,
  server addresses, version strings, sample rates, file durations.
  Weights: 400, 500, 600.

Load via Google Fonts `<link>` (not `@import`).
`font-feature-settings: 'tnum'` on JetBrains Mono where numbers must align.

### Sizes

| Size  | Use                                                          |
|-------|--------------------------------------------------------------|
| 10px  | Micro labels — hotkey chips, small status text.              |
| 11px  | Section labels, uppercase with 0.15em letter-spacing.        |
| 13px  | Body text, buttons, list items.                              |
| 14px  | Primary body text where 13 feels cramped.                    |
| 15px  | Names, headings of components (peer card name, track title). |
| 18px  | Hero headings only (lobby welcome, empty states).            |
| 24px  | Room code display when prominent.                            |

### Rules

- Never use font sizes outside this scale.
- Section labels are always uppercase with `letter-spacing: 0.15em`.
- Room codes and numeric readouts always use JetBrains Mono, never Inter.
- Line height: 1.4 for body, 1.2 for headings, 1.0 for large numerals.

## Spacing

Fixed scale, in pixels:

`4 · 8 · 12 · 16 · 20 · 24 · 32 · 40 · 60`

Never use values outside this scale. If a design needs 18px, use 20px.
If it needs 6px, use 8px.

Common uses:
- 4px — internal padding on tiny elements (hotkey chips, badges).
- 8px — icon-to-label gap, tight list row vertical rhythm.
- 12px — gap between related items in a group.
- 16px — card padding, standard internal spacing.
- 20px — section padding, screen edges on tight layouts.
- 24px — card padding on larger surfaces, screen edges on wider layouts.
- 32px — separation between distinct sections.
- 40px — top padding on a screen's main content area.
- 60px — empty state padding, hero spacing.

## Radii

| Token      | Value | Use                                                    |
|------------|-------|--------------------------------------------------------|
| `--r-sm`   | 5px   | Buttons, inputs, chips, small cards.                   |
| `--r-md`   | 8px   | Standard cards, panels, list containers.               |
| `--r-lg`   | 12px  | Modals, drawers, large surfaces.                       |

Full circles (`border-radius: 50%`) only for avatars and status dots.

## Motion

- Default transition: `140ms ease`.
- Longer transition (larger surfaces): `200ms ease`.
- No bouncy easings. No spring. No overshoot.
- Press feedback: `transform: translateY(1px)` on buttons.
- The **speaking indicator** may use a single pulse animation —
  a ring that expands and fades once every 1.8s. Nothing else animates
  continuously.
- The **live mic meter** may animate its bars while the user is speaking.
- No animated waveforms, no ambient loops, no floating particles.

## Focus and interaction states

Every interactive element must show:

- **Hover** — one step brighter in background or border. No glow.
- **Focus** — 2px `--accent` outline, offset by 2px, visible via
  `:focus-visible` only (not on mouse click).
- **Active / pressed** — 1px vertical translate or one step darker background.
- **Disabled** — `--text-faint` color, 50% opacity, cursor `not-allowed`.

## Scrollbars

Custom only. Never the OS default. Thin, warm-toned:

- Track: transparent (or `--bg` if a track is needed).
- Thumb: `--surface-3` at rest, `--border-2` on hover.
- Width: 6px. Radius: 3px.
- Should be avoidable entirely for lists like the participants grid —
  design so content fits, or wraps, or collapses.

## Icons

- Single-stroke SVG only. Stroke width 1.3–1.5. Round line caps.
- No fills unless the icon is meant to represent a filled state
  (e.g. a filled mute icon when muted).
- No icon fonts. No emoji. No external image assets.
- Size: 14px for inline, 16px for buttons, 20px for header brand marks.

## What to never do

- No gradients on backgrounds of cards or modals.
- No glow effects on hover.
- No animated waveforms, VU meters as decoration, or ambient motion.
- No pure black (`#000`) or pure white (`#fff`).
- No blues, purples, reds, or greens outside the tokens above.
- No serif fonts. No display fonts. No more than two families total.
- No shadows on buttons or inputs. One shadow family only, reserved for
  floating overlays (modals, dropdowns, context menus):
  `0 8px 24px rgba(0, 0, 0, 0.4)`.
- No emoji in UI text.
- No placeholder Lorem Ipsum — use realistic sample content only.