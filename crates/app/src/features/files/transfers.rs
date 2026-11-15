//! Stage 7: binary-safe file transfers over an existing, machine-scoped
//! Herdr agent. The bridge owns Linux paths and never launches a shell.
//! Downloads are explicitly written to a local user-selected directory.
//! Uploads are atomic, SHA-256 checked and refuse to overwrite existing files.

use std::path::PathBuf;
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use pi_rpc::RpcProcess;
use serde_json::json;
use sha2::{Digest as _, Sha256};

pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
const CHUNK: usize = 24 * 1024;

fn hex_digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255
        && name != "." && name != ".." && name != ".git"
        && !name.starts_with(".herdr-upload-")
        && !name.starts_with(".herdr-save-")
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

fn valid_remote_path(root: &str, path: &str) -> bool {
    root.starts_with('/') && !root.contains('\\')
        && path.starts_with('/') && !path.contains('\\')
        && path.len() <= 2048 && !path.chars().any(char::is_control)
        && (path == root || path.starts_with(&(root.to_owned() + "/")))
        && !path.split('/').any(|part| part == "." || part == "..")
}

fn remote_filename(path: &str) -> Result<&str, String> {
    let name = path.rsplit('/').next().unwrap_or_default();
    if !valid_name(name) {
        return Err("Invalid remote transfer file name".into());
    }
    Ok(name)
}

pub(super) async fn fetch(
    root: String,
    process: Arc<RpcProcess>,
    path: String,
) -> Result<Vec<u8>, String> {
    if !valid_remote_path(&root, &path) || path == root {
        return Err("Remote transfer path is outside the selected agent".into());
    }
    let mut bytes = Vec::new();
    let mut epoch: Option<String> = None;
    let mut total: Option<usize> = None;
    loop {
        let reply = process.request(json!({
            "type": "herdr_fs_transfer_read",
            "root": root,
            "path": path,
            "offset": bytes.len(),
            "epoch": epoch,
        })).await.map_err(|error| error.to_string())?;
        let data = &reply.data;
        if data["path"].as_str() != Some(path.as_str())
            || data["offset"].as_u64() != Some(bytes.len() as u64)
        {
            return Err("Remote download returned an unexpected path or offset".into());
        }
        let size = data["size"].as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= MAX_BYTES)
            .ok_or("Remote file exceeds the 16 MiB transfer limit")?;
        if total.is_some_and(|old| old != size) {
            return Err("Remote file size changed during download".into());
        }
        total = Some(size);
        let revision = data["epoch"].as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or("Invalid remote transfer revision")?;
        if epoch.as_deref().is_some_and(|old| old != revision) {
            return Err("Remote file changed during download".into());
        }
        epoch = Some(revision.to_owned());
        let encoded = data["content"].as_str().ok_or("Missing binary file chunk")?;
        let chunk = STANDARD.decode(encoded).map_err(|error| error.to_string())?;
        if chunk.len() > CHUNK || bytes.len() + chunk.len() > size
            || data["chunkHash"].as_str() != Some(hex_digest(&chunk).as_str())
        {
            return Err("Remote file chunk failed SHA-256 integrity".into());
        }
        let complete = data["complete"].as_bool()
            .ok_or("Missing remote transfer completion status")?;
        if chunk.is_empty() && !complete {
            return Err("Remote file transfer stalled".into());
        }
        bytes.extend_from_slice(&chunk);
        if complete {
            if bytes.len() != size {
                return Err("Remote file transfer ended with incomplete content".into());
            }
            return Ok(bytes);
        }
    }
}

async fn put_bytes(
    root: String,
    process: Arc<RpcProcess>,
    parent: String,
    name: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    if root == "/" || !valid_remote_path(&root, &parent)
        || !valid_name(&name) || bytes.len() > MAX_BYTES
    {
        return Err("Invalid upload target, file name or size".into());
    }
    let digest = hex_digest(&bytes);
    let start = process.request(json!({
        "type": "herdr_fs_transfer_begin",
        "root": root, "path": parent, "name": name,
        "size": bytes.len(), "sha256": digest,
    })).await.map_err(|error| error.to_string())?;
    let id = start.data["transfer"].as_str()
        .filter(|id| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Invalid upload token")?.to_owned();
    let result: Result<String, String> = async {
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            let reply = process.request(json!({
                "type": "herdr_fs_transfer_chunk",
                "transfer": id,
                "offset": index * CHUNK,
                "content": STANDARD.encode(chunk),
                "chunkHash": hex_digest(chunk),
            })).await.map_err(|error| error.to_string())?;
            if reply.data["offset"].as_u64() != Some(((index * CHUNK) + chunk.len()) as u64) {
                return Err("Remote upload acknowledged an incorrect offset".into());
            }
        }
        let reply = process.request(json!({
            "type": "herdr_fs_transfer_commit", "root": root,
            "transfer": id,
        })).await.map_err(|error| error.to_string())?;
        let expected_path = format!("{}/{}", parent.trim_end_matches('/'), name);
        if reply.data["path"].as_str() != Some(expected_path.as_str())
            || reply.data["revision"].as_str() != Some(digest.as_str())
        {
            return Err("Remote upload returned the wrong file or SHA-256".into());
        }
        Ok(expected_path)
    }.await;
    if result.is_err() {
        let _ = process.request(json!({
            "type": "herdr_fs_transfer_abort", "root": root, "transfer": id,
        })).await;
    }
    result
}

/// Explicit upload of a local regular file into the selected Linux folder.
/// The local source is read only after the user chooses it in the OS picker.
pub(super) async fn upload(
    root: String,
    process: Arc<RpcProcess>,
    parent: String,
    local: PathBuf,
) -> Result<String, String> {
    let info = std::fs::symlink_metadata(&local).map_err(|e| e.to_string())?;
    if !info.is_file() || info.file_type().is_symlink() || info.len() > MAX_BYTES as u64 {
        return Err("Select a regular file under 16 MiB (symlinks are not uploaded)".into());
    }
    let name = local.file_name().and_then(|name| name.to_str())
        .filter(|name| valid_name(name))
        .ok_or("Invalid local file name")?.to_owned();
    let bytes = std::fs::read(&local).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("Selected file exceeds the 16 MiB limit".into());
    }
    put_bytes(root, process, parent, name, bytes).await
}

/// Explicit download into an OS-chosen directory; create_new prevents any
/// overwrite of existing Windows files. Partial files are removed on error.
pub(super) async fn download(
    root: String,
    process: Arc<RpcProcess>,
    remote: String,
    destination: PathBuf,
) -> Result<PathBuf, String> {
    let name = remote_filename(&remote)?.to_owned();
    let directory = std::fs::canonicalize(&destination).map_err(|e| e.to_string())?;
    if !directory.is_dir() {
        return Err("Select a local destination directory".into());
    }
    let target = directory.join(name);
    let bytes = fetch(root, process, remote).await?;
    let mut file = std::fs::OpenOptions::new()
        .write(true).create_new(true).open(&target)
        .map_err(|error| format!("Cannot create downloaded file (never overwrite): {error}"))?;
    use std::io::Write as _;
    if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&target);
        return Err(error.to_string());
    }
    Ok(target)
}

/// Same transfer stream between two independent Herdr agents, without
/// writing private Linux content into a Windows temporary file.
pub(crate) async fn relay(
    from_root: String,
    from_process: Arc<RpcProcess>,
    from_path: String,
    to_root: String,
    to_process: Arc<RpcProcess>,
    to_directory: String,
) -> Result<String, String> {
    let name = remote_filename(&from_path)?.to_owned();
    let bytes = fetch(from_root, from_process, from_path).await?;
    put_bytes(to_root, to_process, to_directory, name, bytes).await
}

#[cfg(test)]
mod tests {
    use super::{MAX_BYTES, remote_filename, valid_name, valid_remote_path};

    #[test]
    fn blocks_foreign_roots_and_windows_paths() {
        let root = "/srv/project";
        assert!(valid_remote_path(root, "/srv/project/src/file.bin"));
        assert!(!valid_remote_path(root, "/srv/project2/file.bin"));
        assert!(!valid_remote_path(root, "/srv/project/../secret"));
        assert!(!valid_remote_path(root, "C:\\Windows\\file.bin"));
        assert!(remote_filename("/srv/project/file.bin").is_ok());
        assert!(remote_filename("/srv/project/.git").is_err());
    }

    #[test]
    fn rejects_reserved_names_and_caps_content() {
        for bad in [".", "..", ".git", ".herdr-upload-a",
            ".herdr-save-a", "a/b", "a\\b", "bad\nname"] {
            assert!(!valid_name(bad), "{bad:?}");
        }
        assert_eq!(MAX_BYTES, 16 * 1024 * 1024);
    }
}
