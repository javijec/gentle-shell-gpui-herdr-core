//! What the app tells every agent it starts (appended to the system prompt
//! with `--append-system-prompt`): pages open in the app's browser with
//! `gentle-open`; with "Agents can use devices" on, simulators and
//! emulators open in the device panel with `gentle-device` and are driven
//! with `agent-device`; and app themes go in the app's themes folder. The
//! point is that the user watches everything inside the app; nothing opens
//! a separate browser, Simulator or emulator window.

use std::path::{Path, PathBuf};

use crate::features::setup::devices::Prefs;

const PAGES: &str = "\
## Gentle Shell app: pages and devices

You run inside the Gentle Shell desktop app, where the user watches what you open.

- Web pages (a dev server, a preview, docs): run `gentle-open <url>`; the page opens in the app's \
browser. Never use `open`, `xdg-open`, `start` or launch a browser yourself.
";

const DEVICES: &str = "\
- iOS simulators and Android emulators: run `gentle-device list`, then `gentle-device open \"<name>\"`. \
It boots the device without a separate window, shows it in the app's device panel and prints the \
`agent-device` command pinned to it.
- Do every app launch, tap, text entry, screenshot and check on that device with that command: \
`open <bundle-or-package-id>`, `snapshot -i` (what's on screen, with @refs), `press @e3 --settle`, \
`fill @e5 \"text\" --settle`, `install <app>`, `screenshot <path>`.
- Never start or drive a device yourself: no `xcrun simctl boot`, `open -a Simulator`, \
`emulator -avd`, `adb shell input`, `idb`, or scripts that wrap them. Build with the project's \
usual tools, then install and launch through `agent-device`.
- `gentle-device close \"<name>\" --shutdown` turns a device off when you're done with it.
";

/// The guide to making app themes, written into the app's themes folder.
const THEME_GUIDE: &str = include_str!("../../../themes/GUIDE.md");

/// How an agent makes an app theme; `dir` is the app's themes folder, where
/// the guide is.
fn themes(dir: &Path) -> String {
    format!(
        "\
- App themes: when the user asks you to theme, restyle or decorate this app itself (its colors, an \
animated background or scene like falling petals, a koi pond, rain), first read `{guide}`, then write \
the theme (and its scene) into `{dir}`. The app shows it within seconds; tell the user to pick it in \
Configuration › Theme › Animated backgrounds. After writing a scene, check `{error}`: if it exists, the \
scene failed and it says why.
",
        guide = dir.join("GUIDE.md").display(),
        dir = dir.display(),
        error = dir.join("scene-error.txt").display(),
    )
}

/// Write `text` to `path` unless it already holds it.
fn write_if_changed(path: &Path, text: &str) -> Option<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(path, text).ok()?;
    }
    Some(())
}

/// The instructions for `prefs`, written to the app's data folder; the
/// file to append, or `None` when it can't be written.
pub fn agent_instructions(prefs: Prefs) -> Option<PathBuf> {
    let path = dirs::data_dir()?
        .join("gentle-shell-gpui")
        .join("agent-instructions.md");
    let mut text = PAGES.to_string();
    if prefs.enabled && prefs.agents {
        text.push_str(DEVICES);
    }
    if let Some(dir) = crate::shared::themes::app_themes_dir() {
        let scenes: Vec<String> = crate::shared::backdrop::builtin_scene_names()
            .map(|name| format!("`{name}`"))
            .collect();
        let guide = THEME_GUIDE.replace("BUILTIN_SCENES", &scenes.join(", "));
        if write_if_changed(&dir.join("GUIDE.md"), &guide).is_some() {
            text.push_str(&themes(&dir));
        }
    }
    write_if_changed(&path, &text)?;
    Some(path)
}
