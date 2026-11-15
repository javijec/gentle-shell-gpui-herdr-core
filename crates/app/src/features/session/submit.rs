//! What the composer sends: `!` shell commands, built-ins and natively shown
//! commands run here, prompts with their image attachments and long pastes.

use std::path::PathBuf;

use anyhow::Result;
use gpui_kit::component::input::{InlineToken, InputContent, Position};
use gpui_kit::*;
use serde_json::json;

use super::attachments::Kind;
use super::{SessionEvent, SessionView, attachments, builtins};

impl SessionView {
    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.composer.read(cx).value().trim().to_string();
        // Told apart as typed: a paste that starts with `!` or `/` is
        // content, not a command.
        let shell = typed.starts_with('!');
        let text = attachments::expand(&typed, &self.pasted);
        let images = !shell && !self.attachments.is_empty();
        if (text.is_empty() && !images) || self.process().is_none() {
            return;
        }
        if shell && !self.attachments.is_empty() {
            // Clearing the composer would strand the images with no tokens left.
            let message =
                "Shell commands cannot carry images. Remove the attachments or drop the leading !.";
            self.warn(message.into(), window, cx);
            return;
        }
        if images && let Some(model) = self.models.current.as_ref().filter(|m| !m.images) {
            let message = format!(
                "{} does not accept images. Switch to a model with image input, or remove the attachments.",
                model.name
            );
            self.warn(message, window, cx);
            return;
        }
        self.composer
            .update(cx, |state, cx| state.set_value("", window, cx));
        let pasted = !std::mem::take(&mut self.pasted).is_empty();
        self.list.scroll_to_end();

        if let Some(command) = text.strip_prefix('!').filter(|_| shell) {
            self.run_shell(command, cx);
            cx.notify();
            return;
        }

        if self.attachments.is_empty() && !pasted && self.intercept(&text, cx) {
            cx.notify();
            return;
        }

        let images = std::mem::take(&mut self.attachments);
        self.send_prompt_with(text, images, cx);
        cx.notify();
    }

    /// Text from outside the composer (`-p`, a later launch's prompt): runs
    /// the way typing it would, `!` shell commands included.
    pub(super) fn run_text(&mut self, text: String, cx: &mut Context<Self>) {
        match text.trim().strip_prefix('!') {
            Some(command) => {
                self.run_shell(command, cx);
                cx.notify();
            }
            None => self.run_prompt(text, cx),
        }
    }

    /// Pi's TUI convention: `!cmd` runs and shares output, `!!cmd` keeps it
    /// out of context. `command` is what follows the first `!`.
    fn run_shell(&mut self, command: &str, cx: &mut Context<Self>) {
        let (command, exclude) = match command.strip_prefix('!') {
            Some(rest) => (rest.trim().to_string(), true),
            None => (command.trim().to_string(), false),
        };
        if command.is_empty() {
            return;
        }
        let index = self.transcript.start_shell(command.clone());
        self.active_shell = Some(index);
        self.request(
            json!({"type": "bash", "command": command, "excludeFromContext": exclude}),
            cx,
            move |this, result, _| {
                this.active_shell = None;
                match result {
                    Ok(data) => this.transcript.finish_shell(
                        index,
                        data["exitCode"].as_i64(),
                        data["output"].as_str().map(str::to_string),
                    ),
                    Err(error) => {
                        this.transcript
                            .finish_shell(index, None, Some(format!("{error:#}")))
                    }
                }
            },
        );
    }

    /// Run a slash command the way typing it would: built-ins and natively
    /// shown commands run here, the rest goes to Pi.
    pub fn run_prompt(&mut self, text: String, cx: &mut Context<Self>) {
        if !self.intercept(&text, cx) {
            self.send_prompt(text, cx);
        }
        cx.notify();
    }

    /// Run `text` here instead of sending it when it is a Pi built-in or a
    /// Gentle Shell command the app shows natively; false: send it.
    fn intercept(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        match builtins::classify(text) {
            builtins::Submit::Builtin(builtin) => {
                self.run_builtin(builtin, cx);
                true
            }
            builtins::Submit::Open(screen) => {
                cx.emit(SessionEvent::OpenScreen(screen));
                true
            }
            builtins::Submit::Prompt => false,
        }
    }

    /// `/web <url>`: show a page in the in-app browser instead of sending the
    /// command to Gentle Shell.
    fn open_web(&mut self, url: String, cx: &mut Context<Self>) {
        let url = url.trim();
        if url.is_empty() {
            self.note("Usage: /web <url>".to_string(), cx);
        } else if !(url.starts_with("http://") || url.starts_with("https://")) {
            self.note(
                "Only http:// and https:// URLs can open here.".to_string(),
                cx,
            );
        } else {
            cx.emit(SessionEvent::OpenWeb(url.to_string()));
        }
    }

    fn run_builtin(&mut self, builtin: builtins::Builtin, cx: &mut Context<Self>) {
        match builtin {
            builtins::Builtin::Compact(instructions) => self.compact(instructions, cx),
            builtins::Builtin::Name(name) if name.trim().is_empty() => {
                let text = match &self.session_name {
                    Some(name) => {
                        format!("This session is named “{name}”. Rename it with /name <title>.")
                    }
                    None => "This session has no name yet. Name it with /name <title>.".into(),
                };
                self.note(text, cx);
            }
            builtins::Builtin::Name(name) => self.rename(name, cx),
            builtins::Builtin::Copy => self.copy_last_answer(cx),
            builtins::Builtin::Export(path) => self.export_html(path, cx),
            builtins::Builtin::Reload => self.reload(cx),
            builtins::Builtin::Web(url) => self.open_web(url, cx),
        }
    }

    /// `/name ` in the composer, cursor at its end.
    pub(super) fn insert_command(
        &mut self,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = format!("/{name} ");
        let column = text.chars().map(char::len_utf16).sum::<usize>() as u32;
        self.composer.update(cx, |state, cx| {
            state.set_value(text, window, cx);
            state.set_cursor_position(Position::new(0, column), window, cx);
            state.focus(window, cx);
        });
        // The whole text was replaced: no paste's token is left in it.
        self.pasted.clear();
    }

    /// Attach validated images (inserting their tokens at the cursor) and warn
    /// about the rest.
    fn attach(
        &mut self,
        loaded: Vec<Result<attachments::Attachment, attachments::Rejection>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let first = self.attachments.len() + 1;
        let mut rejected = Vec::new();
        for result in loaded {
            match result {
                Ok(attachment) => self.attachments.push(attachment),
                Err(rejection) => rejected.push(rejection.to_string()),
            }
        }
        let added = first..self.attachments.len() + 1;
        if !added.is_empty() {
            self.composer.update(cx, |state, cx| {
                let value = state.value();
                let mut before = value[..state.cursor().min(value.len())].chars().next_back();
                let mut tokens = String::new();
                for n in added {
                    tokens.push_str(&attachments::insertion(before, n));
                    before = Some(' ');
                }
                state.insert(tokens, window, cx);
                state.focus(window, cx);
            });
        }
        for message in rejected {
            self.warn(message, window, cx);
        }
        cx.notify();
    }

    /// Read dropped or picked files off the UI thread, then attach them.
    pub(super) fn attach_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        self.attach_off_thread(
            move || {
                paths
                    .iter()
                    .map(|path| attachments::load_path(path))
                    .collect()
            },
            window,
            cx,
        );
    }

    /// Run `load` on the background executor, then attach what it read.
    fn attach_off_thread(
        &mut self,
        load: impl FnOnce() -> Vec<Result<attachments::Attachment, attachments::Rejection>>
        + Send
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let loading = cx.background_executor().spawn(async move { load() });
        cx.spawn_in(window, async move |this, cx| {
            let loaded = loading.await;
            let _ = this.update_in(cx, |this, window, cx| this.attach(loaded, window, cx));
        })
        .detach();
    }

    /// The composer's paste hook: clipboard images and copied image files
    /// become attachments, and so does long text ([`Self::paste_text`]);
    /// anything else pastes as text.
    pub(super) fn paste_clipboard(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut images = Vec::new();
        let mut paths = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => images.push(image.bytes.clone()),
                ClipboardEntry::ExternalPaths(external) => {
                    paths.extend(external.paths().iter().cloned())
                }
                _ => {}
            }
        }
        // Copied files that are not images paste as their names.
        if !paths.iter().any(|p| attachments::is_image_file(p)) {
            paths.clear();
        }
        if images.is_empty() && paths.is_empty() {
            return item
                .text()
                .is_some_and(|text| self.paste_text(text, window, cx));
        }
        if !images.is_empty() {
            // Off the UI thread: a BMP or TIFF screenshot is re-encoded first.
            self.attach_off_thread(
                move || {
                    images
                        .into_iter()
                        .map(|bytes| attachments::Attachment::from_bytes("The pasted image", bytes))
                        .collect()
                },
                window,
                cx,
            );
        }
        self.attach_paths(paths, window, cx);
        true
    }

    /// Keep a long paste ([`attachments::is_long_paste`]) as an attachment,
    /// its `[Pasted text #N]` token at the cursor shown as a chip (a file
    /// icon and `#N`, see the composer's token renderer);
    /// false: it's short, paste it as text.
    fn paste_text(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !attachments::is_long_paste(&text) {
            return false;
        }
        self.pasted.push(attachments::PastedText::new(text));
        let n = self.pasted.len();
        self.composer.update(cx, |state, cx| {
            let value = state.value();
            let before = value[..state.cursor().min(value.len())].chars().next_back();
            if attachments::needs_lead(before) {
                state.replace(" ", window, cx);
            }
            if state
                .replace_with_token(pasted_chip(n), window, cx)
                .is_err()
            {
                // Mid-composition the chip can't go in; the token still works.
                state.replace(attachments::token(Kind::Pasted, n), window, cx);
            }
            state.insert(" ", window, cx);
            state.focus(window, cx);
        });
        cx.notify();
        true
    }

    /// The 📎 button: pick image files.
    pub(super) fn pick_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.attach_paths(paths, window, cx));
        })
        .detach();
    }

    /// Drop attachment `index` (0-based) and its token; later tokens shift down.
    pub(super) fn remove_attachment(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.attachments.len() {
            return;
        }
        self.attachments.remove(index);
        let text =
            attachments::remove_token(&self.composer.read(cx).value(), Kind::Image, index + 1);
        let end = text.len();
        if self.pasted.is_empty() {
            let position = position_at(&text, end);
            self.composer.update(cx, |state, cx| {
                state.replace_all(text, window, cx);
                state.set_cursor_position(position, window, cx);
            });
        } else {
            // Only setting the value keeps the pastes' chips.
            self.set_composer_text(text, end, window, cx);
        }
        cx.notify();
    }

    /// Drop pasted text `index` (0-based) and its token; later ones shift down.
    pub(super) fn remove_pasted(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.pasted.len() {
            return;
        }
        self.pasted.remove(index);
        let text =
            attachments::remove_token(&self.composer.read(cx).value(), Kind::Pasted, index + 1);
        let end = text.len();
        self.set_composer_text(text, end, window, cx);
        cx.notify();
    }

    /// After an edit: drop the pastes whose token left the text (deleted,
    /// cut, typed over) and renumber the rest, as removing their cards would.
    pub(super) fn sync_pasted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pasted.is_empty() {
            return;
        }
        let (text, cursor) = {
            let state = self.composer.read(cx);
            (state.value(), state.cursor().min(state.value().len()))
        };
        let kept = attachments::present(&text, Kind::Pasted, self.pasted.len());
        if kept.len() == self.pasted.len() {
            return;
        }
        let mut number = 0;
        self.pasted.retain(|_| {
            number += 1;
            kept.binary_search(&number).is_ok()
        });
        let cursor = attachments::renumber(&text[..cursor], Kind::Pasted, &kept).len();
        let text = attachments::renumber(&text, Kind::Pasted, &kept);
        self.set_composer_text(text, cursor, window, cx);
        cx.notify();
    }

    /// Put `text` in the composer with the pastes' tokens as chips and the
    /// cursor at byte `cursor`. Setting the value clears the undo history,
    /// so undo can't bring back the token of a paste that is gone.
    fn set_composer_text(
        &mut self,
        text: String,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = position_at(&text, cursor);
        let chips = attachments::find_tokens(&text, Kind::Pasted);
        let mut content = InputContent::new(text);
        for (range, n) in chips {
            if (1..=self.pasted.len()).contains(&n) {
                content = content
                    .clone()
                    .with_token(range, pasted_chip(n))
                    .unwrap_or(content);
            }
        }
        self.composer.update(cx, |state, cx| {
            state.set_value(content, window, cx);
            state.set_cursor_position(position, window, cx);
        });
    }
}

/// The chip pasted text `n` shows as in the composer over its
/// `[Pasted text #N]` token: `#N`, after the renderer's file icon. The
/// icon is not a `📄` in the label: gpui-kit's tokens draw stray glyphs
/// after an emoji label.
fn pasted_chip(n: usize) -> InlineToken {
    InlineToken::new("pasted-text", attachments::token(Kind::Pasted, n)).with_label(format!("#{n}"))
}

/// The editor position (line, UTF-16 column) of byte `offset` in `text`.
fn position_at(text: &str, offset: usize) -> Position {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() as u32;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .map(char::len_utf16)
        .sum::<usize>() as u32;
    Position::new(line, column)
}
