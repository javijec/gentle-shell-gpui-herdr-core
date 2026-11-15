# ADR-0004: Draw Mermaid diagrams with merman's CLI, downloaded on demand

- Status: accepted
- Date: 2026-11-13
- Affects: `features/files` (Markdown preview), `features/docs`, `features/setup`, `core::markdown`

## Context

Architecture documentation written as code (ADRs, C4, state, sequence and ER diagrams) puts its diagrams in ` ```mermaid ` blocks, and agents now write that documentation (the `architecture-docs` skill, the Architecture docs screen). The Markdown preview showed those blocks as source. Mermaid's reference renderer is JavaScript, and the app draws with GPUI, not a browser.

Most people who use the app will never open a document with a diagram, so whatever draws them shouldn't weigh on everyone's download.

## Decision

Draw each Mermaid block with [merman](https://github.com/Latias94/merman)'s CLI (pure Rust, MIT OR Apache-2.0), installed into the app's toolchain folder (`toolchain/bin/merman-cli`) only when someone asks for it: from the preview of a document that has diagrams, or from the Architecture docs screen. The release is pinned (`MERMAN_VERSION`, 0.10.0) and every download is checked against the `.sha256` its release publishes next to it; there are builds for macOS (arm64, x86_64), Linux (x86_64, arm64) and Windows x86_64.

`core::markdown::mermaid_blocks` finds the blocks; `features/files/mermaid.rs` pipes each one to `merman-cli render - -o - -f svg --svg-pipeline resvg-safe` off the UI thread and swaps the block for the SVG image in the preview's text, and GPUI rasterizes it like any other SVG. Text is measured with the same system sans font GPUI draws it with (Arial, Helvetica, Segoe UI, DejaVu Sans or Liberation Sans, found with `fontdb` and passed as `--font-file`). Diagrams follow the app theme (Mermaid's `dark` or `default` theme).

Without the renderer, a document's diagrams stay as source and the preview says how many there are, with an Install button.

## Alternatives considered

| Option | Why not |
| --- | --- |
| merman linked into the app | Same drawings, but +18 MB (41.6 → 59.8 MB, `release`, macOS arm64) for every user, diagrams or not. Downloading on demand costs the app +0.5 MB (`fontdb`, `lzma-rs` for `.tar.xz`). |
| mermaid.js in the in-app web view (`WebSurface`) | A native web view laid over GPUI's drawing: it can't scroll with the Markdown list, hides under screens, and costs a web process per preview. |
| mermaid-rs-renderer (~5 MB) | HTML in labels (`<br/>`, `<i>`) shows as text, chained edges (`a --> b --> c`) lose edges, and dense state diagrams collapse into a narrow column. |
| merman's default SVG | Labels are HTML (`foreignObject`), which resvg (GPUI's SVG renderer) doesn't draw: boxes come out empty. `--svg-pipeline resvg-safe` fixes that. |
| merman's built-in text metrics | Labels measured for one font and drawn with another overflow their boxes and get cut. |
| Leaving blocks as source | The documents the docs skill writes would be half code. |

## Consequences

- **Good:** diagrams render natively on macOS, Windows and Linux (no browser, no Node) for those who want them; everyone else downloads nothing extra.
- **Good:** a new merman release changes nothing until `MERMAN_VERSION` moves, together with its checksums' source.
- **Bad:** the first diagram needs a ~15–25 MB download (it depends on the platform) and a network connection; behind a proxy that blocks GitHub releases, diagrams stay as source.
- **Bad:** one process per diagram (tens of milliseconds each); a document with many diagrams draws them one after another, off the UI thread.
- **Bad:** switching the app theme redraws a diagram on the next edit, not at once.
- Diagram types merman doesn't draw show as source with its error under them.
