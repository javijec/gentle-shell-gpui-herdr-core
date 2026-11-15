//! Code editors and IDEs a workspace folder can be opened in: where each one
//! installs on macOS, Windows and Linux, and the command that opens a folder
//! in it. Looking on disk is the app's job (`platform/ides.rs`).

use std::cmp::Ordering;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub const CURRENT: Os = if cfg!(target_os = "macos") {
        Os::Mac
    } else if cfg!(windows) {
        Os::Windows
    } else {
        Os::Linux
    };
}

/// The folder an install path starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    /// The user's home folder.
    Home,
    /// `%LOCALAPPDATA%` (per-user installs on Windows).
    LocalAppData,
    /// `%ProgramFiles%` and `%ProgramFiles(x86)%`.
    ProgramFiles,
    /// `/` on macOS and Linux.
    Root,
}

/// One place an editor may be, in order of preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// A `/`-separated path under a base. A segment ending in `*` matches any
    /// entry starting with the rest (versioned folders such as
    /// `IntelliJ IDEA 2025.1`); the newest-named one wins ([`newest`]).
    Path(Base, &'static str),
    /// A command on the search PATH.
    Command(&'static str),
    /// A Flatpak app's exported launcher, system-wide or per user.
    Flatpak(&'static str),
}

/// Where to check on disk: a [`Spot`] with macOS app folders and Flatpak
/// exports spelled out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lookup {
    Path(Base, String),
    Command(&'static str),
}

#[derive(Debug)]
pub struct Ide {
    pub name: &'static str,
    /// macOS app bundles (`Zed.app`), looked up in the usual app folders.
    mac: &'static [&'static str],
    windows: &'static [Spot],
    linux: &'static [Spot],
}

/// Folders macOS apps live in: system-wide, per user (JetBrains Toolbox
/// installs there too) and Setapp's.
const MAC_APP_DIRS: [(Base, &str); 4] = [
    (Base::Root, "Applications"),
    (Base::Home, "Applications"),
    (Base::Home, "Applications/JetBrains Toolbox"),
    (Base::Root, "Applications/Setapp"),
];

impl Ide {
    /// Where to look on `os`, most preferred first; the first hit is used.
    pub fn lookups(&self, os: Os) -> Vec<Lookup> {
        let spots = match os {
            Os::Mac => {
                return self
                    .mac
                    .iter()
                    .flat_map(|app| {
                        MAC_APP_DIRS
                            .iter()
                            .map(move |(base, dir)| Lookup::Path(*base, format!("{dir}/{app}")))
                    })
                    .collect();
            }
            Os::Windows => self.windows,
            Os::Linux => self.linux,
        };
        spots
            .iter()
            .flat_map(|spot| match *spot {
                Spot::Path(base, path) => vec![Lookup::Path(base, path.into())],
                Spot::Command(name) => vec![Lookup::Command(name)],
                Spot::Flatpak(id) => vec![
                    Lookup::Path(Base::Root, format!("var/lib/flatpak/exports/bin/{id}")),
                    Lookup::Path(Base::Home, format!(".local/share/flatpak/exports/bin/{id}")),
                ],
            })
            .collect()
    }
}

/// A JetBrains IDE: `Program Files\JetBrains\<Name> <version>\bin\<exe>`
/// (installer) or `%LOCALAPPDATA%\Programs\<Name>\bin\<exe>` (Toolbox) on
/// Windows; the launcher on the PATH or Toolbox's script on Linux.
macro_rules! jetbrains {
    ($name:literal, [$($app:literal),+], $exe:literal, $script:literal $(, $extra:expr)*) => {
        Ide {
            name: $name,
            mac: &[$($app),+],
            windows: &[
                Spot::Path(Base::ProgramFiles, concat!("JetBrains/", $name, "*/bin/", $exe)),
                Spot::Path(Base::LocalAppData, concat!("Programs/", $name, "*/bin/", $exe)),
            ],
            linux: &[
                Spot::Command($script),
                Spot::Path(Base::Home, concat!(".local/share/JetBrains/Toolbox/scripts/", $script)),
                $($extra,)*
            ],
        }
    };
}

pub const IDES: &[Ide] = &[
    Ide {
        name: "VS Code",
        mac: &["Visual Studio Code.app"],
        windows: &[
            Spot::Path(Base::LocalAppData, "Programs/Microsoft VS Code/Code.exe"),
            Spot::Path(Base::ProgramFiles, "Microsoft VS Code/Code.exe"),
        ],
        linux: &[
            Spot::Command("code"),
            Spot::Path(Base::Root, "snap/bin/code"),
            Spot::Flatpak("com.visualstudio.code"),
        ],
    },
    Ide {
        name: "VS Code Insiders",
        mac: &["Visual Studio Code - Insiders.app"],
        windows: &[
            Spot::Path(
                Base::LocalAppData,
                "Programs/Microsoft VS Code Insiders/Code - Insiders.exe",
            ),
            Spot::Path(
                Base::ProgramFiles,
                "Microsoft VS Code Insiders/Code - Insiders.exe",
            ),
        ],
        linux: &[Spot::Command("code-insiders")],
    },
    Ide {
        name: "Cursor",
        mac: &["Cursor.app"],
        windows: &[Spot::Path(Base::LocalAppData, "Programs/cursor/Cursor.exe")],
        linux: &[Spot::Command("cursor")],
    },
    Ide {
        name: "Windsurf",
        mac: &["Windsurf.app"],
        windows: &[Spot::Path(
            Base::LocalAppData,
            "Programs/Windsurf/Windsurf.exe",
        )],
        linux: &[Spot::Command("windsurf")],
    },
    Ide {
        name: "Kiro",
        mac: &["Kiro.app"],
        windows: &[Spot::Path(Base::LocalAppData, "Programs/Kiro/Kiro.exe")],
        linux: &[Spot::Command("kiro")],
    },
    Ide {
        name: "VSCodium",
        mac: &["VSCodium.app"],
        windows: &[
            Spot::Path(Base::LocalAppData, "Programs/VSCodium/VSCodium.exe"),
            Spot::Path(Base::ProgramFiles, "VSCodium/VSCodium.exe"),
        ],
        linux: &[
            Spot::Command("codium"),
            Spot::Path(Base::Root, "snap/bin/codium"),
            Spot::Flatpak("com.vscodium.codium"),
        ],
    },
    Ide {
        name: "Zed",
        mac: &["Zed.app", "Zed Preview.app"],
        windows: &[Spot::Path(Base::LocalAppData, "Programs/Zed/Zed.exe")],
        linux: &[
            Spot::Command("zed"),
            Spot::Command("zeditor"),
            Spot::Path(Base::Home, ".local/bin/zed"),
            Spot::Flatpak("dev.zed.Zed"),
        ],
    },
    Ide {
        name: "Sublime Text",
        mac: &["Sublime Text.app"],
        windows: &[
            Spot::Path(Base::ProgramFiles, "Sublime Text/sublime_text.exe"),
            Spot::Path(Base::ProgramFiles, "Sublime Text 3/sublime_text.exe"),
        ],
        linux: &[
            Spot::Command("subl"),
            Spot::Path(Base::Root, "opt/sublime_text/sublime_text"),
            Spot::Flatpak("com.sublimetext.three"),
        ],
    },
    Ide {
        name: "Xcode",
        mac: &["Xcode.app", "Xcode-beta.app"],
        windows: &[],
        linux: &[],
    },
    Ide {
        name: "Visual Studio",
        mac: &[],
        windows: &[Spot::Path(
            Base::ProgramFiles,
            "Microsoft Visual Studio/*/*/Common7/IDE/devenv.exe",
        )],
        linux: &[],
    },
    Ide {
        name: "Android Studio",
        mac: &["Android Studio.app"],
        windows: &[Spot::Path(
            Base::ProgramFiles,
            "Android/Android Studio/bin/studio64.exe",
        )],
        linux: &[
            Spot::Command("android-studio"),
            Spot::Path(Base::Home, ".local/share/JetBrains/Toolbox/scripts/studio"),
            Spot::Path(Base::Root, "opt/android-studio/bin/studio.sh"),
            Spot::Path(Base::Root, "snap/bin/android-studio"),
        ],
    },
    jetbrains!(
        "IntelliJ IDEA",
        [
            "IntelliJ IDEA.app",
            "IntelliJ IDEA Ultimate.app",
            "IntelliJ IDEA CE.app",
            "IntelliJ IDEA Community Edition.app"
        ],
        "idea64.exe",
        "idea",
        Spot::Path(Base::Root, "snap/bin/intellij-idea-ultimate"),
        Spot::Path(Base::Root, "snap/bin/intellij-idea-community")
    ),
    jetbrains!("WebStorm", ["WebStorm.app"], "webstorm64.exe", "webstorm"),
    jetbrains!(
        "PyCharm",
        [
            "PyCharm.app",
            "PyCharm Professional Edition.app",
            "PyCharm CE.app",
            "PyCharm Community Edition.app"
        ],
        "pycharm64.exe",
        "pycharm",
        Spot::Path(Base::Root, "snap/bin/pycharm-professional"),
        Spot::Path(Base::Root, "snap/bin/pycharm-community")
    ),
    jetbrains!("GoLand", ["GoLand.app"], "goland64.exe", "goland"),
    jetbrains!(
        "RustRover",
        ["RustRover.app"],
        "rustrover64.exe",
        "rustrover"
    ),
    jetbrains!("CLion", ["CLion.app"], "clion64.exe", "clion"),
    jetbrains!("PhpStorm", ["PhpStorm.app"], "phpstorm64.exe", "phpstorm"),
    jetbrains!("Rider", ["Rider.app"], "rider64.exe", "rider"),
    Ide {
        name: "Nova",
        mac: &["Nova.app"],
        windows: &[],
        linux: &[],
    },
    Ide {
        name: "BBEdit",
        mac: &["BBEdit.app"],
        windows: &[],
        linux: &[],
    },
];

/// Whether directory entry `name` matches path segment `pattern`: equal, or
/// starting with what precedes a trailing `*`. Case-insensitive, as Windows
/// and macOS file names are.
pub fn segment_matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_lowercase(), name.to_lowercase());
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == pattern,
    }
}

/// Order versioned names newest first: digit runs compare as numbers, so
/// `IntelliJ IDEA 2025.10` comes before `IntelliJ IDEA 2025.9`.
pub fn newest_first<S: AsRef<str>>(names: &mut [S]) {
    names.sort_by(|a, b| natural_cmp(b.as_ref(), a.as_ref()));
}

fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.chars().next(), b.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let split = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
                let (na, nb) = (split(a), split(b));
                let (da, db) = (
                    a[..na].trim_start_matches('0'),
                    b[..nb].trim_start_matches('0'),
                );
                let order = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
                if order != Ordering::Equal {
                    return order;
                }
                (a, b) = (&a[na..], &b[nb..]);
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
            }
        }
    }
}

/// The program and arguments that open `folder` in the editor found at
/// `found`: `open -a <App.app> <folder>` on macOS, the editor's own program
/// with the folder elsewhere.
pub fn launch(os: Os, found: &Path, folder: &Path) -> (PathBuf, Vec<OsString>) {
    match os {
        Os::Mac => (
            PathBuf::from("/usr/bin/open"),
            vec!["-a".into(), found.into(), folder.into()],
        ),
        Os::Windows | Os::Linux => (found.to_path_buf(), vec![folder.into()]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_segment_matches_versioned_folders_case_insensitively() {
        assert!(segment_matches("IntelliJ IDEA*", "IntelliJ IDEA 2025.1"));
        assert!(segment_matches(
            "IntelliJ IDEA*",
            "intellij idea Community Edition 2024.3"
        ));
        assert!(segment_matches("Code.exe", "code.exe"));
        assert!(!segment_matches("Code.exe", "Code - Insiders.exe"));
        assert!(!segment_matches("WebStorm*", "IntelliJ IDEA 2025.1"));
    }

    #[test]
    fn newest_first_compares_version_numbers_numerically() {
        let mut names = [
            "IntelliJ IDEA 2025.9",
            "IntelliJ IDEA 2025.10",
            "IntelliJ IDEA 2024.3.1",
        ];
        newest_first(&mut names);
        assert_eq!(
            names,
            [
                "IntelliJ IDEA 2025.10",
                "IntelliJ IDEA 2025.9",
                "IntelliJ IDEA 2024.3.1"
            ]
        );
        let mut years = ["2019", "2022", "02020"];
        newest_first(&mut years);
        assert_eq!(years, ["2022", "02020", "2019"]);
    }

    #[test]
    fn mac_lookups_try_every_app_folder_for_each_bundle_in_order() {
        let zed = IDES.iter().find(|i| i.name == "Zed").unwrap();
        let lookups = zed.lookups(Os::Mac);
        assert_eq!(lookups.len(), 2 * MAC_APP_DIRS.len());
        assert_eq!(
            lookups[0],
            Lookup::Path(Base::Root, "Applications/Zed.app".into())
        );
        assert_eq!(
            lookups[1],
            Lookup::Path(Base::Home, "Applications/Zed.app".into())
        );
        assert!(lookups.contains(&Lookup::Path(
            Base::Home,
            "Applications/Zed Preview.app".into()
        )));
        let xcode = IDES.iter().find(|i| i.name == "Xcode").unwrap();
        assert!(xcode.lookups(Os::Linux).is_empty());
    }

    #[test]
    fn flatpak_spots_expand_to_system_and_user_exports() {
        let code = IDES.iter().find(|i| i.name == "VS Code").unwrap();
        let lookups = code.lookups(Os::Linux);
        assert_eq!(lookups[0], Lookup::Command("code"));
        assert!(lookups.contains(&Lookup::Path(
            Base::Root,
            "var/lib/flatpak/exports/bin/com.visualstudio.code".into()
        )));
        assert!(lookups.contains(&Lookup::Path(
            Base::Home,
            ".local/share/flatpak/exports/bin/com.visualstudio.code".into()
        )));
    }

    #[test]
    fn launch_uses_open_on_mac_and_the_program_elsewhere() {
        let folder = Path::new("/work/app");
        let (program, args) = launch(Os::Mac, Path::new("/Applications/Zed.app"), folder);
        assert_eq!(program, PathBuf::from("/usr/bin/open"));
        assert_eq!(
            args,
            ["-a", "/Applications/Zed.app", "/work/app"].map(OsString::from)
        );
        let (program, args) = launch(Os::Linux, Path::new("/usr/bin/code"), folder);
        assert_eq!(program, PathBuf::from("/usr/bin/code"));
        assert_eq!(args, [OsString::from("/work/app")]);
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<_> = IDES.iter().map(|i| i.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), IDES.len());
    }
}
