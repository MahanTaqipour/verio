# Verio — Design Tokens

Reference document for any designer or AI producing UI for Verio.
This file contains only tokens. No layout, no components, no behavior.

## Brand direction

Midnight Violet / Electric Lavender / Acid Cyber Lime 60/30/10 glass & tactical
instrumentation system (`1280 × 720` widescreen horizontal shell). Dark mode only.

## Color

### Base surfaces (60% Midnight Violet over Deep Violet Void)

| Token           | Value       | Use                                                    |
|-----------------|-------------|--------------------------------------------------------|
| `--bg`          | `#120d1d`   | Deep anchor void for outer window and rail contrast.   |
| `--surface`     | `#241b3a`   | Primary Midnight Violet panel and card base.           |
| `--surface-2`   | `#2e234a`   | Hover states, inset wells, dropdown backgrounds.       |
| `--surface-3`   | `#392c5a`   | Active rows, pressed states.                           |

### Borders & Glass Speculars (30% Electric Lavender)

| Token              | Value                         | Use                                     |
|--------------------|-------------------------------|-----------------------------------------|
| `--border`         | `rgba(185, 167, 230, 0.22)`   | Default hairline.                       |
| `--border-2`       | `rgba(185, 167, 230, 0.42)`   | Hovered, focused, and glass card edge.  |
| `--accent-lavender`| `#b9a7e6`                     | Glass specular accents, sliders, bars.  |

### Text

| Token              | Value     | Use                                              |
|--------------------|-----------|--------------------------------------------------|
| `--text`           | `#f8fafc` | Primary headings, hero numbers, active titles.   |
| `--text-dim`       | `#b8b0cc` | Labels, secondary info, metadata (>= 4.5:1).     |
| `--text-faint`     | `#7e7498` | Section eyebrows, placeholders, disabled text.   |
| `--text-on-accent` | `#120d1d` | Ink on `#e8ff3a` buttons/badges/pills (~14:1).   |

### Semantic (10% Acid Cyber Lime & Status)

| Token           | Value     | Use                                                    |
|-----------------|-----------|--------------------------------------------------------|
| `--accent`      | `#e8ff3a` | Primary CTA, active rail bar, speaking, unread badge.  |
| `--accent-hi`   | `#f1ff7a` | Hover state for accent elements.                       |
| `--accent-lo`   | `#c8df18` | Pressed state for accent elements.                     |
| `--idle`        | `#10b981` | Connected / active online dot & chip.                  |
| `--warn`        | `#f59e0b` | Muted mic / warning.                                   |
| `--danger`      | `#ef4444` | Destructive border/wash (`#fca5a5` for readable text). |

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
| `--r-sm`   | 12px  | Buttons, inputs, select boxes, nav rail items, OTP.    |
| `--r-md`   | 16px  | Inner cards, participant tiles, settings cards.        |
| `--r-lg`   | 22px  | Floating glass islands, lobby cards, chat, music cards.|

Full pill radii (`rounded-full`) for status chips, badges, unread counters, and avatars.

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