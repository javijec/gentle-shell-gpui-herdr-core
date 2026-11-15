//! The route table's model picker: a searchable list of the models Pi
//! knows, grouped by provider, in an island like the composer's picker, with
//! ↑/↓/Enter/Esc.

use gentle_core::profiles::normalize_model_id;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::features::profiles::ProfilesView;
use crate::shared::ui;

/// What a row of the picker sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    /// No model of its own (leave untouched / inherit).
    Inherit,
    /// `models[i]`.
    Model(usize),
    /// Type a model id.
    Custom,
}

/// A line of the list: a provider heading or something to pick.
#[derive(Clone, Copy)]
enum Row {
    Provider(usize),
    Pick(Pick),
}

impl ProfilesView {
    /// The list for the current search: the inherit choice (only while not
    /// searching), the matching models under their provider, then the
    /// custom id.
    fn model_rows(&self, cx: &App) -> Vec<Row> {
        let query = self.model_search.read(cx).value().to_string();
        let mut rows = Vec::new();
        if query.trim().is_empty() {
            rows.push(Row::Pick(Pick::Inherit));
        }
        let mut provider: Option<&str> = None;
        for (i, model) in self.models.iter().enumerate() {
            if !model.matches(&query) || normalize_model_id(&model.route_id()).is_none() {
                continue;
            }
            if provider != Some(model.provider.as_str()) {
                provider = Some(&model.provider);
                rows.push(Row::Provider(i));
            }
            rows.push(Row::Pick(Pick::Model(i)));
        }
        rows.push(Row::Pick(Pick::Custom));
        rows
    }

    fn picks(rows: &[Row]) -> Vec<Pick> {
        rows.iter()
            .filter_map(|row| match row {
                Row::Pick(pick) => Some(*pick),
                Row::Provider(_) => None,
            })
            .collect()
    }

    /// The model a row shows now, as its route id; `None` when it inherits
    /// (or, on the every-agent row, when the agents differ).
    fn row_model(&self, key: &str) -> Option<String> {
        self.current_value(key, |e| &e.model).ok().flatten()
    }

    /// Open the picker on route `key` (closing any other), or close it.
    pub(super) fn set_model_menu(
        &mut self,
        key: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.model_menu == key {
            return;
        }
        self.model_menu = key.clone();
        if let Some(key) = key {
            self.model_search.update(cx, |state, cx| {
                state.set_value("", window, cx);
                state.focus(window, cx);
            });
            // Start on the row's own model, so Enter keeps it.
            let current = self.row_model(&key);
            let rows = self.model_rows(cx);
            let picks = Self::picks(&rows);
            self.model_cursor = picks
                .iter()
                .position(|pick| match (pick, &current) {
                    (Pick::Inherit, None) => true,
                    (Pick::Model(i), Some(id)) => self.models[*i].route_id() == *id,
                    _ => false,
                })
                .unwrap_or(0);
            self.scroll_to_cursor(&rows);
        }
        cx.notify();
    }

    fn scroll_to_cursor(&self, rows: &[Row]) {
        let mut seen = 0;
        for (index, row) in rows.iter().enumerate() {
            if let Row::Pick(_) = row {
                if seen == self.model_cursor {
                    self.model_scroll.scroll_to_item(index);
                    return;
                }
                seen += 1;
            }
        }
    }

    /// ↑/↓/Enter/Esc while the model picker is open; Esc for the effort menu.
    pub(in crate::features::profiles) fn model_menu_key(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.effort_menu.is_some() && keystroke.key == "escape" {
            self.effort_menu = None;
            cx.notify();
            return true;
        }
        let Some(key) = self.model_menu.clone() else {
            return false;
        };
        let m = &keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return false;
        }
        let rows = self.model_rows(cx);
        let picks = Self::picks(&rows);
        let n = picks.len();
        let cursor = self.model_cursor.min(n.saturating_sub(1));
        match keystroke.key.as_str() {
            "up" | "down" => {
                self.model_cursor = if keystroke.key == "up" {
                    (cursor + n - 1) % n
                } else {
                    (cursor + 1) % n
                };
                self.scroll_to_cursor(&rows);
            }
            "enter" => {
                if let Some(pick) = picks.get(cursor) {
                    self.choose(&key, *pick, window, cx);
                }
            }
            "escape" => self.set_model_menu(None, window, cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn choose(&mut self, key: &str, pick: Pick, window: &mut Window, cx: &mut Context<Self>) {
        self.set_model_menu(None, window, cx);
        match pick {
            Pick::Inherit => self.set_model(key, None, cx),
            Pick::Model(i) => {
                let id = normalize_model_id(&self.models[i].route_id());
                if id.is_some() && id != self.row_model(key) {
                    self.set_model(key, id, cx);
                }
            }
            Pick::Custom => self.open_custom(key, window, cx),
        }
    }

    /// The picker for route `key`, opened from `trigger`.
    pub(super) fn render_model_menu(
        &self,
        key: &str,
        trigger: Button,
        inherit: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let open = self.model_menu.as_deref() == Some(key);
        let current = self.row_model(key);
        // Only the open picker builds its rows.
        let rows = if open {
            self.model_rows(cx)
        } else {
            Vec::new()
        };
        let cursor = self.model_cursor;
        let models = self.models.clone();
        let search = self.model_search.clone();
        let scroll = self.model_scroll.clone();
        let key = key.to_string();
        let toggle_key = key.clone();
        let toggle = weak.clone();

        Popover::new(SharedString::from(format!("model-menu-{key}")))
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .open(open)
            .track_focus(&self.model_search.focus_handle(cx))
            .on_open_change(move |open, window, cx| {
                let key = open.then(|| toggle_key.clone());
                let _ = toggle.update(cx, |this, cx| this.set_model_menu(key, window, cx));
            })
            .w(px(420.))
            .appearance(false)
            .content(move |_, _, cx| {
                let mut list = v_flex()
                    .id("model-menu-list")
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .px(px(6.))
                    .pb(px(6.));
                let mut index = 0;
                let matched = rows.iter().any(|r| matches!(r, Row::Pick(Pick::Model(_))));
                for row in &rows {
                    let pick = match row {
                        Row::Provider(i) => {
                            let provider = models[*i].provider.clone();
                            list = list.child(
                                h_flex()
                                    .gap_1p5()
                                    .px_2()
                                    .pt_2()
                                    .pb_1()
                                    .child(
                                        div()
                                            .size(px(6.))
                                            .rounded_full()
                                            .bg(ui::provider_color(&provider)),
                                    )
                                    .child(ui::section_label(provider)),
                            );
                            continue;
                        }
                        Row::Pick(pick) => *pick,
                    };
                    let highlighted = index == cursor;
                    index += 1;
                    let (title, detail, icon, chosen): (SharedString, Option<SharedString>, _, _) =
                        match pick {
                            Pick::Inherit => (inherit.into(), None, None, current.is_none()),
                            Pick::Model(i) => {
                                let model = &models[i];
                                (
                                    model.name.clone().into(),
                                    Some(model.id.clone().into()),
                                    None,
                                    current.as_deref() == Some(model.route_id().as_str()),
                                )
                            }
                            Pick::Custom => (
                                if matched {
                                    "Custom model id…".into()
                                } else {
                                    "No model matches. Type a custom id…".into()
                                },
                                None,
                                Some(IconName::Plus),
                                false,
                            ),
                        };
                    let weak = weak.clone();
                    let key = key.clone();
                    list = list.child(
                        h_flex()
                            .id(("model-menu-row", index))
                            .w_full()
                            .gap_2()
                            .px_3()
                            .py_1p5()
                            .when(pick == Pick::Custom, |d| d.mt_1())
                            .rounded(px(ui::ISLAND_ROW))
                            .cursor_pointer()
                            .hover(|s| s.bg(ui::surface_hover()))
                            .when(highlighted, |d| d.bg(ui::surface_hover()))
                            .children(
                                icon.map(|icon| Icon::new(icon).xsmall().text_color(ui::muted())),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.))
                                            .text_color(match pick {
                                                _ if chosen => ui::accent(),
                                                Pick::Model(_) => ui::fg(),
                                                _ => ui::muted(),
                                            })
                                            .child(title),
                                    )
                                    .children(detail.map(|id| {
                                        div()
                                            .truncate()
                                            .font_family(ui::MONO)
                                            .text_size(px(11.))
                                            .text_color(ui::faint())
                                            .child(id)
                                    })),
                            )
                            .when(chosen, |d| {
                                d.child(
                                    Icon::new(IconName::Check).xsmall().text_color(ui::accent()),
                                )
                            })
                            .on_click(move |_, window, cx| {
                                let _ =
                                    weak.update(cx, |this, cx| this.choose(&key, pick, window, cx));
                            }),
                    );
                }
                let content = v_flex()
                    .w_full()
                    .font_family(cx.theme().font_family.clone())
                    .text_color(ui::fg())
                    .child(
                        div().p(px(6.)).child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .h(px(36.))
                                .px_3()
                                .rounded(px(ui::ISLAND_ROW))
                                .bg(ui::well())
                                .child(Icon::new(IconName::Search).xsmall().text_color(ui::faint()))
                                .child(div().flex_1().child(Input::new(&search).appearance(false))),
                        ),
                    )
                    .child(list);
                ui::island(cx)
                    .w_full()
                    .child(ui::popover_in(
                        ElementId::Name("model-menu-in".into()),
                        content,
                    ))
                    .into_any_element()
            })
    }
}
