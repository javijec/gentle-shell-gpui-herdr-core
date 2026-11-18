# Design system

How the app looks and which pieces it is built from. Everything lives in `crates/app/src/shared/`: `ui.rs` (tokens, components and motion), `themes.rs` (catalog and active theme), `banner.rs` and `surge.rs` (brand and pixel art effects) and `assets.rs` (icons). Views use these pieces instead of loose colors and sizes, so every screen looks like the others.

## Principles

These principles come from the current code. They are not aspirations.

| Principle | How it applies |
| --- | --- |
| **Translucent glass** | The window requests `WindowBackgroundAppearance::Blurred` and a transparent title bar (`crates/app/src/app/mod.rs`). Surfaces (`panel`, `surface`, `well`…) are the theme background, tinted and with alpha, so the blurred desktop shows through. What floats on top (menus, screens, palette) is opaque: `popover_bg` and `screen_bg` have alpha 1 at any transparency. |
| **No lines in the chrome** | The title bar has no border (`title_bar_border = clear` in `ui::apply_theme`). Panels and the sidebar have a transparent `border_1()`; only the focused panel of a split shows a `petal().opacity(0.45)` border (`features/workspace/view.rs`). Dividers between panels are a 6 px gap (`DIVIDER`) with a 2×40 px handle that appears in `petal()` only on hover. Inside, zones are separated by space and by the change of surface (`panel` → `surface` → `well`). |
| **Thin lines only in screens** | Inside an overlay screen there is a `ui::border()` line between header, body and footer (`screen_header` uses `border_b_1`, footers use `border_t_1`), and Settings rows use `border().opacity(0.5)`. No more are added. |
| **Petal as accent** | The brand pink (the theme's `accent`) is reserved for what is live and primary: the dot that pulses while the agent works, the `✿ phase…` text, the focused panel, primary buttons and the welcome glow. It is not used as a background fill. |
| **Dense but calm** | `text_xs` is the most used size and `muted` the most used color (more than `fg`). Semantic color goes on icons, chips and card borders, not on large blocks. Animations last 220–420 ms and overshoot by no more than ~3 %. |

## Color

### Where colors come from

```text
themes/*.json (Pi theme format)
  → themes::parse_pi_theme   → Theme (opaque colors 0xRRGGBB)
  → ui::Palette::new(theme, clear)   → roles with alpha, based on transparency
  → ACTIVE (global RwLock in themes.rs)
  → ui::fg(), ui::surface()…  (read via themes::palette(|p| …), no cx)
  → ui::apply_theme(cx)       → gpui-component colors (buttons, inputs, lists, menus)
```

- **Catalog** (`themes::load_catalog`): first the Gentle Shell themes embedded with `include_str!` from `crates/app/themes/` (`Gentleman-Cute`, `Gentleman-Sexy`, `Gentle`, `Studio`, `Studio Light`, `Clay`, `Clay Light`); themes installed in `<gentle-pi>/themes/` replace them by name. Then the user's themes in `<agent dir>/themes/` (`Source::Pi`) and the app's own in `dirs::data_dir()/gentle-shell-gpui/themes/` (`Source::App`), then the example app themes in `crates/app/themes/examples/`, and finally the popular schemes in `SCHEMES`. Names are unique; the first one wins.
- **Initial theme** (`themes::pick_default`): the one saved in `dirs::data_dir()/gentle-shell-gpui/settings.json` (key `theme`), otherwise the `theme` from Pi's `settings.json`, otherwise `DEFAULT_THEME` (`"Gentleman-Cute"`).
- **Live changes**: `themes::select(index, cx)`, `themes::set_transparency(clear, persist, cx)` and `themes::set_interface_size(percent, cx)` reinstall the palette or the size, call `ui::apply_theme` and refresh all windows. Because tokens are read on every render, no view stores colors.
- **Hot reload** (`themes::watch_user_themes`): every 2 s a background task compares the file names, sizes and modification times of Pi's and the app's theme folders (files only: the app's `previews/` and `examples/` folders don't count); on a change it re-reads them and `replace_user_themes` swaps them in, keeping the active theme by name (re-applying it if its file changed).
- **Backdrops** (`Theme::background`, `shared/backdrop/`): only app themes may have a `background` key (`scene`, `sceneCode`, `image`, `sky`, `speed`, `cell`, `dim`, `veil`; see `backdrop/model.rs` and `crates/app/themes/GUIDE.md`), because Pi's theme schema has `additionalProperties: false` and would reject the file. `backdrop::layer` draws it first in the workspace root, so every pane draws over it and the glass shows it at the window's transparency: the sky as stacked two-stop gradients, the image as a covering `img`, then the scene. A scene runs on its own thread in QuickJS (`backdrop/scene.rs`) with the kit in `backdrop/kit.js`; each frame leaves numeric drawing commands, plus the x, y pairs its paths point into, which the painter turns into quads, glyphs (`paint_glyph`, shaped once per character) and paths (`PathBuilder`, filled with a color or a linear gradient, or stroked), all inside one `paint_layer` so thousands of overlapping shapes skip GPUI's bounds tree. The kit's transforms (`save`, `translate`, `rotate`, `scale`) are applied in JavaScript, so the painter only ever sees window coordinates: a turned rect or ellipse arrives as a path, and a curve is flattened as finely as the scale it's drawn at. Glows are stacked faint discs (GPUI has no radial gradient, and no blend modes). The `veil` calms the middle of the window toward the theme's background. Pacing: the scene thread keeps its script under 8% of one core (`CPU_SHARE`: 24 fps when a frame costs ≤ 3 ms, down to 8 fps), and the window redraws at the scene's own interval (`ui::request_frame_in`); only while the window is focused and motion isn't reduced, otherwise one still frame per size. The person's **Scene motion** (`backdrop::Motion`, `backdropMotion` in the app's settings file) caps it: Smooth, Calm (12 fps), Still. Scenes are not design tokens: they are the theme's own art, like pixel art, so their colors are free.
- **Pixel art in scenes** (`kit.js`: `pixels(rows, palette)` and `g.pixels(art, x, y, size, alpha, flip)`): sprites written as rows of characters, each row's runs of one color precomputed as one rect, drawn snapped to whole pixels so 8-bit scenes stay crisp and cheap (a few hundred rects per frame).
- **Image camera** (`background.camera`, `model::Camera`, `model::image_rect`): a slow Ken Burns move over the theme's image (zoom toward a focus point, sideways drift), always covering the window. When a theme has an image and a scene (or a camera, which then runs an empty scene for its clock), the image is no longer an `img` element: each scene frame carries where the image sits (`Frame::image`), the canvas paints it there with `paint_image` (a GIF's frame chosen by the wall clock), and the scene gets the same placement as `g.image` / `g.fromImage(u, v)`, so what it draws on the picture stays on it as the camera moves. An image alone, without scene or camera, stays a covering `img`.
- **Picture effects** (`background.effects`, `model::Effect`, `backdrop/effects.rs`): Wallpaper Engine's Water Flow, Foliage Sway and Shine on the picture's own pixels, in polygon regions an agent reads off the coordinate grid (no painted masks). GPUI has no custom shaders, so they run on the CPU in the scene thread: each region is prepared once per display scale (its soft-edged mask and per-pixel phase, at most `PATCH_BUDGET` pixels and never finer than the window shows the picture), and each frame renders it into a patch (`Frame::patches`) that `paint_patches` uploads as a `RenderImage`, drawn over the picture where the camera puts it; the previous frame's patches are dropped from the atlas (`drop_image`). `flow` slides two copies half a cycle apart and crossfades them (a flow map, seamless); `sway` displaces each spot on its own phase; `light` takes the region's most light-like pixels (bright, brighter than around them, warm unless `"key": "bright"`) as its lights, blurs them into a halo screened over the picture once at prepare time, and only flickers that lit copy's alpha per frame. Their cost counts in the scene's pacing like a heavy scene: Rivendell's six regions take about 3 ms per frame at half scale in release.
- **Power** (`platform::power`, `backdrop::motion_now`): the app reads the power state every 20 s (macOS: Low Power Mode via `NSProcessInfo` and battery via IOKit's `IOPSGetTimeRemainingEstimate`; Windows: `GetSystemPowerStatus`, battery saver included; Linux: the `low-power` ACPI platform profile, and a discharging battery in `/sys/class/power_supply`). On battery Smooth slows to Calm, and saving power holds every scene still, whatever the person chose; Configuration › Theme says so under Scene motion. Effects also share `TOTAL_BUDGET` pixels per frame, so many regions cost about what four do. Measured in release with the default window: a camera alone about 3% of a core at Calm, six effects about 4%, a busy scene 4–6%; a still theme 0%.
- **Scene previews** (`backdrop/preview.rs`): so the agent that writes a theme can see it, the themes watcher (`themes::watch_user_themes`) draws every app theme with a scene or a picture, active or not, after it changes (or, at launch, when its files are newer than its preview): four moments in a 1280 × 800 window, as SVG rasterized by resvg, into `themes/previews/<stem>.png`, and what a frame costs into `<stem>.txt`; for a picture, `<stem>.grid.png` (a labeled 0.1 grid with its effect regions outlined) and `<stem>.effects.png` (each effect at four moments a quarter cycle apart). The SVG mirrors `paint_frame` command by command (an ellipse is the same rounded box, a glow the same stacked falloff), and the sky, image, effect patches and veil are drawn as the window draws them.

JSON fields used by `parse_pi_theme` (the rest of Pi's format is ignored, except for the terminal's ANSI table):

| `Theme` | JSON | If missing |
| --- | --- | --- |
| `bg` | `export.pageBg`, otherwise `vars.bg` | `0xffffff` light / `0x0e0e11` dark |
| `fg`, `muted`, `faint` | `colors.text`, `colors.muted`, `colors.dim` | mixes of `fg` and `bg` |
| `accent`, `accent_active` | `colors.accent`, `colors.borderAccent` | `fg`; mix of `accent` and `fg` |
| `raised` | `colors.userMessageBg` | `mix(bg, accent, 0.14)` |
| `border` | `colors.border` | `mix(bg, fg, 0.25)` |
| `heading` | `colors.toolTitle`, otherwise `colors.mdHeading` | `warning` |
| `info` | `vars.blue`, otherwise `colors.syntaxFunction` | ANSI blue |
| `success`, `danger`, `warning` | `colors.success`, `colors.error`, `colors.warning` | fixed values |
| `selection` | `colors.selectedBg` | `mix(bg, accent, 0.25)` |

`light` is decided by the luminance of `bg` (> 0.5), or that of `text` if there is no background.

### Tokens

All are argument-less functions in `crate::shared::ui` that return `Hsla`.

| Token | Comes from | Used for |
| --- | --- | --- |
| `fg()` | `fg` | Main text, titles, values. |
| `muted()` | `muted` | Secondary text, subtitles, header icons, card bodies, loading states. |
| `faint()` | `faint` | Metadata, hints, icons inside fields, `section_label`, "N more" counters. |
| `accent()` | `accent` | Interactive things: links, focus, selection, the current session, running jobs. |
| `petal()` | `accent` (the same color) | The brand: live dot, `✿`, focused panel, glows, effects. |
| `petal_active()` | `accent_active` | Stronger variant of `petal`. |
| `champagne()` | `heading` | Tool card titles, Settings section icons, highlighted code. |
| `info()` | `info` | Models, tool chips, "running" state. |
| `success()` / `danger()` / `warning()` | same names | States; also on card borders with `.opacity(0.35)`. |
| `panel()` | background tinted 4.5 % | Panels, sidebar. |
| `surface()` | background tinted 9 % | Cards (`card`), keys (`kbd`), secondary buttons. |
| `surface_hover()` | background tinted 15 % | Hover on rows and collapsible headers; active row in menus. |
| `raised()` | `raised` | User messages, selected row, active option of `segmented`. |
| `well()` | barely tinted background | Sunken: code, outputs, text fields, `segmented` frame. |
| `border()` | background tinted 20 %, alpha 0.69 (dark) / 0.8 (light) | Card borders and thin lines in screens. |
| `border_strong()` | `border`, opaque | Fields, keys, frame of screens and islands, vertical separators. |
| `screen_bg()` | background tinted 5.5 %, opaque | Full screens and palette. |
| `popover_bg()` | background tinted 6.5 %, opaque | Islands, menus, floating cards. |
| `backdrop()` | black, alpha 0.44 (dark) / 0.25 (light) | Veil behind screens (`scrim`) and shadow color of `island`. |

"Tinted" is `mix(bg, mix(fg, accent, 0.6), amount)`: the background leaned toward the text with a touch of the accent. `Palette` also has fields without a public function (`window_bg`, `accent_deep`, `on_accent`, `danger_hover`, `danger_active`, `selection`, `scrollbar_hover`) that only `apply_theme` uses.

### Transparency

`Palette::new(theme, clear)` receives the window transparency (`clear`, 0 = most solid glass, 1 = clearest; default `DEFAULT_TRANSPARENCY = 0.6`, or `0.15` on Linux, where the compositor usually doesn't blur behind the window and a sharp desktop would show through; adjustable with the Settings slider). Each glass surface lowers its alpha from a solid value down to a floor that keeps text readable (`fade(solid, floor)`); for example `panel` goes from 0.93 to 0.16 and `surface` from 0.94 to 0.4. `popover_bg` and `screen_bg` do not change.

**OLED themes** (`Theme::oled`, today only *OLED Black*) ignore transparency: `window_bg`, `panel`, `well` and `screen` are opaque `#000000` so an OLED panel turns those pixels off, and cards, menus and user messages are opaque layers just above black. Don't add tints or glows that would light large areas in that theme.

**Solid themes** (`Theme::solid`, set with `"solid": true` in the theme file — today *Studio*, *Studio Light*, *Clay* and *Clay Light*) ignore transparency the same way, but in the theme's own colors: every glass role (`window_bg`, `panel`, `well`, `screen`, cards, user messages, borders) renders opaque at any transparency, so the whole palette is the same whatever the slider says. Text, accents and the floating layers (`popover_bg`, `screen_bg`) are unchanged.

### gpui-component

`ui::apply_theme(cx)` switches `ThemeMode` to light or dark according to `Palette::light` and rewrites the gpui-component colors, so the stock components already match:

| gpui-component | Token |
| --- | --- |
| `primary`, `button_primary`, `ring`, `link`, `progress_bar`, `slider_bar` | `accent` (hover: `accent_active`; text: `on_accent`) |
| `button`, `secondary`, `muted` | `surface` (hover `surface_hover`, active `raised`) |
| `danger`, `button_danger` | `danger` |
| `list_hover` / `list_active` / `tab_active` | `surface_hover` / `raised` / `raised` |
| `popover` | `popover_bg` |
| `input`, `window_border`, `scrollbar_thumb` | `border_strong` |
| `title_bar`, `title_bar_border`, `tab_bar`, `tab`, `list`, `scrollbar` | transparent |
| `radius` / `radius_lg` | 8 px / 12 px at 100 %, times the interface size |
| `font_size` / `mono_font_size` | 16 px / 13 px at 100 %, times the interface size (see [Interface size](#interface-size)) |

### Light and dark

Views do not branch by mode: they use tokens. The differences live in `Palette::new` (alpha of `window_bg`, untinted `well` in light, alpha of `border`, white `on_accent` in light, softer `backdrop`) and in `apply_theme` (gpui-component's `overlay`). If something really needs it, read it with `themes::palette(|p| p.light)`.

## Typography

- **UI font**: gpui-component's (`cx.theme().font_family`); the app does not change it. `ui::island` sets it explicitly so a menu opened from a monospaced bar does not inherit that font.
- **Monospaced**: `ui::MONO` is `"Menlo"` on macOS, `"Consolas"` on Windows and `"DejaVu Sans Mono"` elsewhere. It is used for paths, settings values, code, counters, clocks and tokens. `well_box` and `kbd` already apply it.
- **Scale**: the rem is gpui-component's `font_size` (16 px at 100 %), so `text_xs` = 12 px, `text_sm` = 14 px and `text_base` = 16 px. Every other size is written in rems too: `.text_size(rems(13. / 16.))` is 13 px at 100 %. The px values below are at 100 %.

| Size | Weight | Where |
| --- | --- | --- |
| 10 px | `SEMIBOLD`, uppercase | `section_label` |
| 10.5 px | normal | `chip`, `kbd` |
| 11–12 px | normal | Hints, status chip rows, tool subjects in `MONO` |
| 13 px | `SEMIBOLD` | Card title (in `champagne`); card body in 13 normal |
| 14 px | normal / `SEMIBOLD` | Message and composer text; titles of error cards and dialogs |
| `text_xs` | normal | Screen subtitles, rows, metadata (the most used) |
| `text_sm` | normal / `MEDIUM` | Row titles, empty and loading states; title of a chip's card |
| `text_base` | `SEMIBOLD` | Screen title in `screen_header` |
| `text_lg` / `text_xl` | `SEMIBOLD` | Agent name, onboarding hero |
| 17 px | `SEMIBOLD`, `petal` | Effort control title |
| 34 px / line height 42 | `BOLD` | Welcome heading |
| 72 px and up | `EXTRA_BOLD`, `petal` | The word at the end of the welcome |

`SEMIBOLD` is the weight for titles; `MEDIUM` for small titles inside floating cards; `BOLD` and `EXTRA_BOLD` only in the welcome.

### Interface size

Configuration › Theme has an **Interface size** row (80–150 %, default 100 %, 10 % steps; `−`, `+`, Reset), and ⌘= (also ⌘+), ⌘- and ⌘0 (Ctrl elsewhere, and the app menu's Zoom In, Zoom Out, Actual Size) do the same in any window. Key bindings run before a terminal's key handler, so they never reach the shell. The size is a whole percent (`Themes::interface_size`; the steps and limits are `gentle_core::interface_size`), saved as the factor `interfaceScale` in the app's `settings.json` next to `transparency`; a hand-set `1.25` stays 125 % (useful under WSLg, where the desktop is scaled but GPUI sees scale 1) and the next step lands on the 10 % grid.

`ui::apply_theme` sets gpui-component's `font_size` to 16 px × size (and `mono_font_size`, `radius`, `radius_lg` in proportion); gpui-component's root makes that every window's rem size on each frame. So everything in rems follows: Tailwind-style spacing (`p_2`, `gap_3`, `rounded_lg`), `text_*`, the stock components, and the app's own sizes, which are written as `rems(px_at_100 / 16.)` (dividing by 16 is exact, so 100 % is pixel-identical). One convention, no scaling helper:

- **Text, and lengths that hold or sit next to text** (row and bar heights, label and table columns, screen, popover and panel widths, icon boxes, key caps, island radius and padding, the title bar, the sidebar) are `rems(N / 16.)`.
- **Math against real pixels** (bounds, viewport, a measured pane width for a breakpoint like `NARROW`) converts with `rems(N / 16.).to_pixels(window.rem_size())`.
- **Stay px**: hairlines and 1–2 px gaps, shadows, entrance offsets in animations, painted art and effects (banner, surge, backdrops, the welcome finale's stage and kisses, the water reminder's drops and glass, the music equalizer, the commit graph's lanes), user-dragged geometry (floating cards, splits, minimum pane sizes, the devices dock) and the window's own size. On macOS the traffic lights keep their place in a taller title bar.
- **Measured things follow the rem**: the terminal measures its cell again at the new font size (13 px at 100 %) and resizes the PTY; the conversation list and the Markdown preview re-measure their rows (a GPUI `list` only does that by itself when its width changes).

## Spacing and radii

Values are at 100 % interface size; they are rems, so they grow with it (see [Interface size](#interface-size)).

| Radius | Size | Used in |
| --- | --- | --- |
| `rounded_md` | 6 px | `chip`, `kbd`, `segmented` options |
| `rounded_lg` | 8 px | `well_box`, `field`, `segmented` frame, list rows, icon boxes |
| `rounded_xl` | 12 px | `card`, panels, sidebar |
| `rounded_2xl` | 16 px | `screen` panel, palette |
| `rounded(px(18.))` | 18 px | `island` |
| `ISLAND_ROW` | 12 px | Rows inside an `island` with `p(px(6.))`: their corners follow the island's |
| `rounded_full` / `rounded(px(999.))` | pill | Status dots, send/stop buttons, welcome primary button |

Recurring sizes:

| Where | Size |
| --- | --- |
| Window body | `px_1p5`, `pb_1p5`, `gap_1p5` between sidebar and panels; 6 px divider |
| `screen` | 32 px margin around the panel; fixed width or, without width, `size_full` with `max_w` 1280 |
| `screen_header` | `px_5`, `py_3`, `gap_3` |
| Screen body | `p_4`, or `px_4 pt_4` with sections in `gap_3` |
| Screen footer | `px_4`, `py_3`, `gap_2` |
| Card header | `px_3`, `py_2`, `gap_2`; body `px_3 pb_2` |
| Settings section | `card().p_4()` (`features/settings/layout.rs`) |
| Fields | `field` is 34 px tall; 30 px in compact bars |
| Chip card | 360 px wide, `p_3`, list with `max_h` 380 |

Current screen widths: Providers 640, Onboarding 720, Sessions and Usage 760, Settings 820; Profiles, Memory and Agents use `screen(None)` (Profiles caps it at 1100 × 760).

## Surfaces and components

Surface order, back to front: window (`window_bg`) → `panel` → `card` (`surface`) → `well_box`/`field` (`well`, sunken). `raised` marks what is selected. `island` and `screen` float, opaque.

| Piece | What it is | When |
| --- | --- | --- |
| `card()` | `rounded_xl` column, `surface`, `border` border | Tool cards, Settings sections, usage cards. |
| `well_box()` | `rounded_lg` column, `well`, `MONO`, `text_xs` | Code, command outputs, diffs, stderr. |
| `chip(text, color)` | 10.5 px pill: `color` text, `color.opacity(0.12)` background, `color.opacity(0.22)` border | Model, state, counts, "current", "N unsaved". |
| `kbd(text)` | 20 px key in `MONO` | Shortcut hints that are not an action on their own: `↑↓`, `←→`. |
| `key_button(id, text)` | `kbd` with a pointer; hover lifts the text to `fg` and the border to `petal().opacity(0.6)` | A hint that stands for one action in a footer legend or dialog (`↵` run/submit, `esc` cancel); add `.on_click` doing what the key does. Screen headers don't show `esc`: they have the close X, and Esc still closes. |
| `section_label(text)` | 10 px `SEMIBOLD` in `faint`, uppercase | Heading of a group. |
| `field()` | 34 px row, `well`, `border_strong` border | Frame for `Input::new(..).appearance(false)`, with an optional icon on the left. |
| `segmented(id, labels, selected, on_select)` | Text options inside a `well` | Two- or three-option toggles ("This project / All projects"). |
| `island(cx)` | Opaque floating card, radius 18, `border_strong().opacity(0.8)` border, shadow with `backdrop()` | Composer menus, chip cards, task island, onboarding groups. |
| `screen(width)` + `screen_header()` | Veil + centered panel; header row | Every overlay screen (see Patterns). |

```rust
// Card section with a title and sunken content.
ui::card().p_4().gap_2()
    .child(ui::section_label("Task"))
    .child(ui::well_box().px_3().py_2().text_color(ui::fg()).child(task));

// Search field.
ui::field()
    .child(Icon::new(IconName::Search).small().text_color(ui::faint()))
    .child(Input::new(&self.filter).appearance(false).small());

// Floating menu with rows that follow the island's corners.
ui::island(cx).p(px(6.)).child(
    div().px_3().py_1p5().rounded(px(ui::ISLAND_ROW))
        .hover(|s| s.bg(ui::surface_hover()))
        .child(label),
);
```

A gpui-component menu that should look like the island is opened with `Popover::appearance(false)` and its content wrapped in `ui::island(cx)` (see `chip_popover` below).

Presentation utilities in the same module: `split_path` (prefix `~/a/b/` and leaf, to show the prefix dimmed), `tail` (last N lines and how many were hidden) and `format_duration`.

## Motion

| Function | Duration | Curve | Effect | Typical use |
| --- | --- | --- | --- | --- |
| `appear(id, el)` | 220 ms | `ease_out_quint` | Opacity 0→1 and relative `top` from 6 px to 0 | First appearance: rows, Settings content, sidebar, usage cards. |
| `appear_after(id, delay, rise, el)` | `delay` + 420 ms | ease-out-quint computed by hand (the wait stays linear) | Opacity and relative `top` from `rise` px | Staggered entrances (welcome, onboarding steps). |
| `pop_in(id, el)` | 340 ms | `spring` | Opacity and relative `top` from 12 px | Screens, dialogs, palette. |
| `popover_in(id, el)` | 300 ms | `spring` | Opacity only, without changing the measured size | Content inside an `island` or a popover. |
| `scrim(id)` | 220 ms | `ease_out_quint` | `backdrop()` veil fading in | Behind a panel (`screen` already adds it). |
| `pulse(el)` | 1400 ms, repeating at ~30 fps | `pulsating_between(0.35, 1.0)` | Breathes the opacity | Live indicators: activity dot, running agents, thinking `✿`. |
| `spinner(size, color)` | 800 ms a turn, ~30 fps | linear | The `Loader` icon turning | Rows that stay on screen while work runs: running tool cards, the model picker loading, running agents. |
| `shimmer(text, base, highlight)` | 2000 ms sweep, ~30 fps | smoothstep band | A highlight crossing the text | "Working…", "Thinking…", "Starting gentle-shell…". |
| `spring(t)` | — | — | Spring with ~3 % overshoot | Inside the animator for anything that opens. |

Guidelines:

- **One animation per id.** GPUI plays an animation once per `ElementId`. To repeat it, change the id with a counter (`ElementId::NamedInteger("effort-knob".into(), moves)` in `features/session/view/effort.rs`, `NamedInteger("max-surge".into(), run)` in `surge.rs`); to keep it from replaying when something reopens, the id must stay stable.
- **Live indicators are slow on purpose.** `pulse`, `spinner` and `shimmer` read one app clock and redraw their view every 33 ms instead of at the display's rate: a frame re-renders the whole view and its ancestors (the conversation, during a turn), so a 120 Hz dot would cost four times the work for no visible gain. Use them, not `Spinner`/`ShimmerText` or a repeating `with_animation`, for anything that stays on screen while work runs. They hold still with reduced motion.
- **Stagger with `appear_after`**: same base and a constant gap per step (`self.entrance + STEP_GAP * (index + 1)` with `STEP_GAP = 110 ms` in `features/onboarding/sequence.rs`).
- **Do not replay on scroll**: the conversation only applies `appear` to a row during `APPEAR_WINDOW` (300 ms) after it was first seen (`features/session/view/transcript_list.rs`).
- **`spring` goes in the animator, not as easing**: GPUI's easing must stay within 0..=1 and the spring goes past 1 (`pop_in` applies it inside the closure).
- **Move without pushing**: `appear`, `pop_in` and `appear_after` move with `.relative().top(..)`, which does not move flex siblings. Don't animate margins or sizes for an entrance. Since `.relative()` replaces the position, don't pass them an `.absolute()` element; wrap it instead.
- **Paint-only effects**: large effects draw in a `canvas` with `window.paint_quad`; they take no input and do not change layout. The surge shake is applied as an offset of the window root with `request_animation_frame`, without wrapping it in an animation, because that would change the key of every inner element (`features/workspace/render.rs`). A `Task` with a timer of the same duration clears the state when it ends (`Workspace::play_surge`, `WelcomeView::enter`).

## Brand

`crates/app/src/shared/banner.rs` draws the Gentle Shell banner (the braille rose and the "Gentle Shell" pixel art logotype) as real pixels. The art comes from the brand assets `gentle_rose.txt` and `gentle_shell_combo.txt`: the rose keeps the per-cell shading of `gentle_rose_color.txt` as a lightness map (`ROSE_SHADE`, applied to whichever palette is active), and the layout follows the combo (a braille dot is half a logotype pixel, 5 columns between rose and logotype, both centered). The palettes and the `banner.json` format come from gentle-pi's `extensions/startup-banner.ts`; `Pink` uses the assets' ink (255, 118, 195) and shadow (120, 45, 80).

| API | What it does |
| --- | --- |
| `banner(config, scale, reveal)` | Rose + logotype at `scale` px per pixel (~170 px wide per scale unit). Drive `reveal` from 0 to 1 linearly over `banner::WRITE` (2600 ms): the rose blooms from its heart outward (petals unfold as they appear) while a pen writes the logotype in cursive along its slant, at a steady ink rate (slower over dense letters, quick through joins, a short lift between the words); each fresh stroke glows like wet ink and cools to the ink color, and the shadow settles in just behind. The rose floats the whole time (a 5.2 s rise and fall, a 7.3 s sway about the bloom, a glint crossing the petals every 6.5 s), redrawn at 20 fps only while the window is active, and still with reduced motion. |
| `BannerConfig { show_rose, show_text_logo, palette }` | Which parts to show and with which palette; `Default` shows everything in `Pink`. |
| `Palette::{Pink, Cyan, Yellow, Green}` | Rose, ink and shadow of the logotype in fixed RGB (they do not depend on the theme). |
| `read_config(config_home)` | Reads `<config>/banner.json` (`showRose`, `showTextLogo`, `color`); anything missing or invalid falls back to the default. |
| `pixel_rose(height, color)` | Only the rose, in a theme color (the onboarding hero uses it in `petal` or `success`). |
| `rose()` | Points and size of the rose, for other effects. |

The pixel art style shares rules: flat squares painted with `paint_quad`, slightly smaller than their cell so the grid shows (`dot * 0.55` in the rose, `dot * 0.9` in sprites); sprites defined as text (`#` body, `o` outline) like `LIPS` and `HEART` in `features/welcome/finale.rs`; colors derived from `petal()` when the effect belongs to the app.

- **Welcome finale** (`features/welcome/finale.rs`, `FINALE = 5600 ms`): the banner blooms and writes itself in again, the rose throws 14 pixelated kisses (one in four is a heart in `petal`), "Sexyyyy" enters letter by letter with `spring` and the screen dissolves from `DISSOLVE = 0.86` with smoothstep.
- **Water reminder** (`features/hydration/drops.rs`, `DURATION = 6000 ms`): pixel drops (`DROP` sprite, `paint_sprite`) rain over the window, each splashing with a flat ripple where it lands, while a floating card fills a pixel glass drop by drop and says it's time for some water; everything fades from `FADE`. `hydration::show` plays it and `hydration::overlay(run)` paints it over the window root. Its water is `info()` and a lighter glint of the same hue. With reduced motion only the card shows.
- **Effort surge** (`shared/surge.rs`, `DURATION = 4400 ms` at max): a pixel comet runs along the window edge, the window shakes, the core hatches and the rose comes out. Every thinking level plays its own share of that show, each adding a beat to the one below (`surge::Level::of`): minimal and low run the comet along a quarter and half of the edge; medium closes the lap and flares the edge; high gathers the core while the window trembles gently; xhigh trembles harder and hatches the core (flash, shockwave, sparks); max adds the rose. `off` plays nothing. `SessionEvent::Effort(level)` triggers it once the backend accepts the level and `Workspace::play_surge` plays it; `surge::overlay(run, level)` paints and `surge::shake_at(elapsed, level)` gives the offset. Its colors (`spark`, `wine`) are derived from the hue of `petal()`.

## Icons

Icons are `IconName` from `gpui_kit::assets` (the Lucide catalog) and are drawn with `Icon::new(IconName::X)`, almost always `.small()` or `.xsmall()`. The binary only includes gpui-kit's default bundle plus the names listed in `ExtraIcons` (`crates/app/src/shared/assets.rs`); `AppAssets` looks in `ExtraIcons` first and then in `gpui_kit::assets::Assets`, and it is the source registered with `.with_assets(assets::AppAssets)` in `app/mod.rs`.

To use a Lucide icon that does not show up:

1. Add its name to the `gpui_kit::assets::icon_assets!(ExtraIcons, [...])` list, with a comment if it belongs to a specific feature.
2. Use it with `Icon::new(IconName::Name)`. No SVG is copied: the macro takes the one shipped with `gpui-kit-assets`.

A custom SVG, outside Lucide, would need one more branch in `AppAssets::load`; none exists today.

Icon color: `muted` in screen headers and neutral cards, `faint` inside fields and in empty states, the tone color in states and extension cards, `champagne` in Settings sections.

## Patterns

### Overlay screen

Settings, Providers, Usage, Sessions, Profiles, Memory and Agents follow the same shape (`features/usage/view.rs`, `features/sessions/render.rs`). Onboarding uses `ui::screen` without `screen_header`: its hero acts as the header and the close X floats at the top right. The palette (`features/palette/view.rs`) repeats the frame by hand (`screen_bg`, `rounded_2xl`, `border_strong`, `shadow_2xl`) with a fixed height of 560 px so it does not jump while filtering.

```rust
let (backdrop, panel) = ui::screen(Some(760.));
backdrop.occlude().child(ui::pop_in(
    "usage-screen",
    panel.w_full().max_w(px(760.))
        .child(
            ui::screen_header()
                .child(Icon::new(IconName::Gauge).small().text_color(ui::muted()))
                .child(div().flex_shrink_0().text_base().font_weight(FontWeight::SEMIBOLD)
                    .text_color(ui::fg()).child("Usage"))
                .child(div().flex_1().min_w_0().truncate().text_xs()
                    .text_color(ui::muted()).child(subtitle))
                .child(ui::key_button("usage-esc", "esc")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(UsageEvent::Close))))
                .child(Button::new("usage-close").ghost().xsmall().icon(Icon::new(IconName::X))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(UsageEvent::Close)))),
        )
        .child(div().id("usage-body").flex_1().min_h_0().overflow_y_scroll().p_4().child(body))
        .child(footer),
))
```

- Header, in this order: `small` icon in `muted`, `text_base SEMIBOLD` title, subtitle that takes the remaining space and truncates, optional actions (spinner, count chip, `segmented`, `ghost().xsmall()` buttons with tooltip) and the close X (`ghost().xsmall()` with `IconName::X`). No `esc` badge in the header: Esc closes every screen anyway.
- The body is the only part that scrolls (`flex_1().min_h_0().overflow_y_scroll()` with an id).
- The view does not handle Esc: it emits its `Close` event and the `Workspace` removes it (`on_close_overlay`). The veil uses `.occlude()` so clicks do not pass through to what is behind; only the palette closes on a click on the veil.
- Screens open below the title bar (`Workspace::render` places them under its 36 px), so its buttons stay usable: a screen button opens its screen in place of the current one, and the open screen's button shows selected and closes it. Only the welcome and onboarding cover the whole window.

### Full screen: welcome

`features/welcome/` takes up the whole window (below the 36 px title bar on Windows and Linux; on macOS it leaves a drag area below the traffic lights).

- `screen_bg()` background with a gradient from the top: `petal().opacity(0.10)` while installing, `success().opacity(0.10)` when everything required is ready and `petal().opacity(0.18)` during the entrance.
- Banner at scale 2.6, blooming and writing itself over `banner::WRITE` (2600 ms, as long as `INTRO` + `AFTER_INTRO`); when ready it switches to `Palette::Green` with `pop_in`. Heading (34 px `BOLD`) and footer enter with `appear_after` at `AFTER_INTRO` (1100 ms). If the screen is shown again, `intro_done` draws everything in place without animating.
- Content centered with `max_w(px(680.))`; 88 px footer with `px_10`; ↵ runs the primary button.

### Session status chips

`features/session/view/activity.rs` draws the activity line above the composer. On the left, while the agent works: `pulse` of a 6 px `petal` dot, clock in `MONO` and `muted`, a 1×10 px separator in `border_strong`, `✿ phase…` in `petal` and the current tool, truncated, in `muted`. On the right, the chips.

Every new chip uses `chip_popover(id, icon, color, label, title, count, body)`: a `ghost().xsmall()` `Button` with `text_color(color)` that opens a `Popover` anchored `BottomRight`, 360 px wide, with `appearance(false)`, content in `ui::island(cx)` + `ui::popover_in`. The card shows icon and title (`text_sm MEDIUM`), the count in `MONO` and a scrollable list; `body` builds the rows only on open. Chip color: `accent` if something is running, `muted` at rest, `faint` for passive information (token usage). In panels narrower than `TINY`, extension states collapse into a single chip.

### Conversation cards

Tool cards (`render_card` in `features/session/view.rs`) and extension message cards (`features/session/view/cards.rs`) share a shape:

- `ui::card().w_full().overflow_hidden()`; if the tone is danger or warning, `border_color(color.opacity(0.35))`.
- Header `px_3 py_2 gap_2`: `small` icon (tone or `muted`), 13 px `SEMIBOLD` title in `champagne`, truncated, optional chip (`info` for tools, the tone for extensions), subject in `MONO` 12 px `muted`, `+n` in `success` and `−n` in `danger`, `faint` chevron if it collapses. A collapsible header has a `surface_hover` hover.
- Body `px_3 pb_2`: 13 px text in `muted`; outputs in `MONO` 12 px (`CustomBody::Tail`); Markdown with `TextView` in `fg`. Long content is cut and offers "N more lines" / "Show less".
- The tone (`Tone::{Success, Danger, Warning, Info, Muted}`) is decided by a pure function from the type and the `details` (`custom_look`), with tests.

### Generative UI diagram card

A `gentle_ui` diagram (`features/genui/view.rs`) is a conversation card in the style of React Flow, built from our tokens: the same header (`Workflow` icon in `info`, `champagne` title, `v<n>` chip, node and edge counts in `MONO`) with `ghost().xsmall()` icon tools on the right (add node, group the selection, direction `ArrowRight`/`ArrowDown`, undo, redo, expand); a `well` canvas; and a `border_t_1` footer like the tool cards' status footer: the hint line in `faint` ("View only" when locked), or the pending edits (`N changes` chip in `accent`, "Add a note…", `ghost` Discard, `primary` Send to agent), "Sent to the agent" in `success`, a refused edit in `danger`, or a rejected send (the extension's reason in `danger` with `CircleAlert`, the full text in a tooltip, and the primary button turned into Retry with `RefreshCw`). In the conversation it is `CARD_HEIGHT` (460 px at 100 %) tall, a fixed size so the surface is a cached view that only redraws when it changes; expanded, it fills the conversation's place over the composer.

- Canvas: a dotted grid (one `border_strong` dot per 20-unit crossing, thinned to every other line under 10 px, moving and scaling with the view), group boxes `surface().opacity(0.55)` with a `border()` hairline (`accent` when the group is selected), 14 px corners and the title in `muted` (`accent` selected) in the top padding; edges painted with `PathBuilder` as smooth Bézier chains routed around boxes (sync `muted` solid, async `warning` dashed, data `info` thicker and dashed, dependency `faint` dotted; hovered `accent` at 0.75 and 1 px wider, selected `accent`), ending in a filled arrowhead; labels in a small `surface` pill with a `border()` hairline (`accent` when highlighted). Everything painted or laid out in the canvas is diagram geometry: px × zoom × interface size, not rems.
- Nodes are elements (text, hover, clicks): a `surface` card with 10 px corners, a `border_strong` hairline and `shadow_sm`, a 26 px icon tile in the kind's tone at 0.14 (service and function `info`, database and storage `success`, queue `warning`, cache `champagne`, client `accent`, external `muted`, note `faint`), label 13 px `SEMIBOLD` `fg`, tech or kind 11 px `muted`, a status dot; `surface_hover` on hover, `shadow_lg` while dragged. Status: dashed when planned, `danger` border for risk, the whole node at 0.55 when deprecated. Selected: `accent` border and a ring 4 px outside (`accent` at 0.35) that fades in with `ui::popover_in`. Two 10 px handles (`surface` dot with an `accent` ring) sit on the target side and the source side (left/right left to right, top/bottom top to bottom) and show on hover and on the selection.
- Floating panels: the controls bottom-left (zoom in, zoom out, fit, lock: `ghost().xsmall()` icons stacked in a `popover_bg` box with a `border()` hairline and `shadow_sm`), the minimap bottom-right (168 × 108 px, same box; nodes as small boxes in their kind's tone at 0.7, the view as an `accent` outline over `accent` at 0.08), and the selection's details top-right in a `ui::island`: for a node, Kind / Status / Group as `ghost` dropdown buttons (`PopupMenu`, the current one checked, "New group…" last), Tech and Description `field`s, the id in `MONO`; for an edge, its style dropdown and label; for a group, its member count and description; for several nodes, their labels; with Rename / Group / Ask about this / Delete (Ungroup for a group). Rename edits in place with a `field` over the node, the label or the group title. The shift-drag selection box is `accent` at 0.08 with a dashed `accent` border.
- Motion: an agent's update eases in over 300 ms (ease-out cubic): kept nodes slide from where they were, new ones fade in growing from 92 %, removed ones fade out where they were, edges morph with them. Fit and the zoom buttons ease the view over 260 ms; wheel, pinch and drags follow the pointer at once. Async and data edges march (their dashes move at 24 units/s) at ~30 fps with Smooth motion and ~12 fps with Calm (`backdrop::motion_now`), only while one is on screen and the window is active; Still motion, reduced motion and an idle diagram ask for no frames at all (`motion::frame_delay`, tested). Frames come from `ui::request_frame_in`, so a card scrolled out of the conversation stops asking.
- The rows around it reuse the card header shape: drawing (spinner), failed (`danger` border, the error in `MONO`), superseded or closed (`faint`, at 0.7), shown expanded (with "Back here"), and the user's sent edits ("You edited …", the summary as bullets and the note).

### Generative UI panels

Tables, forms, checklists, kanban boards and charts (`features/genui/panel.rs` and one file per kind) use the diagram card's frame: the same header (the kind's icon in `info`: `Table`, `ClipboardList`, `ListChecks`, `SquareKanban`, `ChartColumn`; `champagne` title, `v<n>` chip, counts in `MONO`; `ghost().xsmall()` add / undo / expand tools) and footer (hint in `faint`, or `N changes` in `accent`, "Add a note…", the rejected send's reason in `danger`, Discard, `primary` Send to agent or Retry with `RefreshCw`; a form's footer is its Submit). Their height follows the content up to 480 px at 100 % (more scrolls inside, or opens expanded).

- Table: a `field` filter, a `well` header row (sort arrow in `accent`), 32 px rows with hairline `border()` separators, selected rows washed with `accent` at 0.10; typed columns have fixed widths (number and date in `MONO`, status as a `chip` toned by its words, links in `accent`).
- Form: label 12.5 px `MEDIUM` (`*` in `danger` when required), help in `muted`, controls in `field` frames; choices are pills (`accent` at 0.14 when on), radios use `Circle`/`CircleCheck`; problems in `danger` under each field. Submitted answers show read-only as label/value rows.
- Checklist: 32 px rows (grip and actions on hover), `CircleCheck` in `success` when done with the label struck through in `faint`; sub-items indented 26 px with a `n/m` count on their parent.
- Kanban: columns are `well` boxes (236 px), cards `surface` with a `border()` (`accent` when selected or held, `shadow_lg` while dragged), tags as `chip`s toned with `ui::provider_color`, the assignee as initials on `champagne`; a column over its limit gets a `warning` count and outline. Details and the card editor open in a `ui::island`.
- Chart: painted with `PathBuilder`/`paint_quad`, series in the theme's tones (`info`, `accent`, `success`, `warning`, `champagne`, `petal`, `danger`, `muted`), `border()` grid lines, `faint` `MONO` ticks; the hovered value in a `ui::island` tooltip, the selected one in an island with Ask about this.
- Run plan (checklist with `run`): the header's `ListChecks` tool turns it on (`accent` while on); a 36 px bar over the rows with a `done/total` `chip` (`accent` while running), the running or next step in `muted`, and `ghost` Run next (`SkipForward`) + `primary` Run (`Play`), or `ghost` Stop (`Square`) while a step runs; disabled while sending. Row status icons: `Circle` `faint` pending, `LoaderCircle` `accent` running (static, never spun), `CircleCheck` `success` done, `CircleX` `danger` failed, `Ban` `faint` skipped (label struck through like done); Skip is a hover action (`Ban`).
- Kanban subagents: a `starts` column shows `Bot` in `accent` beside its label; a card with an agent gets a 20 px row (`Bot`, the agent name in `muted`, then status · step · cost toned by status: `accent` working, `success` completed, `danger` failed/cancelled, `info` "dispatched", `faint` "not started"); its details add Stop subagent (`Square`) while it works.
- Live panels: the header shows a small pill "Live · updated 5 s ago" (`Activity`, `success` at 0.12; `faint` while paused or not allowed, `danger` with the error as its tooltip) and a `ghost` Pause/Play. A command source asks over the body's top in a `ui::card` with a `warning` outline: `Terminal` + "Allow this UI to run a command?", the command in `MONO`, its interval and folder in `muted`, then `ghost` Deny and Always for this session and `primary` Allow once; a denial leaves a one-line `Ban` strip with Ask again. Live checklists and the Todos island's checklist are read-only (no grip, check, add or remove; Ask stays). The label refreshes on the live tick (every 5 s, only while drawn in the active window), never an animation.
- Review (`features/genui/review.rs`, `GitCompare` icon; header counts files, `+`/`−` and "k of n hunks decided"; `CheckCheck` / `X` Accept all / Reject all tools): the body is the file panes' diff viewer (`gpui_kit` `Diff`, unified, line hover, syntax colors, its own file headers with a `MONO` `faint` "k/n" and `ghost().xsmall()` Accept all / Reject all), beside a 196 px `well` file list from two files on (status letter as a `chip`: A `success`, M `warning`, D `danger`, R `info`; name in `fg`, folder in `faint`; `+n` `success` / `−n` `danger` in `MONO`; `CircleCheck` in `success` once every hunk is decided; the current file on `surface_hover`). Under each hunk a decision bar (`surface`, `border()` hairlines; `accent` at 0.07 and the hunk's lines tinted the same when it is current): the state (`chip` Accepted `success`, Rejected `danger`, Applied `info`, or "Undecided" in `faint`), the agent's note in `muted`, `ghost().xsmall()` Accept / Reject / Comment (selected and toned when on; gone once applied), and the comment (`MessageSquareText` in `accent`) or its `field` with an `accent` border while typing. The footer's count is the progress and its primary is Send decisions. No motion of its own.
- Motion: only `ui::spring`/ease-out-quint glides and tweens, gated by reduced motion, backdrop motion Still and the window being active (`genui/motion.rs`, shared with the diagram).
- Shared tools (every kind, `genui/share.rs`): `ghost().xsmall()` Pin (`Pin`, `PinOff` in `accent` while pinned) and Export (`Share2`, a `PopupMenu` with Copy to clipboard ▸ / Save to file… ▸ submenus, Ask agent to add to docs with `BookOpen`, Save as template with `LayoutTemplate`), before Expand. Results are toasts (success, or warning on failure).
- Code refs (`genui/refs.rs`): an inline link, `FileCode` + `file.rs:42` in `MONO` 11 px `accent`, `rounded_md` with a `surface_hover` hover, at most 200 px (truncated); `FileX` in `faint` when the file isn't there or is outside the workspace; right-click for the editors. On a diagram node it is an 18 px icon button in the card's top-right corner, on an erd table one at the end of its header (both scaled with the canvas, hidden below 50 %).
- Pinned side panel (`genui/pinned.rs`): docked right of the conversation like the Agents panel (`border_l_1` hairline, 40 % wide, 300–560 px), a row of tabs (title truncated at 180 px; active on `raised` in `fg`, others `muted`; × in `faint`) over the live surface; the conversation's card becomes a row like the expanded one ("pinned to the side panel", Back here with `PinOff`).
- Templates manager (`genui/templates.rs`): a `ui::island` (440 px) over the top of the conversation: `LayoutTemplate` title row with a close ×, rows with the kind's icon, name in `fg` and `kind · title` in `faint`, Rename/Delete `ghost` icons and a `primary().xsmall()` Insert; an inline `field` while renaming; the empty state in `muted`.

### Generative UI canvas kinds

Sequence diagrams, data models (erd), state machines and timelines (`features/genui/canvas.rs` plus one file per kind) are panels whose body is the diagram's canvas, 384 px at 100 % (the card is the diagram's 460): the panel header (`ArrowLeftRight`, `Database`, `CircleDot`, `ChartGantt` in `info`; + adds a participant, table, state or task and opens its rename field) and footer, a `well` canvas with the dotted grid, the zoom in / out / fit controls bottom-left and the minimap bottom-right in the diagram's `popover_bg` boxes, the selection's details top-right in a `ui::island` (choice pills: `accent` at 0.14 with an `accent` hairline when on, `surface` with `border()` otherwise; `ghost().xsmall()` actions: Rename, Ask about this, Delete…) and a `field` over the canvas for renames. Shapes are painted (`canvas::Shape`: boxes, lines, curves with filled or open heads, polygons, the dashed `accent` rubber line while connecting); text, cards and handles are elements scaled with the zoom.

- Sequence: participant cards like the diagram's nodes (26 px kind tile: actor `accent`, service `info`, db `success`, queue `warning`, external `muted`), dashed `border_strong` lifelines, activation bars `info` at 0.18 with an `info` hairline, messages in `fg` at 0.75 (sync: filled head; async: open head; return: dashed `muted`, open head; self: a loop), labels on `well` pills, fragments as `surface` at 0.35 boxes with a `border_strong` hairline and a tab (`MONO` `accent` kind, `muted` condition), `else` dividers dashed. Selected: `accent`.
- Erd: table cards (`surface`, 10 px corners, `border_strong`; header washed `success` at 0.06 with the `Database` icon, the name `SEMIBOLD` and the column count `faint` `MONO`), 26 px column rows (`KeyRound` in `warning` for keys, `Link` in `info` for foreign keys, the type `faint` `MONO` with `?` when nullable), ● handles at both sides on hover, relations as `muted` curves ending in crow's feet (three prongs for many, a bar for one, then a bar when required or a `well` ring when optional, that is when the other end's column is nullable), the cardinality in a `MONO` pill.
- State: state cards with 16 px corners (the current one `accent`-bordered on `accent` at 0.08 with a dot and "current"), the initial state a filled `fg` dot, final states a ring with a dot, choices a `border_strong` diamond, composite states as the diagram's group boxes, transitions as the diagram's `muted` arrows with labels in pills; ● handle on the right on hover.
- Timeline: a task-name column (rows with hairline `border()` separators, group rows on `surface` at 0.6, milestones with `Flag` in `warning`), bars `info` at 0.2 with an `info` hairline (the critical path in `accent`, a task starting before its dependency ends with a `danger` hairline), progress filled at 0.4 with the percent in `MONO`, milestones as `warning` diamonds, weekend columns on `surface` at 0.3, dependency arrows (`accent` on the critical path, `danger` when late), a `surface` axis strip pinned to the top with `faint` `MONO` ticks (days, Mondays or months by zoom) and today as an `accent` line with a Today `chip`. Bar ends resize (`ResizeLeftRight`), ● on the right links.
- Motion: only the fit and zoom buttons ease the view (`motion::ViewTween`), never with reduced motion; drags follow the pointer.

### Generative UI HTML card

A `gentle_ui` html page (`features/genui/html.rs`) uses the diagram card's shell (`surface`, `rounded_xl`, `border()`): in the conversation a compact card (`CARD_HEIGHT` 104 px at 100 %) with the `CodeXml` icon in `info`, the `champagne` title, the `v<n>` chip, what the page may reach (`ShieldCheck` in `success` "isolated · no network", or `Globe` in `warning` with the origins) and a `primary().xsmall()` Open; below, one `faint` line (or the last send's result in `success`/`danger`). The whole card opens it on click and takes `surface_hover`. Expanded: the same title row with a `ghost` collapse button, the page in a `well` box with `rounded_lg` corners (its height is what the page reports, else the spec's, else the room there is), and a footer like the diagram's (hint in `faint`, a spinner while a message goes, "Sent to the agent" in `success`, errors in `danger` with `ghost` Discard and `primary` Retry when a message didn't reach the agent). A link the page asks to open replaces the page with a centered question (`ExternalLink` in `accent`, the URL in `MONO` on `raised`, `ghost` Cancel, `primary` "Open in the browser card"): the native view can't sit under GPUI content. The page gets the app's look as MCP Apps theme variables (`apps::style_variables`: backgrounds from `well`/`raised`/`panel`, text from `fg`/`muted`/`faint`, borders, `accent` as `--color-ring-primary`, the status tones, the interface font and `MONO`, sizes from the rem) plus a base style (text color, font, `color-scheme`) under its own.

### Buttons and footers

| Rule | Example |
| --- | --- |
| Secondary `ghost()` buttons on the left, a spacer (`div().flex_1()` or `ml_auto()`), the primary `primary()` last on the right. | Footer of Sessions, Usage, welcome, Profiles. |
| Sizes: `small()` in footers, `xsmall()` in headers and bars, `large()` only in the welcome. | |
| Icon-only buttons have a `tooltip`. | Refresh in headers. |
| Work in progress: `.loading(busy)` and the label says so ("Installing…"); `.disabled(..)` when the action is not possible. | `welcome-primary`, `usage-refresh`, `welcome-recheck`. |
| Destructive: `danger()` in confirmations (Discard, Delete), `danger().outline()` to stop agents. | `features/profiles/view/confirm.rs`, `features/session/agents.rs`. |
| Inline confirm: `outline()`; cancel: `ghost()` with "Cancel". | Rename profile, custom model. |
| Pill (`rounded(px(999.))`) only for the composer's send/stop and the welcome primary button. | `features/session/view/composer.rs` |

Typical screen footer: `h_flex().px_4().py_3().gap_2().border_t_1().border_color(ui::border())`, with hints (`kbd`/`key_button` + `text_xs muted` text) or a status line on the left.

### Empty, loading and error

| State | Shape |
| --- | --- |
| Loading | `h_flex().gap_2().text_sm().text_color(ui::muted())` + `Spinner::new().small()` (in `muted` or `accent`) + "Reading sessions…". Inline, `Spinner::new().xsmall()`. Brief loads only; a spinner that can stay up through a whole turn is `ui::spinner`. |
| Reloading with data | The list stays and the refresh button shows `.loading(..)`. |
| Empty | `v_flex().items_center().py_8().gap_2()`, icon in `faint` (`Inbox`, `Layers`) and a `text_sm muted` sentence that says what to do ("No profiles left. Create one with New."). |
| Error | `text_sm` text in `danger` with a `ghost` "Retry" button; or a `card` with `border_color(ui::danger().opacity(0.35))`, `CircleAlert` icon, `section_label` and the message; technical output, if any, goes in a `well_box`. |
| Action result | `text_xs` line with an `xsmall` icon: `muted` + `Info`, `success` + `CircleCheck`, `danger` + `CircleAlert` (Profiles footer). |

## Do and don't

| Do | Don't |
| --- | --- |
| Use `ui::*` for every color and the `ui.rs` pieces for cards, fields and chips. | Write `rgb(..)`/`hsla(..)` in a view, except pixel art with its own colors. |
| Separate zones with space and a change of surface. | Add borders or lines inside panels, cards or the chrome. |
| Reserve `petal` for what is live and primary. | Use `petal` or `accent` as the background of large blocks. |
| Secondary text in `muted`, metadata in `faint`, values in `MONO`. | Bump everything to `fg` or `text_sm`. |
| Open screens with `screen` + `pop_in` and menus with `island` + `popover_in`. | Use the stock popover frame or a custom veil. |
| Stable animation ids; a counter in the id only to repeat it. | Ids that change on every render (the animation restarts endlessly). |
| Large effects in a `canvas`, without input and with a `Task` that clears them. | Wrap the window root in an animation. |
| Test in a light theme (e.g. Catppuccin Latte) and with transparency at maximum. | Assume dark mode or an opaque background. |
| Make a key hint that stands for one action a `key_button` with the key's handler. | Leave an `esc`/`↵` cap that looks pressable but ignores the mouse. |

## Checklist for a new screen

1. The view lives in its feature (`features/<name>/`); anything generic used by more than one feature goes to `shared/ui.rs`, not to the view.
2. `ui::screen(Some(width))` (640–820) or `None`, wrapped in `ui::pop_in("<name>-screen", ..)` and with `.occlude()` on the veil.
3. Header with `screen_header`: `muted` icon, `text_base SEMIBOLD` title, truncated subtitle, `ghost().xsmall()` actions with tooltip, and the close X; the view emits `Close` and the `Workspace` handles it.
4. Scrollable body with an id; groups in `card()` with `section_label`; fields with `field()` + `Input::appearance(false)`; toggles with `segmented`.
5. Loading, empty and error states with the shapes above; nothing stays blank while reading.
6. Footer with secondary `ghost` buttons on the left and a single `primary` on the right; `.loading` and `.disabled` reflect work in progress.
7. New icons added to `ExtraIcons`.
8. Animations only with `appear`/`appear_after`/`pop_in`/`popover_in`/`pulse`, with stable ids.
9. Reviewed in a light theme, in a dark theme and with different transparencies.
10. Sizes in rems (`rems(px / 16.)`), and checked at 80 % and 150 % interface size: nothing clips or overlaps.
