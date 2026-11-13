//! Closing panes safely: an agent that is working or waiting on you asks
//! first, and every pane or tab closed can be reopened where it was (most
//! recent first). Agents come back with their conversation, terminals in
//! their folder, files at their path.

use super::*;

/// How many closes can be undone.
const KEEP: usize = 20;

/// Closing a Herdr pane intentionally terminates its terminal. Keep the
/// conversation and machine/cwd for Undo, but never reattach the dead address.
/// In native mode an address is dormant metadata and must remain preserved.
fn snapshot_for_reopen(mut saved: SavedPane, herdr_enabled: bool) -> SavedPane {
    if herdr_enabled {
        saved.herdr_terminal = None;
    }
    saved
}

/// A pane in a closed tab's layout: one still open, or one that closed.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Slot {
    Open(PaneId),
    Closed(SavedPane),
}

/// One close, to undo.
pub(super) struct Closed {
    /// The workspace it happened in, by index.
    space: usize,
    /// The tab, by id, and where it sat among the tabs.
    tab: usize,
    index: usize,
    /// The tab's layout just before the close.
    layout: PaneTree<Slot>,
    /// What the notice names.
    title: SharedString,
}

impl Workspace {
    /// Remember that `closing` (all of tab `index`'s panes, or some) are
    /// about to close. Panes that can't be described again aren't kept.
    pub(super) fn remember_close(&mut self, index: usize, closing: &[PaneId], cx: &App) {
        let Some(tab) = self.space().tabs.get(index) else {
            return;
        };
        let herdr_enabled = crate::features::terminal::herdr_enabled();
        let layout = tab.tree.filter_map(&mut |id: &PaneId| {
            if closing.contains(id) {
                self.saved_pane(*id, cx)
                    .map(|saved| Slot::Closed(snapshot_for_reopen(saved, herdr_enabled)))
            } else {
                Some(Slot::Open(*id))
            }
        });
        let Some(layout) = layout else { return };
        if !layout.leaves().iter().any(|s| matches!(s, Slot::Closed(_))) {
            return;
        }
        let title = match closing {
            [one] => self.pane_title(*one),
            _ => SharedString::from(format!("{} panes", closing.len())),
        };
        let closed = Closed {
            space: self.current,
            tab: tab.id,
            index,
            layout,
            title,
        };
        if self.closed.len() == KEEP {
            self.closed.remove(0);
        }
        self.closed.push(closed);
    }

    /// The window just closed something: say so, with a way back.
    pub(super) fn announce_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.closed.last() else {
            return;
        };
        let weak = cx.entity().downgrade();
        let note = Notification::info(format!(
            "Closed \u{201c}{}\u{201d}. Click to reopen ({}).",
            closed.title.trim_end_matches(" •"),
            REOPEN_KEY
        ))
        .id::<Closed>()
        .on_click(move |_, window, cx| {
            let _ = weak.update(cx, |this, cx| this.reopen_closed(window, cx));
        });
        window.push_notification(note, cx);
    }

    /// Bring back the last thing closed, where it was.
    pub(super) fn reopen_closed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(closed) = self.closed.pop() else {
            return;
        };
        // Its notice (if still up) would now point at the close before it.
        window.remove_notification::<Closed>(cx);
        if closed.space < self.spaces.len() && closed.space != self.current {
            self.switch_workspace(closed.space, window, cx);
        }
        let mut reopened = Vec::new();
        let mut build = |this: &mut Self,
                         slot: &Slot,
                         window: &mut Window,
                         cx: &mut Context<Self>| {
            match slot {
                Slot::Open(id) => this.panes.contains_key(id).then_some(*id),
                Slot::Closed(saved) => {
                    let id = this.restore_pane(saved, window, cx);
                    reopened.extend(id);
                    id
                }
            }
        };
        let still_open = closed.layout.filter_map(&mut |slot| match slot {
            Slot::Open(id) => Some(*id),
            Slot::Closed(_) => None,
        });
        let index = self.space().tabs.iter().position(|t| t.id == closed.tab);
        match index {
            // The tab is as the close left it: rebuild its layout exactly.
            Some(index) if still_open.as_ref() == Some(&self.space().tabs[index].tree) => {
                let tree = closed
                    .layout
                    .filter_map(&mut |slot| build(self, slot, window, cx));
                if let Some(tree) = tree {
                    self.space_mut().tabs[index].tree = tree;
                }
                self.show_reopened(index, &reopened, window, cx);
            }
            // The tab changed since: the panes come back beside its focus.
            Some(index) => {
                for slot in closed.layout.leaves() {
                    if let Slot::Closed(_) = slot
                        && let Some(id) = build(self, &slot, window, cx)
                    {
                        let tab = &mut self.space_mut().tabs[index];
                        let focused = tab.focused;
                        tab.tree.split(&focused, id, SplitAxis::Horizontal);
                    }
                }
                self.show_reopened(index, &reopened, window, cx);
            }
            // The whole tab closed: it comes back where it sat.
            None => {
                let Some(tree) = closed
                    .layout
                    .filter_map(&mut |slot| build(self, slot, window, cx))
                else {
                    return;
                };
                let id = self.alloc_id();
                let focused = tree.leaves()[0];
                let index = closed.index.min(self.space().tabs.len());
                self.space_mut()
                    .tabs
                    .insert(index, Tab { id, tree, focused });
                self.show_reopened(index, &reopened, window, cx);
            }
        }
        self.schedule_save(cx);
    }

    fn show_reopened(
        &mut self,
        index: usize,
        reopened: &[PaneId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let (Some(first), Some(tab)) = (reopened.first(), self.space_mut().tabs.get_mut(index)) {
            tab.focused = *first;
        }
        self.activate(index, window, cx);
    }

    /// Workspace `index` is gone: its closes can't come back, and the ones
    /// after it moved down.
    pub(super) fn forget_closed_in(&mut self, index: usize) {
        self.closed.retain(|c| c.space != index);
        for closed in &mut self.closed {
            if closed.space > index {
                closed.space -= 1;
            }
        }
    }
}

/// The shortcut that reopens, as the notice and the palette show it.
pub(super) const REOPEN_KEY: &str = if cfg!(target_os = "macos") {
    "⌥⌘T"
} else {
    "Ctrl+Alt+Shift+T"
};

#[cfg(test)]
mod tests {
    // Keep the built-in #[test] attribute: super::* pulls GPUI's macro
    // imports into this module and can recursively expand #[test].
    use super::snapshot_for_reopen;
    use crate::features::workspaces::SavedPane;
    use crate::shared::pane::PaneKind;
    use std::path::PathBuf;

    fn remote_pane() -> SavedPane {
        SavedPane {
            kind: PaneKind::Agent,
            cwd: PathBuf::from("C:/projects/gentle-shell"),
            path: None,
            session: Some(PathBuf::from("C:/sessions/remote.jsonl")),
            herdr_terminal: Some("javipi/t33".into()),
            agent_target: Some(pi_rpc::AgentTarget::Remote {
                machine: "javipi".into(),
                cwd: "/srv/workspaces/JaviPi".into(),
            }),
        }
    }

    #[test]
    fn undo_after_explicit_herdr_close_starts_fresh_not_dead_terminal() {
        let original = remote_pane();
        let restored = snapshot_for_reopen(original.clone(), true);
        assert_eq!(restored.herdr_terminal, None);
        assert_eq!(restored.session, original.session);
        assert_eq!(restored.agent_target, original.agent_target);
        assert_eq!(restored.cwd, original.cwd);
    }

    #[test]
    fn native_mode_keeps_dormant_herdr_address() {
        let original = remote_pane();
        assert_eq!(snapshot_for_reopen(original.clone(), false), original);
    }
}
