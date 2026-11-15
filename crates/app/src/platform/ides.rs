//! The code editors and IDEs installed on this machine, and opening a folder
//! in one. What to look for and how to launch it is `gentle_core::ides`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use gentle_core::ides::{self, Base, IDES, Lookup, Os};

/// An editor found on disk.
#[derive(Clone, Debug)]
pub struct Installed {
    pub name: &'static str,
    /// The app bundle (macOS) or the program to run.
    pub path: PathBuf,
}

/// Every catalogued editor present here, in catalog order. Reads the disk and
/// may run the login shell once for the PATH: call it off the main thread.
pub fn detect() -> Vec<Installed> {
    IDES.iter()
        .filter_map(|ide| {
            let path = ide.lookups(Os::CURRENT).iter().find_map(find)?;
            Some(Installed {
                name: ide.name,
                path,
            })
        })
        .collect()
}

/// Open `folder` in `ide`, without waiting for it.
pub fn open(ide: &Installed, folder: &Path) -> std::io::Result<()> {
    let (program, args) = ides::launch(Os::CURRENT, &ide.path, folder);
    let mut child = pi_rpc::command(program)
        .args(args)
        .current_dir(folder)
        .env("PATH", pi_rpc::child_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    // Reap it whenever it exits (`open` and most launchers return at once).
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn find(lookup: &Lookup) -> Option<PathBuf> {
    match lookup {
        Lookup::Command(name) => pi_rpc::find_program(name),
        Lookup::Path(base, path) => {
            let segments: Vec<&str> = path.split('/').collect();
            bases(*base).iter().find_map(|dir| resolve(dir, &segments))
        }
    }
}

fn bases(base: Base) -> Vec<PathBuf> {
    let env = |key: &str| std::env::var_os(key).map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = match base {
        Base::Home => dirs::home_dir().into_iter().collect(),
        Base::LocalAppData => env("LOCALAPPDATA").into_iter().collect(),
        Base::ProgramFiles => [
            env("ProgramFiles"),
            env("ProgramW6432"),
            env("ProgramFiles(x86)"),
        ]
        .into_iter()
        .flatten()
        .collect(),
        Base::Root => vec![PathBuf::from("/")],
    };
    dirs.dedup();
    dirs
}

/// `segments` under `dir` if it exists. A wildcard segment tries its matches
/// newest first, so a folder like `Microsoft Visual Studio/Installer` that
/// lacks the rest of the path is skipped.
fn resolve(dir: &Path, segments: &[&str]) -> Option<PathBuf> {
    let Some((first, rest)) = segments.split_first() else {
        return dir.exists().then(|| dir.to_path_buf());
    };
    if !first.ends_with('*') {
        return resolve(&dir.join(first), rest);
    }
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| ides::segment_matches(first, name))
        .collect();
    ides::newest_first(&mut names);
    names.iter().find_map(|name| resolve(&dir.join(name), rest))
}
