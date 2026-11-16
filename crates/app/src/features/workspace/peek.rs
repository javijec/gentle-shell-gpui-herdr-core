//! Quick look: a single click on a file in the sidebar shows it in a modal
//! over the panes. The sidebar stays usable, so another click swaps the file
//! and a double click (or "Open in panel") moves the same view, unsaved
//! edits included, into the layout like [`Workspace::open_file`].

use super::*;

/// Native and remote quick looks carry the same editor, but must retain
/// their different ownership metadata when pinned into a normal pane.
pub(super) enum PeekSource {
    Local(PathBuf),
    Herdr {
        session: Entity<SessionView>,
        machine: String,
        root: String,
        path: String,
        subscription: Subscription,
    },
}

/// The file shown over the panes.
pub(super) struct Peek {
    pub(super) item: PaneItem,
    pub(super) source: PeekSource,
}

impl Workspace {
    /// Show `path` in the quick look, replacing the file it shows.
    pub fn peek_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.peek.as_ref().is_some_and(|peek| {
            matches!(&peek.source, PeekSource::Local(current) if *current == path)
        }) {
            self.focus_active(window, cx);
            return;
        }
        let Some(factory) = PaneFactories::get(cx).file else {
            self.open_file(path, window, cx);
            return;
        };
        self.leave_peek(window, cx, move |this, window, cx| {
            let item = factory(path.clone(), window, cx);
            this.peek = Some(Peek {
                item,
                source: PeekSource::Local(path.clone()),
            });
            this.focus_active(window, cx);
            cx.notify();
        });
    }

    /// Hide the quick look, asking first when it holds unsaved edits.
    pub(super) fn close_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_peek(window, cx, |this, window, cx| {
            this.peek = None;
            this.focus_active(window, cx);
            cx.notify();
        });
    }

    /// Move the quick look's view into the layout, placed like a file opened
    /// from the sidebar.
    pub(super) fn pin_peek(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Peek { item, source }) = self.peek.take() else {
            return;
        };
        let id = match source {
            PeekSource::Local(path) => self.file_pane(item, path, cx),
            PeekSource::Herdr {
                session,
                machine,
                root,
                subscription,
                ..
            } => {
                let id = self.alloc_id();
                let mut pane = Pane {
                    item,
                    session: None,
                    linked_session: Some(session),
                    cwd: self.space().root.clone(),
                    path: None,
                    session_file: None,
                    herdr_address: None,
                    agent_target: Some(pi_rpc::AgentTarget::Remote {
                        machine,
                        cwd: root,
                    }),
                    title: SharedString::default(),
                    running: false,
                    needs_you: false,
                    _subscriptions: vec![subscription],
                };
                pane.sync_chrome(cx);
                self.panes.insert(id, pane);
                id
            }
        };
        self.place_preview(id, window, cx);
        cx.notify();
    }

    /// Run `then` once the shown file may be dropped: right away when it has
    /// no unsaved edits, else after "Close without saving?".
    pub(super) fn leave_peek(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let unsaved = self
            .peek
            .as_ref()
            .filter(|peek| peek.item.unsaved.as_ref().is_some_and(|probe| probe(cx)))
            .map(|peek| (peek.item.title)(cx));
        match unsaved {
            Some(title) => self.confirm_close(vec![title], Vec::new(), window, cx, then),
            None => then(self, window, cx),
        }
    }

    /// The modal over the panes' area; a click on the veil closes it.
    pub(super) fn render_peek(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let peek = self.peek.as_ref()?;
        let footer = h_flex()
            .flex_none()
            .px_4()
            .py_3()
            .gap_2()
            .border_t_1()
            .border_color(ui::border())
            .text_xs()
            .text_color(ui::muted())
            .child(
                ui::key_button("peek-esc", "esc")
                    .on_click(cx.listener(|this, _, window, cx| this.close_peek(window, cx))),
            )
            .child("close")
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(ui::faint())
                    .child("Double-click a file to open it in a panel"),
            )
            .child(
                Button::new("peek-pin")
                    .primary()
                    .small()
                    .icon(Icon::new(IconName::Columns2))
                    .label("Open in panel")
                    .on_click(cx.listener(|this, _, window, cx| this.pin_peek(window, cx))),
            );
        let panel = v_flex()
            .id("peek")
            .size_full()
            .max_w(px(1100.))
            .rounded_2xl()
            .border_1()
            .border_color(ui::border_strong())
            .bg(ui::screen_bg())
            .shadow_2xl()
            .overflow_hidden()
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(peek.item.view.clone()),
            )
            .child(footer);
        Some(
            div()
                .id("peek-backdrop")
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p(px(24.))
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| this.close_peek(window, cx)))
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded_xl()
                        .overflow_hidden()
                        .child(ui::scrim("peek-scrim")),
                )
                .child(ui::pop_in(
                    "peek-in",
                    div().flex().size_full().justify_center().child(panel),
                ))
                .into_any_element(),
        )
    }
}
