use std::path::PathBuf;

use super::store::{load_from, save_to};
use super::{SavedPane, SavedState, SavedTab, SavedWorkspace, VERSION};
use crate::shared::pane::{PaneKind, PaneTree, SplitAxis};

fn sample() -> SavedState {
    let agent = SavedPane {
        kind: PaneKind::Agent,
        cwd: PathBuf::from("/tmp/project"),
        path: None,
        session: Some(PathBuf::from("/tmp/sessions/a.jsonl")),
        herdr_terminal: None,
    };
    let terminal = SavedPane {
        kind: PaneKind::Terminal,
        cwd: PathBuf::from("/tmp/project"),
        path: None,
        session: None,
        herdr_terminal: Some("remote/t7".to_string()),
    };
    let file = SavedPane {
        kind: PaneKind::File,
        cwd: PathBuf::from("/tmp/project"),
        path: Some(PathBuf::from("/tmp/project/README.md")),
        session: None,
        herdr_terminal: None,
    };
    SavedState {
        version: VERSION,
        current: 1,
        workspaces: vec![
            SavedWorkspace {
                name: "Default".into(),
                root: PathBuf::from("/tmp"),
                tabs: vec![SavedTab {
                    layout: PaneTree::Leaf(agent.clone()),
                    focused: 0,
                }],
                active_tab: 0,
                last_active: 10,
            },
            SavedWorkspace {
                name: "project".into(),
                root: PathBuf::from("/tmp/project"),
                tabs: vec![SavedTab {
                    layout: PaneTree::Split {
                        axis: SplitAxis::Horizontal,
                        ratio: 0.4,
                        first: Box::new(PaneTree::Leaf(agent)),
                        second: Box::new(PaneTree::Split {
                            axis: SplitAxis::Vertical,
                            ratio: 0.5,
                            first: Box::new(PaneTree::Leaf(terminal)),
                            second: Box::new(PaneTree::Leaf(file)),
                        }),
                    },
                    focused: 2,
                }],
                active_tab: 0,
                last_active: 20,
            },
        ],
        sidebar: false,
    }
}

#[test]
fn json_round_trip() {
    let state = sample();
    let json = serde_json::to_string(&state).unwrap();
    let back: SavedState = serde_json::from_str(&json).unwrap();
    assert_eq!(back, state);
    // Optional fields stay out of the file and default back in.
    assert!(!json.contains("\"path\":null"));
    assert!(json.contains("\"herdr_terminal\":\"remote/t7\""));
    let minimal: SavedState =
        serde_json::from_str(r#"{"workspaces":[{"name":"x","root":"/x"}]}"#).unwrap();
    assert_eq!(minimal.workspaces[0].tabs.len(), 0);
    assert_eq!(minimal.current, 0);
    // A file from before the sidebar was saved keeps it showing.
    assert!(minimal.sidebar);
    assert_eq!(SavedState::default().sidebar, minimal.sidebar);

    // Files saved by older versions had no Herdr identity.
    let legacy: SavedPane = serde_json::from_str(
        r#"{"kind":"terminal","cwd":"/tmp/project","session":null}"#,
    )
    .unwrap();
    assert_eq!(legacy.herdr_terminal, None);
    let restored: SavedPane = serde_json::from_str(
        r#"{"kind":"terminal","cwd":"/tmp/project","herdr_terminal":"javipi/t7"}"#,
    )
    .unwrap();
    assert_eq!(restored.herdr_terminal.as_deref(), Some("javipi/t7"));
}

#[test]
fn agent_herdr_address_round_trip_and_legacy_layout_are_compatible() {
    let saved = SavedPane {
        kind: PaneKind::Agent,
        cwd: PathBuf::from("D:/Programacion/my-project"),
        path: None,
        session: Some(PathBuf::from("/home/dietpi/.gentle-shell/agent/sessions/chat.jsonl")),
        herdr_terminal: Some("javipi/t42".into()),
    };
    let json = serde_json::to_string(&saved).unwrap();
    assert!(json.contains("\\"herdr_terminal\\":\\"javipi/t42\\""));
    assert_eq!(serde_json::from_str::<SavedPane>(&json).unwrap(), saved);
    let old: SavedPane = serde_json::from_str(
        r#"{"kind":"agent","cwd":"D:/Programacion/my-project"}"#,
    ).unwrap();
    assert!(old.herdr_terminal.is_none());
}

#[test]
fn file_round_trip() {
    let dir = std::env::temp_dir().join(format!("gentle-ws-test-{}", std::process::id()));
    let path = dir.join("nested").join("workspaces.json");
    let state = sample();
    save_to(&path, &state).unwrap();
    assert_eq!(load_from(&path), Some(state));
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(load_from(&path), None);
    let _ = std::fs::remove_dir_all(dir);
}
