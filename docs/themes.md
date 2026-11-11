# Themes

Fast Harness ships eight colour themes and reads your own from JSON files.
The TUI and the GUI use the same themes.

| Id | Name | Kind | Look |
| --- | --- | --- | --- |
| `dark` | Dark | dark | Ink blue-black, periwinkle and lavender accents (the default) |
| `light` | Light | light | Cool white, indigo and violet accents |
| `aurora` | Aurora | dark | Night blue with teal, violet and green glows |
| `synthwave` | Synthwave | dark | Deep purple, hot pink and cyan |
| `sunset` | Sunset | dark | Plum fading to orange, coral and gold |
| `emerald` | Emerald | dark | Deep green, mint and lime |
| `sakura` | Sakura | light | Soft pink, magenta and plum |
| `citrus` | Citrus | light | Warm cream, orange and lime |

Pick a theme in the GUI under Settings, or press F5 in the TUI to step
through every theme (built-in first, then yours). The pick is saved as
`theme` in `~/.fast-harness/config.json`; `FAST_HARNESS_THEME` overrides it.

## Greyscale

Greyscale is one setting for every theme, from 0 (full colour) to 100 (pure
greys), saved as `greyscale` in `config.json` (`FAST_HARNESS_GREYSCALE`
overrides it). In the TUI, F6 raises it by 10 and Shift+F6 lowers it by 10
(both wrap around); the status bar says the new level in words. Each colour
moves, in linear light, toward the grey of the same brightness, so the
contrast between text and background stays the same at every level.

The old `greyscale-dark` and `greyscale-light` themes are gone: a config that
still names one loads as `dark` or `light` with greyscale 100.

## Your own themes

Put one JSON file per theme in `~/.fast-harness/themes/` (or
`$FAST_HARNESS_HOME/themes/`). Files are read at start and when you press
Reload in the GUI settings; they are listed after the built-in themes, in
file name order. A broken file is reported and skipped; the others still load.

```json
{
  "version": 1,
  "id": "midnight-rose",
  "name": "Midnight rose",
  "kind": "dark",
  "extends": "synthwave",
  "colors": {
    "accent": "#ff9ad5",
    "glow": "#ff5fb0"
  }
}
```

| Key | Meaning |
| --- | --- |
| `version` | Always `1`. |
| `id` | Lower-case letters, digits, `-` and `_`. Saved in `config.json`. A file with a built-in id, or with the id of an earlier file, is skipped with a note. |
| `name` | Shown in the pickers. |
| `kind` | `"dark"` or `"light"`: which variant of native widgets and scroll bars goes with it. |
| `extends` | Optional. The id of another theme (built-in or yours) to start from; only the roles you list change. Chains are fine; a cycle or a missing base is an error. |
| `colors` | Role name to `"#rrggbb"`. Without `extends`, every role must be set. An unknown role or a bad colour is an error that names the file and the key. |

## Roles

| Role | Used for | Contrast the built-ins meet |
| --- | --- | --- |
| `bg` | Window, transcript and page background; start of the background gradient | — |
| `bg_end` | End of the background gradient (`bg` top left to `bg_end` bottom right) | — |
| `surface` | Sidebar, header and status bars, cards, the composer | — |
| `raised` | Inputs, dropdowns and buttons at rest | — |
| `hover` | A hovered row or control | — |
| `selection` | A selected row or list item | — |
| `bubble` | Your message bubble | — |
| `code` | Code spans and code blocks | — |
| `accent_surface` | A surface tinted with the accent: active navigation item, highlighted card, callout | — |
| `line` | Decorative separators | none |
| `border` | Borders of inputs, dropdowns and buttons | 3:1 on `bg`, `bg_end`, `surface`, `raised` |
| `text` | Body text | 7:1 and APCA Lc 75 on every background role above |
| `dim` | Secondary text: captions, timings, thinking, hints | 4.5:1 and Lc 60 on `bg`, `bg_end`, `surface`, `raised`, `hover`, `bubble`, `code`, `accent_surface` |
| `accent` | Links, the focus ring, active markers, the primary button, start of accent gradients | 4.5:1 and Lc 60 on `bg`, `bg_end`, `surface`, `raised`; 3:1 on `hover`, `selection`, `accent_surface` |
| `accent2` | Second accent: end of accent gradients, secondary highlights | 4.5:1 and Lc 60 on `bg`, `bg_end`, `surface`, `raised` |
| `on_accent` | Text and icons on `accent`, `accent2` and a gradient between them | 4.5:1 and Lc 60 on `accent` and `accent2` |
| `glow` | Coloured glow around the focused or primary element, decorative gradients (drawn see-through) | none |
| `shadow` | Drop shadows of cards, popups and menus (drawn see-through) | none |
| `error` | Error text and marks | 4.5:1 and Lc 60 on `bg`, `bg_end`, `surface` |
| `warning` | Warning text and marks | same as `error` |
| `success` | Success text and marks | same as `error` |
| `info` | Information text and marks | same as `error` |

The ratios are WCAG 2.2 contrast ratios (7:1 body text, 4.5:1 other text, 3:1
borders and focus); Lc is the APCA lightness contrast (Bronze level). Status
colours always come with a word or a mark (`✓`, `✗`, `↻`), so no meaning rests
on colour alone.

## The contrast check

Every built-in theme passes the check above at greyscale 0, 25, 50, 75 and
100 (a test holds them to it). Your theme loads even when it fails: the TUI
status bar and the GUI show a warning that lists each failing pair with its
ratio, for example `dim on bg: 3.10:1 (needs 4.5:1), APCA Lc 41.2 (needs 60)`.
