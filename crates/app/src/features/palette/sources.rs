//! What the switcher reads from disk: files under the workspace root and
//! local git branches; plus `git switch` for the branch rows.

use std::path::Path;
use std::process::{Command, Stdio};

use gentle_core::fuzzy::FileEntry;
use serde_json::Value;

/// Indexed remote files, local branches and the truncation flag.
type RemotePaletteSnapshot = (Vec<FileEntry>, Vec<(String, bool)>, bool);

/// Private Herdr palette response. Paths are relative POSIX strings and
/// never become Windows PathBuf values. Invalid or unbounded data is rejected.
pub(super) fn parse_remote_palette(data: &Value) -> Option<RemotePaletteSnapshot> {
    let entries = data["files"].as_array()?;
    let refs = data["branches"].as_array()?;
    if entries.len() > 2000 || refs.len() > 120 {
        return None;
    }
    let valid_relative = |name: &str| {
        !name.is_empty() && name.len() <= 2048
            && !name.starts_with('/') && !name.contains('\\')
            && !name.contains(':') && !name.chars().any(char::is_control)
            && !name.split('/').any(|part| part.is_empty()
                || part == "." || part == ".." || part == ".git")
    };
    let files = entries.iter().map(|entry| {
        let path = entry.as_str()?;
        valid_relative(path).then(|| FileEntry::new(path.to_owned()))
    }).collect::<Option<Vec<_>>>()?;
    let branches = refs.iter().map(|entry| {
        let name = entry["name"].as_str()?;
        let current = entry["current"].as_bool()?;
        (!name.is_empty() && name.len() <= 512
            && !name.chars().any(char::is_control))
            .then(|| (name.to_owned(), current))
    }).collect::<Option<Vec<_>>>()?;
    Some((files, branches, data["truncated"].as_bool()?))
}

/// Files under `root`, relative, honoring `.gitignore`; at most `cap`.
pub(super) fn walk_files(root: &Path, cap: usize) -> Vec<String> {
    ignore::WalkBuilder::new(root)
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|entry| {
            entry
                .path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .take(cap)
        .collect()
}

fn git_command(root: &Path) -> Command {
    let mut command = pi_rpc::command("git");
    command
        .arg("-C")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Local branches, `(name, is_current)`; empty outside a repository.
pub(super) fn list_branches(root: &Path) -> Vec<(String, bool)> {
    let Ok(out) = git_command(root)
        .args(["branch", "--format=%(HEAD)%(refname:short)"])
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse_branches(&String::from_utf8_lossy(&out.stdout))
}

fn parse_branches(raw: &str) -> Vec<(String, bool)> {
    raw.lines()
        .filter_map(|line| {
            let (head, name) = line.split_at_checked(1)?;
            let name = name.trim();
            (!name.is_empty() && !name.starts_with('(')).then(|| (name.to_string(), head == "*"))
        })
        .collect()
}

/// `git switch <branch>` in `root`; the error is git's own message.
pub fn switch_branch(root: &Path, branch: &str) -> Result<(), String> {
    let out = git_command(root)
        .args(["switch", branch])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_branches, parse_remote_palette};
    use serde_json::json;

    #[test]
    fn remote_palette_paths_cannot_escape_or_be_treated_as_windows_paths() {
        let valid = json!({
            "files": ["src/main.rs", "README.md"],
            "branches": [{"name": "main", "current": true}],
            "truncated": false
        });
        let (files, branches, truncated) = parse_remote_palette(&valid).unwrap();
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(branches, vec![("main".to_string(), true)]);
        assert!(!truncated);
        for unsafe_name in ["../secrets", "/etc/passwd", "C:/Users/test",
                            "dir\\file", "folder/.git/config", "x/./y"] {
            let mut data = valid.clone();
            data["files"] = json!([unsafe_name]);
            assert!(parse_remote_palette(&data).is_none(), "{unsafe_name}");
        }
        let mut invalid = valid.clone();
        invalid["branches"] = json!([{"name": "bad\nbranch", "current": true}]);
        assert!(parse_remote_palette(&invalid).is_none());
    }

    #[test]
    fn branches_parse_with_the_current_one_marked() {
        let raw = "*main\n feature/x\n (HEAD detached at 1234)\n\n";
        assert_eq!(
            parse_branches(raw),
            vec![("main".to_string(), true), ("feature/x".to_string(), false)]
        );
    }
}
