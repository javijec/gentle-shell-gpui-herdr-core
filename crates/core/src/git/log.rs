//! Commit history for the History screen: the `git log` records it reads and
//! the lanes of the graph drawn beside them.

/// `git log` arguments whose output [`parse_log`] reads: each commit starts
/// with `\x1e`, its fields are split by NUL, and `--shortstat`'s line follows.
/// `--topo-order` keeps a merged branch's commits together, between the
/// merge and the commit it forked from.
pub const LOG_ARGS: &[&str] = &[
    "log",
    "--topo-order",
    "--decorate=full",
    "--shortstat",
    "--no-color",
    "--format=%x1e%H%x00%P%x00%an%x00%ct%x00%D%x00%s%x00%(trailers:key=Co-authored-by,valueonly,separator=%x1f)",
];

/// A ref pointing at a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefLabel {
    Branch(String),
    Remote(String),
    Tag(String),
}

/// What `--shortstat` counted. Merges have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stat {
    pub files: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub hash: String,
    pub parents: Vec<String>,
    pub author: String,
    /// Committer time, Unix seconds.
    pub time: i64,
    /// HEAD points at it.
    pub head: bool,
    pub refs: Vec<RefLabel>,
    pub subject: String,
    /// Names from `Co-authored-by` trailers, without their emails.
    pub coauthors: Vec<String>,
    pub stat: Option<Stat>,
}

impl Commit {
    pub fn short(&self) -> &str {
        self.hash.get(..7).unwrap_or(&self.hash)
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

/// Parse `git log` run with [`LOG_ARGS`]. A record without a hash is skipped.
pub fn parse_log(raw: &str) -> Vec<Commit> {
    raw.split('\x1e')
        .filter_map(|record| {
            let mut f = record.splitn(7, '\0');
            let hash = f.next()?.trim();
            if hash.is_empty() {
                return None;
            }
            let parents = f.next()?.split_whitespace().map(str::to_string).collect();
            let author = f.next()?.to_string();
            let time = f.next()?.trim().parse().unwrap_or(0);
            let (head, refs) = parse_decorations(f.next()?);
            let subject = f.next()?.to_string();
            // The trailers end at the first newline; the stat line comes after.
            let rest = f.next().unwrap_or_default();
            let (trailers, stat) = rest.split_once('\n').unwrap_or((rest, ""));
            Some(Commit {
                hash: hash.to_string(),
                parents,
                author,
                time,
                head,
                refs,
                subject,
                coauthors: trailers
                    .split('\x1f')
                    .map(coauthor_name)
                    .filter(|name| !name.is_empty())
                    .collect(),
                stat: parse_shortstat(stat),
            })
        })
        .collect()
}

/// `%D` with `--decorate=full`: `HEAD -> refs/heads/main, refs/remotes/origin/main,
/// tag: refs/tags/v1.0`. A bare `HEAD` is a detached HEAD; remote `HEAD`s and
/// other namespaces (stash, notes) are dropped.
fn parse_decorations(raw: &str) -> (bool, Vec<RefLabel>) {
    let mut head = false;
    let mut refs = Vec::new();
    for part in raw.split(", ").map(str::trim).filter(|p| !p.is_empty()) {
        let name = if let Some(branch) = part.strip_prefix("HEAD -> ") {
            head = true;
            branch
        } else if part == "HEAD" {
            head = true;
            continue;
        } else {
            part
        };
        let label = if let Some(tag) = name.strip_prefix("tag: refs/tags/") {
            RefLabel::Tag(tag.to_string())
        } else if let Some(branch) = name.strip_prefix("refs/heads/") {
            RefLabel::Branch(branch.to_string())
        } else if let Some(remote) = name.strip_prefix("refs/remotes/") {
            if remote.ends_with("/HEAD") {
                continue;
            }
            RefLabel::Remote(remote.to_string())
        } else {
            continue;
        };
        refs.push(label);
    }
    (head, refs)
}

/// `Claude <noreply@anthropic.com>` → `Claude`.
fn coauthor_name(value: &str) -> String {
    let value = value.trim();
    value
        .split_once('<')
        .map_or(value, |(name, _)| name)
        .trim()
        .to_string()
}

/// ` 6 files changed, 214 insertions(+), 38 deletions(-)`; either count may
/// be missing. `None` when there is no such line.
fn parse_shortstat(raw: &str) -> Option<Stat> {
    let line = raw.lines().find(|l| l.contains(" changed"))?;
    let mut stat = Stat::default();
    for part in line.split(',').map(str::trim) {
        let mut words = part.split_whitespace();
        let (Some(n), Some(word)) = (words.next(), words.next()) else {
            continue;
        };
        let n = n.parse().unwrap_or(0);
        if word.starts_with("file") {
            stat.files = n;
        } else if word.starts_with("insertion") {
            stat.insertions = n;
        } else if word.starts_with("deletion") {
            stat.deletions = n;
        }
    }
    Some(stat)
}

/// The branch a merge commit's subject names: GitHub's `Merge pull request
/// #12 from owner/feat/x`, git's `Merge branch 'x'` and `Merge remote-tracking
/// branch 'origin/x'`, each with an optional `into …`.
pub fn merged_branch(subject: &str) -> Option<String> {
    if let Some(rest) = subject.strip_prefix("Merge pull request #") {
        let (_, source) = rest.split_once(" from ")?;
        let source = source.split_whitespace().next()?;
        // `owner/branch`: the owner is the fork, not part of the branch.
        let branch = source.split_once('/').map_or(source, |(_, b)| b);
        return (!branch.is_empty()).then(|| branch.to_string());
    }
    let quoted = subject
        .strip_prefix("Merge branch '")
        .or_else(|| subject.strip_prefix("Merge remote-tracking branch '"))?;
    let (branch, _) = quoted.split_once('\'')?;
    (!branch.is_empty()).then(|| branch.to_string())
}

/// A line of the graph inside one row: from lane `from` to lane `to`, in the
/// color of the lane it belongs to. Equal lanes are a straight line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub color: usize,
}

/// The graph beside one commit. `top` runs from the row's top edge to the
/// node's height, `bottom` from the node's height to the bottom edge; a
/// lane that only passes by has an edge in both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRow {
    pub lane: usize,
    /// Index into the renderer's lane colors; lane 0's color is the tip's
    /// first-parent line.
    pub color: usize,
    pub top: Vec<Edge>,
    pub bottom: Vec<Edge>,
    /// On the tip's first-parent line: the branch's own history, not a
    /// branch merged into it.
    pub mainline: bool,
    /// The branch this commit's lane carries, when the history says (a merge
    /// subject or a branch ref at the lane's first commit). `None` on the
    /// mainline.
    pub branch: Option<String>,
    /// The first commit of a named lane: where its label goes.
    pub lane_tip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Graph {
    pub rows: Vec<GraphRow>,
    /// Lanes the widest row uses.
    pub width: usize,
}

struct Slot {
    /// The commit this lane is waiting for.
    hash: String,
    color: usize,
    branch: Option<String>,
    /// No commit has been drawn in this lane yet.
    fresh: bool,
}

/// Lay out `commits` (newest first, topological order) in lanes. Each lane
/// follows a line of first parents; a merge opens a lane for each other
/// parent, and lanes waiting for the same commit join at it. Freed lanes are
/// reused from the left, and lanes never shift sideways, so an edge between
/// different lanes is always a fork or a join.
pub fn layout(commits: &[Commit]) -> Graph {
    let mut lanes: Vec<Option<Slot>> = Vec::new();
    let mut next_color = 0;
    let mut width = 0;
    let mut mainline = commits.first().map(|c| c.hash.clone());
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let on_mainline = mainline.as_deref() == Some(commit.hash.as_str());
        if on_mainline {
            mainline = commit.parents.first().cloned();
        }
        let waiting: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_ref().is_some_and(|s| s.hash == commit.hash))
            .map(|(i, _)| i)
            .collect();

        let top: Vec<Edge> = lanes
            .iter()
            .enumerate()
            .filter_map(|(j, slot)| slot.as_ref().map(|s| (j, s.color)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(j, color)| Edge {
                from: j,
                to: if waiting.contains(&j) { usize::MAX } else { j },
                color,
            })
            .collect();

        // The node takes the leftmost lane waiting for it, or a free one: a
        // commit nothing loaded so far descends from (the tip, another branch).
        let (lane, color, branch, lane_tip) = match waiting.first() {
            Some(&lane) => {
                let slot = lanes[lane].as_ref().expect("waiting lane is occupied");
                (lane, slot.color, slot.branch.clone(), slot.fresh)
            }
            None => {
                let lane = free_lane(&mut lanes);
                let color = next_color;
                next_color += 1;
                let branch = (!on_mainline).then(|| first_branch(commit)).flatten();
                (lane, color, branch, true)
            }
        };
        let top = top
            .into_iter()
            .map(|e| Edge {
                to: if e.to == usize::MAX { lane } else { e.to },
                ..e
            })
            .collect();
        for &j in &waiting {
            lanes[j] = None;
        }

        // The first parent continues this lane; other parents join a lane
        // already waiting for them, or open one of their own.
        let mut bottom = Vec::new();
        let mut opened = Vec::new();
        if let Some(first) = commit.parents.first() {
            lanes[lane] = Some(Slot {
                hash: first.clone(),
                color,
                branch: branch.clone(),
                fresh: false,
            });
        }
        for parent in commit.parents.iter().skip(1) {
            let existing = lanes
                .iter()
                .position(|s| s.as_ref().is_some_and(|s| &s.hash == parent));
            match existing {
                Some(k) => {
                    let color = lanes[k].as_ref().map_or(0, |s| s.color);
                    bottom.push(Edge {
                        from: lane,
                        to: k,
                        color,
                    });
                }
                None => {
                    let k = free_lane(&mut lanes);
                    lanes[k] = Some(Slot {
                        hash: parent.clone(),
                        color: next_color,
                        branch: merged_branch(&commit.subject),
                        fresh: true,
                    });
                    next_color += 1;
                    opened.push(k);
                }
            }
        }
        for (j, slot) in lanes.iter().enumerate() {
            let Some(slot) = slot else { continue };
            let from = if j == lane || opened.contains(&j) {
                lane
            } else {
                j
            };
            bottom.push(Edge {
                from,
                to: j,
                color: slot.color,
            });
        }
        while lanes.last().is_some_and(Option::is_none) {
            lanes.pop();
        }
        width = width.max(lanes.len()).max(lane + 1);
        rows.push(GraphRow {
            lane,
            color,
            top,
            bottom,
            mainline: on_mainline,
            branch: if on_mainline { None } else { branch },
            lane_tip,
        });
    }
    Graph { rows, width }
}

/// The leftmost free lane, appending one if all are taken.
fn free_lane(lanes: &mut Vec<Option<Slot>>) -> usize {
    match lanes.iter().position(Option::is_none) {
        Some(i) => i,
        None => {
            lanes.push(None);
            lanes.len() - 1
        }
    }
}

fn first_branch(commit: &Commit) -> Option<String> {
    commit.refs.iter().find_map(|r| match r {
        RefLabel::Branch(name) => Some(name.clone()),
        _ => None,
    })
}

/// Up to two initials for an avatar: `Maya Chen` → `MC`, `sofia` → `S`.
pub fn initials(name: &str) -> String {
    let mut words = name.split_whitespace();
    let first = words.next().and_then(|w| w.chars().next());
    let last = words.last().and_then(|w| w.chars().next());
    first
        .into_iter()
        .chain(last)
        .flat_map(char::to_uppercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(hash: &str, parents: &[&str], subject: &str) -> Commit {
        Commit {
            hash: hash.into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author: "A".into(),
            time: 0,
            head: false,
            refs: Vec::new(),
            subject: subject.into(),
            coauthors: Vec::new(),
            stat: None,
        }
    }

    fn edge(from: usize, to: usize, color: usize) -> Edge {
        Edge { from, to, color }
    }

    #[test]
    fn parses_records_with_refs_trailers_and_stats() {
        let raw = "\x1eaaaaaaa1\0bbbbbbb2\0Maya Chen\x001700000000\0HEAD -> refs/heads/main, refs/remotes/origin/main, refs/remotes/origin/HEAD, tag: refs/tags/v2.4, refs/stash\0Released v2.4\0Claude <noreply@anthropic.com>\x1fLeo Martins <leo@x.dev>\n\n 6 files changed, 214 insertions(+), 38 deletions(-)\n\
                   \x1ebbbbbbb2\0ccccccc3 ddddddd4\0Jonas Weber\x001699990000\0\0Merge branch 'feat'\0\n\
                   \x1eccccccc3\0\0Sofia\x001699980000\0HEAD\0Initial\0\n\n 1 file changed, 3 deletions(-)\n";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 3);

        let first = &commits[0];
        assert_eq!(first.short(), "aaaaaaa");
        assert_eq!(first.parents, vec!["bbbbbbb2"]);
        assert_eq!(first.author, "Maya Chen");
        assert_eq!(first.time, 1_700_000_000);
        assert!(first.head);
        assert_eq!(
            first.refs,
            vec![
                RefLabel::Branch("main".into()),
                RefLabel::Remote("origin/main".into()),
                RefLabel::Tag("v2.4".into()),
            ]
        );
        assert_eq!(first.subject, "Released v2.4");
        assert_eq!(first.coauthors, vec!["Claude", "Leo Martins"]);
        assert_eq!(
            first.stat,
            Some(Stat {
                files: 6,
                insertions: 214,
                deletions: 38
            })
        );

        // A merge: two parents, no stat, no trailers.
        assert!(commits[1].is_merge());
        assert_eq!(commits[1].stat, None);
        assert!(commits[1].coauthors.is_empty());

        // A detached HEAD and a stat with only deletions.
        assert!(commits[2].head && commits[2].refs.is_empty());
        assert!(commits[2].parents.is_empty());
        assert_eq!(
            commits[2].stat,
            Some(Stat {
                files: 1,
                insertions: 0,
                deletions: 3
            })
        );
    }

    #[test]
    fn parse_log_tolerates_empty_and_truncated_output() {
        assert!(parse_log("").is_empty());
        assert!(parse_log("\x1e\n").is_empty());
        // Fewer fields than the format prints: skipped, not a panic.
        assert!(parse_log("\x1eabc\0def").is_empty());
    }

    #[test]
    fn merged_branch_reads_common_merge_subjects() {
        assert_eq!(
            merged_branch("Merge pull request #40 from ismaelosuna7824/feat/quick-look"),
            Some("feat/quick-look".into())
        );
        assert_eq!(
            merged_branch("Merge branch 'hybrid-search' into main"),
            Some("hybrid-search".into())
        );
        assert_eq!(
            merged_branch("Merge remote-tracking branch 'origin/main'"),
            Some("origin/main".into())
        );
        assert_eq!(merged_branch("Fix the merge of two lists"), None);
        assert_eq!(merged_branch("Merge branch ''"), None);
    }

    #[test]
    fn a_straight_history_stays_in_one_lane() {
        let graph = layout(&[
            commit("c", &["b"], ""),
            commit("b", &["a"], ""),
            commit("a", &[], ""),
        ]);
        assert_eq!(graph.width, 1);
        let r = &graph.rows;
        assert!(
            r.iter()
                .all(|row| row.lane == 0 && row.color == 0 && row.mainline)
        );
        assert!(r[0].top.is_empty());
        assert_eq!(r[0].bottom, vec![edge(0, 0, 0)]);
        assert_eq!(r[1].top, vec![edge(0, 0, 0)]);
        // The root ends the line.
        assert_eq!(r[2].top, vec![edge(0, 0, 0)]);
        assert!(r[2].bottom.is_empty());
    }

    #[test]
    fn a_merged_branch_forks_out_and_joins_at_its_base() {
        // m merges x (branch) into p; x and p both descend from base.
        let graph = layout(&[
            commit("m", &["base", "x2"], "Merge branch 'streaming'"),
            commit("x2", &["x1"], ""),
            commit("x1", &["base"], ""),
            commit("base", &[], ""),
        ]);
        assert_eq!(graph.width, 2);
        let r = &graph.rows;

        // The merge: mainline continues down, the branch curves out to lane 1.
        assert_eq!(r[0].bottom, vec![edge(0, 0, 0), edge(0, 1, 1)]);
        // Branch commits: lane 1, own color, not mainline, named by the merge.
        assert_eq!((r[1].lane, r[1].color, r[1].mainline), (1, 1, false));
        assert_eq!(r[1].branch.as_deref(), Some("streaming"));
        assert!(r[1].lane_tip);
        assert!(!r[2].lane_tip);
        assert_eq!(r[1].top, vec![edge(0, 0, 0), edge(1, 1, 1)]);
        // The base: both lanes wait for it and join into lane 0.
        assert_eq!(r[3].top, vec![edge(0, 0, 0), edge(1, 0, 1)]);
        assert!(r[3].mainline && r[3].branch.is_none());
    }

    #[test]
    fn a_second_parent_already_waited_for_joins_that_lane() {
        // Two merges of the same branch commit: the second reuses its lane.
        let graph = layout(&[
            commit("m2", &["m1", "x"], "Merge branch 'x'"),
            commit("m1", &["base", "x"], "Merge branch 'x'"),
            commit("x", &["base"], ""),
            commit("base", &[], ""),
        ]);
        assert_eq!(graph.width, 2);
        // m1 opens no lane for x: it curves into lane 1, which passes by.
        assert_eq!(
            graph.rows[1].bottom,
            vec![edge(0, 1, 1), edge(0, 0, 0), edge(1, 1, 1)]
        );
        assert_eq!(graph.rows[1].top, vec![edge(0, 0, 0), edge(1, 1, 1)]);
    }

    #[test]
    fn freed_lanes_are_reused_from_the_left() {
        let graph = layout(&[
            commit("m2", &["m1", "y"], "Merge branch 'y'"),
            commit("y", &["m1"], ""),
            commit("m1", &["base", "x"], "Merge branch 'x'"),
            commit("x", &["base"], ""),
            commit("base", &[], ""),
        ]);
        // y's lane closed at m1, so x takes lane 1 again.
        assert_eq!(graph.width, 2);
        assert_eq!(graph.rows[3].lane, 1);
        assert_eq!(graph.rows[3].branch.as_deref(), Some("x"));
        // Each opened lane gets its own color.
        assert_ne!(graph.rows[1].color, graph.rows[3].color);
    }

    #[test]
    fn a_commit_no_loaded_child_reaches_opens_a_lane_named_by_its_branch() {
        let mut tip = commit("t", &["base"], "");
        tip.refs = vec![RefLabel::Branch("main".into())];
        let mut other = commit("o", &["base"], "");
        other.refs = vec![RefLabel::Branch("feature".into())];
        let graph = layout(&[tip, other, commit("base", &[], "")]);
        let r = &graph.rows;
        // The tip's own branch is the mainline, unnamed.
        assert_eq!(r[0].branch, None);
        assert_eq!((r[1].lane, r[1].branch.as_deref()), (1, Some("feature")));
        assert!(r[1].lane_tip && !r[1].mainline);
        assert_eq!(r[2].top, vec![edge(0, 0, 0), edge(1, 0, 1)]);
    }

    #[test]
    fn initials_take_the_first_and_last_word() {
        assert_eq!(initials("Maya Chen"), "MC");
        assert_eq!(initials("Jean de la Fontaine"), "JF");
        assert_eq!(initials("sofia"), "S");
        assert_eq!(initials("  "), "");
    }
}
