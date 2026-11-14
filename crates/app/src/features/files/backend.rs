//! Typed, source-neutral operations for file and Git views.
//!
//! A WorkspaceBackend owns the execution location. A remote operation cannot
//! silently fall back to the Windows filesystem or Windows Git. Call these
//! futures from the background executor; native operations are blocking.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use pi_rpc::RpcProcess;
use serde_json::json;
use sha2::{Digest as _, Sha256};

use crate::features::git;
use super::listing;

pub(super) type BackendFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'static>>;

#[derive(Clone)]
pub(super) enum WorkspacePath {
    Local(PathBuf),
    Herdr(String),
}

#[derive(Clone)]
pub(super) enum WorkspaceBackend {
    Local { root: PathBuf },
    Herdr { root: String, process: Arc<RpcProcess> },
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceDirEntry {
    pub name: String,
    pub kind: String,
    pub ignored: bool,
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceListing {
    pub entries: Vec<WorkspaceDirEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub(super) struct WorkspaceText {
    pub path: String,
    pub content: String,
    pub revision: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum GitFileAction {
    Stage,
    Unstage,
}

/// Revision fencing is mandatory on Herdr. The existing server independently
/// validates the repository root, HEAD, branch, index and selected file.
pub(super) struct HerdrGitGuard {
    pub toplevel: String,
    pub head_oid: Option<String>,
    pub current_branch: Option<String>,
    pub index_token: String,
}

pub(super) struct GitFileMutation {
    /// Relative to the Git repository, never an arbitrary shell argument.
    pub path: String,
    /// Local porcelain-v2 rename source; Herdr currently reports no renames.
    pub source: Option<String>,
    pub index: char,
    pub worktree: char,
    pub herdr_guard: Option<HerdrGitGuard>,
}

impl WorkspaceBackend {
    pub fn local(root: PathBuf) -> Self {
        Self::Local { root }
    }

    pub fn herdr(root: String, process: Arc<RpcProcess>) -> Self {
        Self::Herdr { root, process }
    }

    pub fn list(self, directory: WorkspacePath, show_ignored: bool) -> BackendFuture<WorkspaceListing> {
        Box::pin(async move {
            match (self, directory) {
                (Self::Local { root }, WorkspacePath::Local(directory)) => {
                    ensure_local_path(&root, &directory)?;
                    let entries = listing::list_dir(&directory, show_ignored)
                        .into_iter()
                        .map(|entry| WorkspaceDirEntry {
                            name: entry.name,
                            kind: if entry.is_dir { "directory" } else { "file" }.into(),
                            ignored: entry.ignored,
                        })
                        .collect();
                    Ok(WorkspaceListing { entries, truncated: false })
                }
                (Self::Herdr { root, process }, WorkspacePath::Herdr(directory)) => {
                    ensure_herdr_path(&root, &directory)?;
                    let reply = process.request(json!({
                        "type": "herdr_fs_list", "root": root, "path": directory
                    })).await.map_err(|error| error.to_string())?;
                    let data = reply.data;
                    let entries = data["entries"].as_array()
                        .ok_or("Invalid remote directory listing")?
                        .iter()
                        .map(|item| {
                            let name = item["name"].as_str()
                                .filter(|name| !name.is_empty() && !name.contains('/')
                                    && !name.contains('\\') && *name != "." && *name != ".."
                                    && !name.chars().any(char::is_control))
                                .ok_or("Invalid remote directory entry")?;
                            let kind = item["kind"].as_str()
                                .filter(|kind| matches!(*kind,
                                    "directory" | "file" | "symlink" | "other"))
                                .ok_or("Invalid remote directory entry kind")?;
                            Ok(WorkspaceDirEntry {
                                name: name.to_owned(), kind: kind.to_owned(), ignored: false,
                            })
                        })
                        .collect::<Result<Vec<_>, &'static str>>()
                        .map_err(str::to_owned)?;
                    Ok(WorkspaceListing {
                        entries, truncated: data["truncated"].as_bool().unwrap_or(false),
                    })
                }
                _ => Err("Workspace path does not belong to the selected backend".into()),
            }
        })
    }

    pub fn read(self, path: WorkspacePath) -> BackendFuture<WorkspaceText> {
        Box::pin(async move {
            match (self, path) {
                (Self::Local { root }, WorkspacePath::Local(path)) => {
                    ensure_local_path(&root, &path)?;
                    let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
                    if bytes.len() > 128 * 1024 || bytes.contains(&0) {
                        return Err("Preview requires a text file under 128 KiB".into());
                    }
                    let revision = format!("{:x}", Sha256::digest(&bytes));
                    let content = String::from_utf8(bytes).map_err(|error| error.to_string())?;
                    Ok(WorkspaceText {
                        path: path.to_string_lossy().into_owned(),
                        content, revision: Some(revision),
                    })
                }
                (Self::Herdr { root, process }, WorkspacePath::Herdr(path)) => {
                    ensure_herdr_path(&root, &path)?;
                    let reply = process.request(json!({
                        "type": "herdr_fs_read", "root": root, "path": path,
                    })).await.map_err(|error| error.to_string())?;
                    let data = reply.data;
                    let content = data["content"].as_str()
                        .ok_or("Invalid remote file content")?.to_owned();
                    if content.len() > 128 * 1024 || content.contains('\0') {
                        return Err("Remote file exceeds the text preview limit".into());
                    }
                    let revision = data["revision"].as_str()
                        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                        .map(str::to_owned);
                    Ok(WorkspaceText { path, content, revision })
                }
                _ => Err("Workspace path does not belong to the selected backend".into()),
            }
        })
    }

    /// Create only a single new item under an already selected folder.
    /// Herdr's bridge enforces no-clobber O_EXCL and validates the agent root.
    pub fn create(
        self, parent: WorkspacePath, name: String, directory: bool,
    ) -> BackendFuture<WorkspacePath> {
        Box::pin(async move {
            if name.is_empty() || name.len() > 255
                || name == "." || name == ".." || name == ".git"
                || name.starts_with(".herdr-save-")
                || name.contains('/') || name.contains('\\')
                || name.chars().any(char::is_control)
            {
                return Err("Invalid new file or folder name".into());
            }
            match (self, parent) {
                (Self::Local { root }, WorkspacePath::Local(parent)) => {
                    ensure_local_path(&root, &parent)?;
                    let path = parent.join(&name);
                    if directory {
                        std::fs::create_dir(&path)
                            .map_err(|error| error.to_string())?;
                    } else {
                        std::fs::OpenOptions::new().write(true).create_new(true)
                            .open(&path).map_err(|error| error.to_string())?;
                    }
                    Ok(WorkspacePath::Local(path))
                }
                (Self::Herdr { root, process }, WorkspacePath::Herdr(parent)) => {
                    ensure_herdr_path(&root, &parent)?;
                    let target = if parent == "/" {
                        format!("/{name}")
                    } else {
                        format!("{parent}/{name}")
                    };
                    ensure_herdr_path(&root, &target)?;
                    let reply = process.request(json!({
                        "type": "herdr_fs_create",
                        "root": root,
                        "path": parent,
                        "name": name,
                        "kind": if directory { "directory" } else { "file" },
                    })).await.map_err(|error| error.to_string())?;
                    if reply.data["path"].as_str() != Some(target.as_str()) {
                        return Err("Remote create returned an unexpected path".into());
                    }
                    Ok(WorkspacePath::Herdr(target))
                }
                _ => Err("Workspace path does not belong to the selected backend".into()),
            }
        })
    }

    /// Existing-file UTF-8 save, fenced by a SHA-256 read revision.
    /// Herdr enforces the guard again server-side, including symlink checks.
    pub fn write(
        self, path: WorkspacePath, content: String, expected_revision: String,
    ) -> BackendFuture<String> {
        Box::pin(async move {
            if expected_revision.len() != 64
                || !expected_revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("Missing or invalid file revision".into());
            }
            if content.len() > 128 * 1024 || content.contains('\0') {
                return Err("Only UTF-8 text under 128 KiB can be saved".into());
            }
            match (self, path) {
                (Self::Local { root }, WorkspacePath::Local(path)) => {
                    ensure_local_path(&root, &path)?;
                    let metadata = std::fs::symlink_metadata(&path)
                        .map_err(|error| error.to_string())?;
                    if !metadata.is_file() || metadata.len() > 128 * 1024 {
                        return Err("Only an existing regular text file may be saved".into());
                    }
                    let original = std::fs::read(&path)
                        .map_err(|error| error.to_string())?;
                    if original.contains(&0) || std::str::from_utf8(&original).is_err() {
                        return Err("The original file is not UTF-8 text".into());
                    }
                    if format!("{:x}", Sha256::digest(&original)) != expected_revision {
                        return Err("Conflict: file changed since it was read".into());
                    }
                    std::fs::write(&path, &content)
                        .map_err(|error| error.to_string())?;
                    Ok(format!("{:x}", Sha256::digest(content.as_bytes())))
                }
                (Self::Herdr { root, process }, WorkspacePath::Herdr(path)) => {
                    ensure_herdr_path(&root, &path)?;
                    let reply = process.request(json!({
                        "type": "herdr_fs_write", "root": root,
                        "path": path, "content": content,
                        "expectedRevision": expected_revision,
                    })).await.map_err(|error| error.to_string())?;
                    let revision = reply.data["revision"].as_str()
                        .filter(|value| value.len() == 64
                            && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
                        .ok_or("Write response had no valid revision")?;
                    Ok(revision.to_owned())
                }
                _ => Err("Workspace path does not belong to the selected backend".into()),
            }
        })
    }

    pub fn git_file(
        self, action: GitFileAction, request: GitFileMutation,
    ) -> BackendFuture<()> {
        Box::pin(async move {
            safe_git_relative(&request.path)?;
            if let Some(source) = request.source.as_deref() {
                safe_git_relative(source)?;
            }
            match self {
                Self::Local { root } => {
                    if request.herdr_guard.is_some() {
                        return Err("Remote revision cannot be used with local Git".into());
                    }
                    let mut files = vec![request.path];
                    if let Some(source) = request.source {
                        files.push(source);
                    }
                    match action {
                        GitFileAction::Stage => git::stage(&root, &files),
                        GitFileAction::Unstage => git::unstage(&root, &files),
                    }
                }
                Self::Herdr { root, process } => {
                    if request.source.is_some() {
                        return Err("Remote rename staging requires server support".into());
                    }
                    let guard = request.herdr_guard
                        .ok_or("Missing remote Git revision guard")?;
                    ensure_herdr_path(&root, &guard.toplevel)?;
                    if guard.index_token.len() != 64
                        || !guard.index_token.bytes().all(|byte| byte.is_ascii_hexdigit())
                    {
                        return Err("Invalid remote index revision".into());
                    }
                    let target = format!(
                        "{}/{}", guard.toplevel.trim_end_matches('/'), request.path
                    );
                    ensure_herdr_path(&guard.toplevel, &target)?;
                    let command = match action {
                        GitFileAction::Stage => "herdr_git_stage",
                        GitFileAction::Unstage => "herdr_git_unstage",
                    };
                    process.request(json!({
                        "type": command, "root": root, "toplevel": guard.toplevel,
                        "expectedHead": guard.head_oid,
                        "expectedBranch": guard.current_branch,
                        "expectedIndexToken": guard.index_token,
                        "path": target, "expectedIndex": request.index.to_string(),
                        "expectedWorktree": request.worktree.to_string(),
                    })).await.map_err(|error| error.to_string())?;
                    Ok(())
                }
            }
        })
    }
}

fn ensure_local_path(root: &Path, selected: &Path) -> Result<(), String> {
    // Preserve the native tree's deliberate navigation into symlinked
    // directories. Remote paths have a stricter server-side realpath guard.
    // Path::starts_with compares whole components, not string prefixes.
    if !selected.starts_with(root)
        || selected.components().any(|part| part == std::path::Component::ParentDir)
    {
        return Err("Path is outside the selected local workspace".into());
    }
    Ok(())
}

fn ensure_herdr_path(root: &str, selected: &str) -> Result<(), String> {
    if !root.starts_with('/') || root.contains('\\') || root.contains('\0')
        || root.split('/').any(|s| s == ".." || s == ".")
        || !selected.starts_with('/') || selected.contains('\\')
        || selected.chars().any(char::is_control)
        || selected.split('/').any(|s| s == ".." || s == ".")
        || !(root == "/" || selected == root.trim_end_matches('/') || selected.starts_with(&format!("{}/", root.trim_end_matches('/'))))
    {
        return Err("Path is outside the selected remote workspace".into());
    }
    Ok(())
}

fn safe_git_relative(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\')
        || path.contains(':') || path.chars().any(char::is_control)
        || path.split('/').any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err("Invalid relative Git path".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_paths_never_cross_between_local_and_remote() {
        assert!(ensure_herdr_path("/srv/project", "/srv/project/src/lib.rs").is_ok());
        assert!(ensure_herdr_path("/srv/project", "/srv/project2/file").is_err());
        assert!(ensure_herdr_path("/srv/project", "/srv/project/../secrets").is_err());
        assert!(ensure_herdr_path("/srv/project", r"C:\\Users\\x").is_err());
        assert!(safe_git_relative("src/test.rs").is_ok());
        for path in ["../outside", "/absolute", "src/../outside", "C:/Windows", ""] {
            assert!(safe_git_relative(path).is_err(), "{path}");
        }
    }

    #[test]
    fn remote_create_names_are_confined_to_the_parent() {
        for name in ["../etc", "a/b", "a\\b", "..", ".git", ""] {
            assert!(
                name.contains('/') || name.contains('\\') || name.is_empty()
                    || name == ".." || name == ".git",
                "{name}"
            );
        }
        assert!(ensure_herdr_path(
            "/srv/project", "/srv/project/subfolder/new-file.rs",
        ).is_ok());
        assert!(ensure_herdr_path(
            "/srv/project", "/srv/project-neighbor/secret",
        ).is_err());
    }

    #[test]
    fn local_workspace_paths_are_component_scoped() {
        let root = Path::new("workspace");
        assert!(ensure_local_path(root, Path::new("workspace/docs")).is_ok());
        assert!(ensure_local_path(root, Path::new("workspace-other/docs")).is_err());
        assert!(ensure_local_path(root, Path::new("workspace/../secret")).is_err());
    }

    #[test]
    fn revision_guards_are_stable_for_utf8_bytes() {
        assert_eq!(
            format!("{:x}", Sha256::digest("hello".as_bytes())),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        );
        assert!(ensure_herdr_path("/", "/tmp/agent/file.txt").is_ok());
        assert!(ensure_herdr_path("/srv/project/", "/srv/project/file").is_ok());
    }
}
