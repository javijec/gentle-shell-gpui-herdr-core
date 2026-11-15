//! The History screen: a branch's commits, how many there are in all, the
//! local branches to pick from, and whether the work tree has changes.

use std::path::Path;

use gentle_core::git::log::{Commit, LOG_ARGS, parse_log};

use super::git;

/// One read of the History screen.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    /// Newest first, at most the limit asked for.
    pub commits: Vec<Commit>,
    /// Every commit reachable from the branch, loaded or not.
    pub total: usize,
    /// The branch shown: the one asked for, else HEAD's. `None` on a
    /// detached HEAD.
    pub branch: Option<String>,
    /// HEAD's branch.
    pub current: Option<String>,
    /// Local branches, most recently committed first.
    pub branches: Vec<String>,
    /// The work tree has changes not yet committed. Only read for HEAD.
    pub dirty: bool,
}

/// The newest `limit` commits of `branch` (HEAD when `None`). `None` outside
/// a repository; a repository without commits reads as an empty history.
pub fn history(toplevel: &Path, branch: Option<&str>, limit: usize) -> Option<History> {
    let current = git(toplevel, &["symbolic-ref", "--short", "-q", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let branches: Vec<String> = git(
        toplevel,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname:short)",
            "refs/heads",
        ],
    )?
    .lines()
    .map(str::to_string)
    .collect();
    // A branch deleted since it was picked falls back to HEAD.
    let branch = branch
        .filter(|b| branches.iter().any(|known| known == b))
        .map(str::to_string);
    let rev = branch.clone().unwrap_or_else(|| "HEAD".to_string());
    let on_head = branch.is_none() || branch == current;
    let limit = format!("-n{limit}");
    let mut args: Vec<&str> = LOG_ARGS.to_vec();
    args.extend([limit.as_str(), rev.as_str(), "--"]);
    let commits = git(toplevel, &args)
        .map(|raw| parse_log(&raw))
        .unwrap_or_default();
    let total = git(toplevel, &["rev-list", "--count", rev.as_str(), "--"])
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(commits.len());
    let dirty = on_head
        && git(
            toplevel,
            &["status", "--porcelain", "--untracked-files=normal"],
        )
        .is_some_and(|s| !s.trim().is_empty());
    Some(History {
        commits,
        total,
        branch: branch.or_else(|| current.clone()),
        current,
        branches,
        dirty,
    })
}
