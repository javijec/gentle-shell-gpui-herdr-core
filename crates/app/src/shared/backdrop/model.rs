//! A theme's backdrop as data: what the theme file says (`background`), the
//! files it points to, and the single-file form themes are shared in.
//!
//! ```json
//! "background": {
//!   "scene": "moonlit" | "my-scene.js",   // a built-in scene, or a file beside the theme
//!   "sceneCode": "SCENES.mine = {…}",     // the scene inline (what shared files carry)
//!   "image": "art.gif" | "data:image/png;base64,…",
//!   "sky": ["#1a2a5e", "#0d1530"],         // a vertical gradient under everything
//!   "speed": 1, "cell": 22, "dim": 0.3, "veil": 0.5
//! }
//! ```
//!
//! Themes made for claude-desktop-themes (`"wallpaper": "<id>"` with
//! `<id>.js` beside it, `wall-sky` in `vars`) read the same way.

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use gpui_kit::{Image, ImageFormat};
use serde_json::{Map, Value, json};

/// The extension of a shared theme: one JSON file with its scene and image inside.
pub const SHARE_EXTENSION: &str = "gentletheme";

/// Larger scene files are refused.
const MAX_SCENE: u64 = 1 << 20;
/// Larger images are refused.
const MAX_IMAGE: u64 = 25 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    pub scene: Option<SceneCode>,
    pub image: Option<Arc<Image>>,
    /// Top to bottom; empty for none.
    pub sky: Vec<u32>,
    /// Seconds of scene time per second: 0 holds it still, at most 16.
    pub speed: f32,
    /// The ASCII cell size in pixels, 10 to 40.
    pub cell: f32,
    /// How much the image is darkened toward the theme's background, 0 to 0.9.
    pub dim: f32,
    /// How much the middle of the window, where the conversation is, is
    /// calmed toward the background (the sides stay lively), 0 to 1.
    pub veil: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneCode {
    /// The built-in scene's name or the file's stem.
    pub name: String,
    pub code: Arc<str>,
}

/// Every `#rrggbb` in `text`, in order (a CSS gradient's stops, say).
fn hex_colors(text: &str) -> Vec<u32> {
    let bytes = text.as_bytes();
    let mut colors = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#'
            && bytes.len() >= i + 7
            && bytes[i + 1..i + 7].iter().all(u8::is_ascii_hexdigit)
            && !bytes.get(i + 7).is_some_and(u8::is_ascii_hexdigit)
        {
            colors.extend(u32::from_str_radix(&text[i + 1..i + 7], 16).ok());
            i += 7;
        } else {
            i += 1;
        }
    }
    colors
}

/// A plain file name beside the theme (no folders, nothing hidden): a
/// theme can't point at the rest of the disk.
fn beside(dir: Option<&Path>, name: &str) -> Option<std::path::PathBuf> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\'])
        && !name.contains("..");
    let path = dir?.join(name);
    (plain && path.is_file()).then_some(path)
}

fn read_limited(path: &Path, limit: u64) -> Option<Vec<u8>> {
    (std::fs::metadata(path).ok()?.len() <= limit)
        .then(|| std::fs::read(path).ok())
        .flatten()
}

/// `data:<mime>;base64,<data>` as an image.
fn data_image(uri: &str) -> Option<Image> {
    let rest = uri.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    let mime = meta.strip_suffix(";base64")?;
    let format = ImageFormat::from_mime_type(mime)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .ok()?;
    (bytes.len() as u64 <= MAX_IMAGE).then(|| Image::from_bytes(format, bytes))
}

fn file_image(path: &Path) -> Option<Image> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let format = ImageFormat::from_mime_type(&format!(
        "image/{}",
        if extension == "jpg" {
            "jpeg"
        } else {
            &extension
        }
    ))?;
    Some(Image::from_bytes(format, read_limited(path, MAX_IMAGE)?))
}

/// A scene by name: a `.js` file beside the theme, else a built-in one.
fn named_scene(name: &str, dir: Option<&Path>) -> Option<SceneCode> {
    let file = if name.ends_with(".js") {
        name.to_string()
    } else {
        format!("{name}.js")
    };
    if let Some(path) = beside(dir, &file) {
        let code = String::from_utf8(read_limited(&path, MAX_SCENE)?).ok()?;
        let stem = path.file_stem()?.to_string_lossy().into_owned();
        return Some(SceneCode {
            name: stem,
            code: code.into(),
        });
    }
    let name = name.trim_end_matches(".js");
    super::builtin_scene(name).map(|code| SceneCode {
        name: name.to_string(),
        code: code.into(),
    })
}

/// The backdrop of a theme file's JSON, or `None` when it has none.
/// `resolve` reads a color as the theme does (hex or a var name); `dir` is
/// the theme file's folder, where its scene and image files are.
pub fn parse(
    theme: &Value,
    dir: Option<&Path>,
    resolve: impl Fn(&Value) -> Option<u32>,
) -> Option<Backdrop> {
    let empty = Value::Object(Map::new());
    let background = theme.get("background").unwrap_or(&empty);
    let field = |key: &str| background.get(key).or_else(|| theme.get(key));
    let text = |key: &str| background.get(key).and_then(Value::as_str);

    let scene = match text("sceneCode") {
        Some(code) if (code.len() as u64) <= MAX_SCENE => Some(SceneCode {
            name: "inline".to_string(),
            code: code.into(),
        }),
        _ => text("scene")
            .or_else(|| theme.get("wallpaper").and_then(Value::as_str))
            .and_then(|name| named_scene(name, dir)),
    };
    let image = text("image").and_then(|image| {
        if image.starts_with("data:") {
            data_image(image)
        } else {
            file_image(&beside(dir, image)?)
        }
    });
    let sky = match background.get("sky") {
        Some(Value::Array(colors)) => colors.iter().filter_map(&resolve).collect(),
        Some(Value::String(css)) => hex_colors(css),
        _ => theme
            .pointer("/vars/wall-sky")
            .and_then(Value::as_str)
            .map(hex_colors)
            .unwrap_or_default(),
    };
    if scene.is_none() && image.is_none() && sky.is_empty() {
        return None;
    }
    let number = |key: &str| field(key).and_then(Value::as_f64).map(|n| n as f32);
    Some(Backdrop {
        scene,
        image: image.map(Arc::new),
        sky,
        speed: number("speed").unwrap_or(1.).clamp(0., 16.),
        cell: number("cell").unwrap_or(22.).clamp(10., 40.),
        dim: number("dim").unwrap_or(0.).clamp(0., 0.9),
        veil: number("veil").unwrap_or(0.5).clamp(0., 1.),
    })
}

/// The theme file `text` (in `dir`) as one self-contained file to share:
/// its scene and image are put inside, and a claude-desktop-themes
/// `wallpaper` becomes a `background`.
pub fn bundle(text: &str, dir: &Path) -> Result<String, String> {
    let mut theme: Value =
        serde_json::from_str(text).map_err(|e| format!("not a theme file: {e}"))?;
    let object = theme.as_object_mut().ok_or("not a theme file")?;
    let mut background = match object.remove("background") {
        Some(Value::Object(b)) => b,
        _ => Map::new(),
    };
    if let Some(Value::String(wallpaper)) = object.remove("wallpaper") {
        background
            .entry("scene")
            .or_insert(Value::String(wallpaper));
    }
    for key in ["speed", "cell"] {
        if let Some(value) = object.remove(key) {
            background.entry(key).or_insert(value);
        }
    }
    if let Some(Value::String(name)) = background.get("scene").cloned() {
        // A file beside the theme goes inside; a built-in name stays a name.
        let file = if name.ends_with(".js") {
            name.clone()
        } else {
            format!("{name}.js")
        };
        if let Some(path) = beside(Some(dir), &file) {
            let code = read_limited(&path, MAX_SCENE).ok_or("the scene file is too big")?;
            let code = String::from_utf8(code).map_err(|_| "the scene file isn't text")?;
            background.remove("scene");
            background.insert("sceneCode".into(), Value::String(code));
        } else if super::builtin_scene(name.trim_end_matches(".js")).is_none() {
            return Err(format!("the scene {name} isn't beside the theme"));
        }
    }
    if let Some(Value::String(name)) = background.get("image").cloned()
        && !name.starts_with("data:")
    {
        let image = beside(Some(dir), &name)
            .and_then(|path| file_image(&path))
            .ok_or_else(|| format!("the image {name} isn't beside the theme"))?;
        let data = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
        background.insert(
            "image".into(),
            Value::String(format!("data:{};base64,{data}", image.format.mime_type())),
        );
    }
    if !background.is_empty() {
        object.insert("background".into(), Value::Object(background));
    }
    serde_json::to_string_pretty(&theme).map_err(|e| e.to_string())
}

/// A file name for a theme called `name`: lowercase, dashes for the rest.
pub fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            stem.extend(c.to_lowercase());
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-').to_string();
    if stem.is_empty() {
        "theme".to_string()
    } else {
        stem
    }
}

/// The colors a scene sees as `g.theme`.
pub fn scene_colors(theme: &crate::shared::themes::Theme) -> String {
    let hex = |c: u32| format!("#{c:06x}");
    json!({
        "bg": hex(theme.bg),
        "text": hex(theme.fg),
        "muted": hex(theme.muted),
        "accent": hex(theme.accent),
        "info": hex(theme.info),
        "success": hex(theme.success),
        "warning": hex(theme.warning),
        "danger": hex(theme.danger),
        "light": theme.light,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(v: &Value) -> Option<u32> {
        v.as_str()
            .and_then(|s| s.strip_prefix('#'))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
    }

    fn temp() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gsg-backdrop-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_sky_from_a_list_or_a_css_gradient() {
        let theme = json!({"background": {"sky": ["#102030", "nope", "#405060"]}});
        let backdrop = parse(&theme, None, resolve).unwrap();
        assert_eq!(backdrop.sky, [0x102030, 0x405060]);

        // claude-desktop-themes keeps it as CSS in `wall-sky`.
        let theme = json!({"vars": {"wall-sky":
            "radial-gradient(120% 90% at 75% 10%, #1a2a5e, #0d1530 60%, #090f24)"}});
        let backdrop = parse(&theme, None, resolve).unwrap();
        assert_eq!(backdrop.sky, [0x1a2a5e, 0x0d1530, 0x090f24]);
        assert_eq!((backdrop.speed, backdrop.cell, backdrop.dim), (1., 22., 0.));
    }

    #[test]
    fn nothing_to_draw_is_no_backdrop_and_numbers_are_clamped() {
        assert_eq!(parse(&json!({"name": "x"}), None, resolve), None);
        let theme = json!({"background": {"scene": "moonlit", "speed": 99, "cell": 2, "dim": 5}});
        let backdrop = parse(&theme, None, resolve).unwrap();
        assert_eq!(backdrop.scene.unwrap().name, "moonlit");
        assert_eq!(
            (backdrop.speed, backdrop.cell, backdrop.dim),
            (16., 10., 0.9)
        );
    }

    #[test]
    fn scene_files_must_sit_beside_the_theme() {
        let dir = temp();
        std::fs::write(dir.join("mine.js"), "SCENES.mine = {}").unwrap();
        let theme = json!({"background": {"scene": "mine.js"}});
        let scene = parse(&theme, Some(&dir), resolve).unwrap().scene.unwrap();
        assert_eq!(
            (scene.name.as_str(), &*scene.code),
            ("mine", "SCENES.mine = {}")
        );

        // A path out of the folder is not followed.
        let theme = json!({"background": {"scene": "../mine.js", "sky": ["#000000"]}});
        assert_eq!(parse(&theme, Some(&dir), resolve).unwrap().scene, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_shared_file_carries_its_scene_and_image_and_reads_back() {
        let dir = temp();
        std::fs::write(dir.join("koi.js"), "SCENES.koi = { draw() {} }").unwrap();
        // A 1×1 PNG.
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=")
            .unwrap();
        std::fs::write(dir.join("art.png"), &png).unwrap();
        // The claude-desktop-themes shape: `wallpaper` and `speed` at the top.
        let text = r##"{"name": "Koi", "bg": "#11332d", "wallpaper": "koi", "speed": 2,
            "background": {"image": "art.png"}}"##;
        let shared = bundle(text, &dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        let value: Value = serde_json::from_str(&shared).unwrap();
        assert_eq!(value.get("wallpaper"), None);
        let backdrop = parse(&value, None, resolve).unwrap();
        assert_eq!(&*backdrop.scene.unwrap().code, "SCENES.koi = { draw() {} }");
        assert_eq!(backdrop.image.unwrap().bytes, png);
        assert_eq!(backdrop.speed, 2.);
    }

    #[test]
    fn sharing_refuses_a_missing_scene_but_keeps_built_in_names() {
        let dir = temp();
        let missing = bundle(r#"{"name": "x", "background": {"scene": "gone.js"}}"#, &dir);
        assert!(missing.is_err());
        let built_in =
            bundle(r#"{"name": "x", "background": {"scene": "moonlit"}}"#, &dir).unwrap();
        assert!(built_in.contains(r#""scene": "moonlit""#));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_stems() {
        assert_eq!(file_stem("  Sakura Dusk!! v2 "), "sakura-dusk-v2");
        assert_eq!(file_stem("Ñandú"), "ñandú");
        assert_eq!(file_stem("///"), "theme");
    }
}
