//! The route table's model, effort and tier cells and their menus; the
//! model picker itself is `model_menu`.

use gentle_core::profiles::{
    ORCHESTRATOR, RouteEntry, THINKING_LEVELS, clamp_level, is_review_role,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::features::profiles::{ALL_ROW, ProfilesView};
use crate::shared::ui;

use super::{EFFORT_W, TIER_W};

impl ProfilesView {
    pub(in crate::features::profiles) fn inherit_label(&self, key: &str) -> &'static str {
        match key {
            ORCHESTRATOR => "leave untouched",
            ALL_ROW => "inherit",
            k if is_review_role(k) => "Pi persisted default model",
            _ => "inherit",
        }
    }

    /// The value one row shows; the every-agent row shows the shared value or
    /// "mixed".
    pub(super) fn current_value(
        &self,
        key: &str,
        field: fn(&RouteEntry) -> &Option<String>,
    ) -> Result<Option<String>, ()> {
        if key == ALL_ROW {
            let values: Vec<Option<String>> = self
                .names
                .iter()
                .map(|n| self.draft.get(n).and_then(|e| field(e).clone()))
                .collect();
            return match values.first() {
                Some(first) if values.iter().all(|v| v == first) => Ok(first.clone()),
                Some(_) => Err(()),
                None => Ok(None),
            };
        }
        Ok(self.draft.get(key).and_then(|e| field(e).clone()))
    }

    fn is_edited(&self, key: &str, field: fn(&RouteEntry) -> &Option<String>) -> bool {
        if key == ALL_ROW {
            return false;
        }
        self.draft.get(key).and_then(|e| field(e).as_ref())
            != self.stored.get(key).and_then(|e| field(e).as_ref())
    }

    pub(super) fn render_model_cell(&self, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let current = self.current_value(key, |e| &e.model);
        let inherit = self.inherit_label(key);
        let label = match &current {
            Ok(Some(model)) => model.clone(),
            Ok(None) if key != ORCHESTRATOR && is_review_role(key) => "Pi default".into(),
            Ok(None) => inherit.to_string(),
            Err(()) => "mixed".into(),
        };
        let edited = self.is_edited(key, |e| &e.model);
        let color = if edited {
            ui::warning()
        } else if matches!(current, Ok(Some(_))) {
            ui::fg()
        } else {
            ui::muted()
        };
        let trigger = Button::new(SharedString::from(format!("model-{key}")))
            .ghost()
            .xsmall()
            .w_full()
            .font_family(ui::MONO)
            .text_color(color)
            .label(label)
            .dropdown_caret(true)
            .disabled(self.busy);
        // A stretched column gives the trigger a definite width, so a long
        // model id ellipsizes instead of spilling into the effort column.
        v_flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .child(self.render_model_menu(key, trigger, inherit, cx))
            .into_any_element()
    }

    pub(super) fn render_effort_cell(&self, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let model = if key == ALL_ROW {
            None
        } else {
            self.draft.get(key).and_then(|e| e.model.clone())
        };
        if key == ORCHESTRATOR && model.is_none() {
            return div()
                .w(px(EFFORT_W))
                .flex_shrink_0()
                .px_2()
                .text_xs()
                .text_color(ui::faint())
                .child("follows model")
                .into_any_element();
        }
        let supported = if key == ALL_ROW {
            let known: Vec<Vec<&str>> = self
                .names
                .iter()
                .filter_map(|n| {
                    self.supported_levels(self.draft.get(n).and_then(|e| e.model.as_deref()))
                })
                .collect();
            (!known.is_empty()).then(|| {
                THINKING_LEVELS
                    .iter()
                    .copied()
                    .filter(|l| known.iter().all(|k| k.contains(l)))
                    .collect::<Vec<_>>()
            })
        } else {
            self.supported_levels(model.as_deref())
        };
        let current = self.current_value(key, |e| &e.thinking);
        let label = match &current {
            Ok(Some(level)) => match &supported {
                Some(levels) if !levels.contains(&level.as_str()) => {
                    format!("{level} → {}", clamp_level(level, levels))
                }
                _ => level.clone(),
            },
            Ok(None) if is_review_role(key) => "Pi default".into(),
            Ok(None) => "inherit".into(),
            Err(()) => "mixed".into(),
        };
        let edited = self.is_edited(key, |e| &e.thinking);
        let color = if edited {
            ui::warning()
        } else if matches!(current, Ok(Some(_))) {
            ui::fg()
        } else {
            ui::muted()
        };
        let options: Vec<&'static str> = THINKING_LEVELS
            .iter()
            .copied()
            .filter(|l| supported.as_ref().is_none_or(|s| s.contains(l)))
            .collect();
        let inherit = if is_review_role(key) {
            "Pi persisted default effort"
        } else {
            "Inherit effort"
        };
        let weak = cx.entity().downgrade();
        let toggle = weak.clone();
        let open = self.effort_menu.as_deref() == Some(key);
        let key = key.to_string();
        let toggle_key = key.clone();
        let current = current.ok().flatten();
        let trigger = Button::new(SharedString::from(format!("effort-{key}")))
            .ghost()
            .xsmall()
            .w_full()
            .font_family(ui::MONO)
            .text_color(color)
            .label(label)
            .dropdown_caret(true)
            .disabled(self.busy);
        let menu = Popover::new(SharedString::from(format!("effort-menu-{key}")))
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .open(open)
            .on_open_change(move |open, _, cx| {
                let key = open.then(|| toggle_key.clone());
                let _ = toggle.update(cx, |this, cx| {
                    this.effort_menu = key;
                    cx.notify();
                });
            })
            .w(px(240.))
            .appearance(false)
            .content(move |_, _, cx| {
                let row = |i: usize, label: SharedString, level: Option<String>| {
                    let chosen = current == level;
                    let weak = weak.clone();
                    let key = key.clone();
                    h_flex()
                        .id(("effort-row", i))
                        .w_full()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .rounded(px(ui::ISLAND_ROW))
                        .cursor_pointer()
                        .hover(|s| s.bg(ui::surface_hover()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.))
                                .when(level.is_some(), |d| d.font_family(ui::MONO))
                                .text_color(if chosen {
                                    ui::accent()
                                } else if level.is_some() {
                                    ui::fg()
                                } else {
                                    ui::muted()
                                })
                                .child(label),
                        )
                        .when(chosen, |d| {
                            d.child(Icon::new(IconName::Check).xsmall().text_color(ui::accent()))
                        })
                        .on_click(move |_, _, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.effort_menu = None;
                                this.set_thinking(&key, level.clone(), cx);
                            });
                        })
                };
                let rows = std::iter::once(row(0, inherit.into(), None)).chain(
                    options
                        .iter()
                        .enumerate()
                        .map(|(i, l)| row(i + 1, (*l).into(), Some(l.to_string()))),
                );
                ui::island(cx)
                    .w_full()
                    .child(ui::popover_in(
                        ElementId::Name("effort-menu-in".into()),
                        v_flex()
                            .w_full()
                            .p(px(6.))
                            .font_family(cx.theme().font_family.clone())
                            .children(rows),
                    ))
                    .into_any_element()
            });
        v_flex()
            .w(px(EFFORT_W))
            .flex_shrink_0()
            .overflow_hidden()
            .child(menu)
            .into_any_element()
    }

    pub(super) fn render_tier_cell(&self, key: &str, cx: &mut Context<Self>) -> AnyElement {
        let entry = self.draft.get(key);
        let model = entry.and_then(|e| e.model.as_deref());
        let tier = entry.and_then(|e| e.tier.as_deref());
        let edited = self.is_edited(key, |e| &e.tier);
        let cell = div().w(px(TIER_W)).flex_shrink_0();
        if key == ALL_ROW {
            return cell.into_any_element();
        }
        if !self.fast_capable(model) {
            let text = match tier {
                Some(tier) => format!("{tier} (n/a)"),
                None => "—".into(),
            };
            return cell
                .px_2()
                .text_xs()
                .text_color(ui::faint())
                .child(text)
                .into_any_element();
        }
        let fast = tier == Some("fast");
        let key = key.to_string();
        cell.child(
            Button::new(SharedString::from(format!("tier-{key}")))
                .xsmall()
                .when(fast, |b| b.outline().selected(true))
                .when(!fast, |b| b.ghost())
                .when(edited, |b| b.text_color(ui::warning()))
                .icon(Icon::new(IconName::Zap).when(fast, |i| i.text_color(ui::champagne())))
                .label(match tier {
                    Some("fast") => "Fast",
                    Some(_) => "Standard",
                    None => "Standard",
                })
                .tooltip("OpenAI Fast (service_tier priority): 2.5x subscription usage or 2x purchased credits")
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_tier(&key, cx))),
        )
        .into_any_element()
    }
}
