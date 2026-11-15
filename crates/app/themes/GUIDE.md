# Making a Gentle Shell app theme

An app theme changes the desktop app's colors and can put an animated scene
behind the panes (the panes are glass, so it shows through them). Themes live
in this folder; the app notices new or edited files within two seconds, no
restart. The person picks one in Configuration › Theme › Animated backgrounds.

Never put these files in Pi's own `themes` folder: Pi's theme schema rejects
the `background` key.

## Files

- `<name>.json`: the theme.
- `<name>.js`: its scene, if it has its own (optional).
- An image beside it, if it uses one (optional: png, jpg, gif, webp).
- `scene-error.txt` appears here when the active theme's scene fails, with
  the error and the line; it goes away once the scene draws. Read it after
  writing a scene and fix what it says.

## The theme file

```json
{
  "name": "Rainy Cafe",
  "bg": "#2a211c",
  "text": "#f2e6d8",
  "accent": "#e0a458",
  "background": {
    "scene": "rainy-cafe.js",
    "sky": ["#1c1512", "#2a211c", "#3a2a20"],
    "speed": 1,
    "veil": 0.5
  }
}
```

- `name`: shown in the picker; unique (a theme with the same name as a
  shipped example replaces it).
- `bg`, `text`, `accent`: `#rrggbb`. Every other UI color is derived from
  them. Keep `text` readable on `bg` (contrast 7:1 or more) and the accent
  3:1 or more. Pi's full format (`vars`, `colors` with `muted`, `dim`,
  `border`, `success`, `error`, `warning`, `toolTitle`, `userMessageBg`,
  `selectedBg`, …, and `export.pageBg`) also works when you want finer control.
- `background` (all optional):
  - `scene`: a built-in scene's name (see below) or a `.js` file beside the
    theme (plain file name, no folders).
  - `image`: an image file beside the theme, drawn to cover the window
    (animated GIFs play). `dim` (0–0.9) darkens it toward `bg`.
  - `sky`: colors of a vertical gradient under everything, top to bottom.
  - `speed`: seconds of scene time per second (0 holds it still; up to 16).
  - `cell`: the ASCII cell size in pixels for cell scenes (default 22).
  - `veil`: how much the middle of the window, where the conversation is,
    is calmed toward `bg` (0–1, default 0.5). The sides stay lively.

Built-in scenes: BUILTIN_SCENES.

## Scenes

A scene is JavaScript, run sandboxed: no files, network, timers, DOM or
modules, only computing and drawing. A frame may take at most 250 ms and
memory is capped. The app keeps a scene's script under 8% of one core: a
frame that takes 3 ms or less runs at 24 fps; a heavier one is shown less
often (down to 8 fps) and looks choppier, so keep frames cheap. The file
registers itself:

```js
SCENES['rainy-cafe'] = {
  draw(g, t) { /* pixels */ },
  cell(x, y, cols, rows, t) { /* ASCII cells */ },
  sprites(cols, rows, t) { /* ASCII sprites */ },
  size: 22, // optional: the largest cell size the scene allows
};
```

Use `draw`, the ASCII pair, or both (`draw` paints first). `t` is seconds.

### Pixels: `draw(g, t)`

`g.width`, `g.height` (logical pixels), `g.theme` (`bg`, `text`, `muted`,
`accent`, `info`, `success`, `warning`, `danger`, `light`). Colors are
`'#rgb'`, `'#rrggbb'`, `'#rrggbbaa'` or `'rgba(r, g, b, a)'`; `alpha` is
0–1 and multiplies the color's own.

- `g.rect(x, y, w, h, color, alpha = 1, radius = 0)`
- `g.circle(x, y, r, color, alpha = 1)`
- `g.ellipse(x, y, rx, ry, color, alpha = 1)`
- `g.line(x1, y1, x2, y2, color, alpha = 1, width = 1)`
- `g.text(x, y, text, color, alpha = 1, size = 13, bold = false)`: centered
  on (x, y), monospace; emoji work too.
- `g.glow(x, y, r, color, alpha = 1)`: a soft radial light.
- `g.gradient(x, y, w, h, angle, from, to, alpha = 1)`: linear; 180 is top
  to bottom, 90 left to right.

### ASCII: `cell` and `sprites`

The window is a grid of `cols` × `rows` cells. This is the format of
claude-desktop-themes' live wallpapers: their scene files work unchanged.

- `cell(x, y, cols, rows, t)` returns, for one cell, a tile
  `{ bg, fg, a }` (a rounded square with a glyph), a loose character
  `{ fg, a, ch? }`, or nothing. Without `ch` the glyph is picked and
  shimmers on its own.
- `sprites(cols, rows, t)` returns pictures placed in cell units:
  `{ x, y, key, art, pal }` where `art` is rows of characters and `pal` maps
  each character to a cell (`.` is empty), or `{ x, y, key, w, h, field }`
  where `field(gx, gy)` returns `{ c: cell }` or nothing. Optional: `a`
  (opacity), `shadow` (a color), `glow: { r, color, a }`, and for `art`
  sprites `halo` (a color).
- Helpers: `hash(a, b, c?)` (stable random 0–1), `glyph(set, x, y, t)`,
  `ell(x, y, cx, cy, rx, ry)` (≤ 1 inside an ellipse),
  `drift(cols, f, t, v, w)`, `near`, `twinkle(x, y, t, fg, odds)`,
  `rising(x, y, t, v, odds)`, `sketch(w, h, [[row, col, 'text'], …])`,
  `shape(w, h, (x, y) => ch)`, `flip(art)`, `frame(frames, t, rate, phase?)`,
  `shade(color, k)`, `tint(bg, fg, alphas)`, `rgb`, `hex`, `mix`,
  `GLYPHS`, `LOOSE`.

## What looks good

- One clear idea with two or three kinds of things, each easy to recognize.
  A busy, tiled background is too much.
- Things move their bodies, not only slide: tails beat, petals tumble,
  flames flicker, lights twinkle.
- Keep the middle calm: put the big things at the sides and corners; the
  veil softens the middle.
- Size things by the window (`g.width`, `cols`) so a small window still works.
- Cheap frames: a few hundred to a couple of thousand shapes, no per-pixel
  loops, nothing recomputed that could be computed once at the top of the
  file (shapes, palettes, paths) and reused.

## Sharing

The share button on a theme's card saves it as one `<name>.gentletheme` file
with its scene and image inside. Anyone imports it in Configuration › Theme
(Import…, a link, or dropping the file).
