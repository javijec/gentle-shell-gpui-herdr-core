//! The Theme section: the app's theme grid, window transparency, and
//! sharing themes (export one as a `.gentletheme` file, import one from a
//! file, a link or a drop).

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use super::layout::section;
use crate::shared::backdrop::{self, SHARE_EXTENSION};
use crate::{shared::themes, shared::ui};

/// A shared theme bigger than this isn't downloaded.
const MAX_DOWNLOAD: u64 = 32 << 20;

/// Sharing themes: the link field and how the last import or export went.
pub(super) struct ThemeShare {
    url: Entity<InputState>,
    /// `(text, failed)`.
    note: Option<(String, bool)>,
    /// A link being downloaded.
    busy: bool,
    _url: Subscription,
}

impl ThemeShare {
    pub(super) fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> Self {
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Paste a theme link (https://…/name.gentletheme)")
        });
        let subscription = cx.subscribe_in(
            &url,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    let link = input.read(cx).value().trim().to_string();
                    if !link.is_empty() {
                        input.update(cx, |s, cx| s.set_value("", window, cx));
                        this.import_theme_link(link, cx);
                    }
                }
            },
        );
        Self {
            url,
            note: None,
            busy: false,
            _url: subscription,
        }
    }
}

impl SettingsView {
    fn theme_note(&mut self, note: Result<String, String>, cx: &mut Context<Self>) {
        self.theme_share.note = Some(match note {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        });
        cx.notify();
    }

    fn install_theme(&mut self, text: Result<String, String>, cx: &mut Context<Self>) {
        let result = text.and_then(|text| themes::install_shared(&text, cx));
        self.theme_note(
            result.map(|name| format!("Added \u{201c}{name}\u{201d} and switched to it.")),
            cx,
        );
    }

    /// Import… : pick `.gentletheme` (or theme `.json`) files.
    fn import_theme_files(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Import theme".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| this.import_theme_paths(paths, cx));
        })
        .detach();
    }

    fn import_theme_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("Couldn't read {}: {e}", path.display()));
            self.install_theme(text, cx);
        }
    }

    fn import_theme_link(&mut self, link: String, cx: &mut Context<Self>) {
        if !(link.starts_with("https://") || link.starts_with("http://")) {
            self.theme_note(Err("A theme link starts with https://".into()), cx);
            return;
        }
        self.theme_share.busy = true;
        cx.notify();
        let download = cx.background_executor().spawn(async move {
            let agent: ureq::Agent = ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(30)))
                .build()
                .into();
            let mut response = agent
                .get(&link)
                .call()
                .map_err(|e| format!("Couldn't download the theme: {e}"))?;
            if !response.status().is_success() {
                return Err(format!(
                    "Couldn't download the theme: HTTP {}",
                    response.status()
                ));
            }
            response
                .body_mut()
                .with_config()
                .limit(MAX_DOWNLOAD)
                .read_to_string()
                .map_err(|e| format!("Couldn't download the theme: {e}"))
        });
        cx.spawn(async move |this, cx| {
            let text = download.await;
            let _ = this.update(cx, |this, cx| {
                this.theme_share.busy = false;
                this.install_theme(text, cx);
            });
        })
        .detach();
    }

    /// Share: save the theme as one `.gentletheme` file, scene and image inside.
    fn share_theme(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(theme) = cx.global::<themes::Themes>().catalog.get(index).cloned() else {
            return;
        };
        let (stem, text) = match themes::share_text(&theme) {
            Some(Ok(shared)) => shared,
            Some(Err(err)) => return self.theme_note(Err(format!("Couldn't share it: {err}")), cx),
            None => return,
        };
        let directory = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let chosen = cx.prompt_for_new_path(&directory, Some(&format!("{stem}.{SHARE_EXTENSION}")));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return;
            };
            let written = std::fs::write(&path, text)
                .map(|()| {
                    format!(
                        "Saved {}: send it to anyone with the app, they import it here.",
                        path.display()
                    )
                })
                .map_err(|e| format!("Couldn't save it: {e}"));
            let _ = this.update(cx, |this, cx| {
                if written.is_ok() {
                    cx.reveal_path(&path);
                }
                this.theme_note(written, cx);
            });
        })
        .detach();
    }

    /// Tern-style theme grid: a card per theme drawn in its own colors;
    /// clicking applies it to every window and remembers it. A slider above it
    /// sets how much of the blurred desktop shows through.
    pub(super) fn theme_section(&self, cx: &mut Context<Self>) -> Div {
        let state = cx.global::<themes::Themes>();
        let groups = [
            (themes::Source::Gentle, "Gentle Shell"),
            (themes::Source::App, "Animated backgrounds"),
            (themes::Source::Pi, "Your Pi themes"),
            (themes::Source::Builtin, "Popular"),
        ];
        let percent = (state.transparency * 100.).round();
        let active = state.active_theme().clone();
        let mut card = section(IconName::Sparkles, "Theme")
            .child(
                h_flex()
                    .pt_1()
                    .pb_2()
                    .gap_3()
                    .child(
                        div()
                            .w(px(120.))
                            .text_sm()
                            .text_color(ui::muted())
                            .child("Transparency"),
                    )
                    .child(
                        div().flex_1().child(
                            Slider::new(&self.transparency)
                                .horizontal()
                                .accessibility_label("Window transparency"),
                        ),
                    )
                    .child(
                        div()
                            .w(px(44.))
                            .text_right()
                            .text_sm()
                            .font_family(ui::MONO)
                            .text_color(ui::fg())
                            .child(format!("{percent:.0}%")),
                    ),
            )
            .when(active.background.as_ref().is_some_and(|b| b.scene.is_some()), |d| {
                let motion = backdrop::motion(cx);
                let labels = backdrop::Motion::ALL.map(backdrop::Motion::label);
                d.child(
                    h_flex()
                        .pb_2()
                        .gap_3()
                        .child(
                            div()
                                .w(px(120.))
                                .text_sm()
                                .text_color(ui::muted())
                                .child("Scene motion"),
                        )
                        .child(ui::segmented(
                            "backdrop-motion",
                            &labels,
                            backdrop::Motion::ALL.iter().position(|m| *m == motion),
                            cx.listener(|_, index: &usize, _, cx| {
                                backdrop::set_motion(backdrop::Motion::ALL[*index], cx);
                                cx.notify();
                            }),
                        ))
                        .child(
                            div()
                                .flex_1()
                                .text_xs()
                                .text_color(ui::faint())
                                .child("Calm moves at half the rate; Still costs nothing. Scenes only move while this window is in front."),
                        ),
                )
            })
            .when(active.oled, |d| {
                d.child(
                    div()
                        .pb_2()
                        .text_xs()
                        .text_color(ui::muted())
                        .child("OLED Black stays solid black at any transparency, so an OLED screen keeps those pixels off."),
                )
            })
            .when_some(backdrop::scene_error(&active.name), |d, error| {
                d.child(
                    div()
                        .pb_2()
                        .text_xs()
                        .text_color(ui::danger())
                        .child(format!("This theme's scene stopped: {error}")),
                )
            });
        let cards_of = |source: themes::Source, cx: &mut Context<Self>| -> Vec<Stateful<Div>> {
            let state = cx.global::<themes::Themes>();
            let entries: Vec<(usize, themes::Theme)> = state
                .catalog
                .iter()
                .enumerate()
                .filter(|(_, t)| t.source == source)
                .map(|(i, t)| (i, t.clone()))
                .collect();
            let active = state.active;
            entries
                .into_iter()
                .map(|(i, t)| theme_card(i, &t, i == active, cx))
                .collect()
        };
        for (source, label) in groups {
            let cards = cards_of(source, cx);
            if cards.is_empty() {
                continue;
            }
            card = card
                .child(div().pt_2().pb_1p5().child(ui::section_label(label)))
                .when(source == themes::Source::App, |d| {
                    d.child(self.share_bar(cx))
                })
                .child(div().grid().grid_cols(4).gap_2().children(cards));
        }
        card
    }

    /// Above the animated themes: how to get more, and importing them.
    fn share_bar(&self, cx: &mut Context<Self>) -> Div {
        let share = &self.theme_share;
        v_flex()
            .gap_2()
            .pb_2()
            .child(
                div()
                    .text_xs()
                    .text_color(ui::muted())
                    .child("Ask an agent for one (\u{201c}a theme with koi in a pond, in my colors\u{201d}): it writes the theme and its scene to the app's themes folder and it shows up here. Share any of them with the share button on its card; import one you got with a link, Import\u{2026}, or by dropping its file here."),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&share.url).small()))
                    .when(share.busy, |d| {
                        d.child(div().text_xs().text_color(ui::muted()).child("Downloading…"))
                    })
                    .child(
                        Button::new("theme-import")
                            .small()
                            .icon(Icon::new(IconName::Download))
                            .label("Import…")
                            .on_click(cx.listener(|this, _, _, cx| this.import_theme_files(cx))),
                    )
                    .when_some(themes::app_themes_dir(), |d, dir| {
                        d.child(
                            Button::new("theme-folder")
                                .small()
                                .ghost()
                                .icon(Icon::new(IconName::FolderOpen))
                                .tooltip("Open the themes folder")
                                .on_click(move |_, _, cx| {
                                    let _ = std::fs::create_dir_all(&dir);
                                    cx.open_url(&format!("file://{}", dir.display()))
                                }),
                        )
                    }),
            )
            .when_some(share.note.clone(), |d, (text, failed)| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(if failed { ui::danger() } else { ui::success() })
                        .child(text),
                )
            })
    }

    /// Theme files dropped on the section are imported.
    pub(super) fn theme_drop(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        let themes: Vec<PathBuf> = paths
            .paths()
            .iter()
            .filter(|p| {
                p.extension()
                    .is_some_and(|x| x == SHARE_EXTENSION || x == "json")
            })
            .cloned()
            .collect();
        if !themes.is_empty() {
            self.import_theme_paths(themes, cx);
        }
    }
}

fn theme_card(
    index: usize,
    theme: &themes::Theme,
    selected: bool,
    cx: &mut Context<SettingsView>,
) -> Stateful<Div> {
    let solid = |hex: u32| -> Hsla { rgb(hex).into() };
    // Red, yellow, green, cyan, blue, magenta, then the accent.
    let swatches = [1usize, 3, 2, 6, 4, 5]
        .into_iter()
        .map(|i| theme.ansi[i])
        .chain([theme.accent]);
    let sky = theme
        .background
        .as_ref()
        .filter(|b| b.sky.len() > 1)
        .map(|b| (b.sky[0], b.sky[b.sky.len() - 1]));
    let shareable = theme.source == themes::Source::App;
    div()
        .id(("theme-card", index))
        .flex()
        .flex_col()
        .gap_1p5()
        .p_1p5()
        .rounded_xl()
        .border_2()
        .border_color(if selected { ui::accent() } else { ui::border() })
        .when(selected, |d| d.bg(ui::accent().opacity(0.08)))
        .cursor_pointer()
        .hover(|d| d.bg(ui::surface_hover()))
        .on_click(move |_, _, cx| themes::select(index, cx))
        .child(
            v_flex()
                .gap_2()
                .px_2p5()
                .py_2()
                .rounded_lg()
                .bg(solid(theme.bg))
                // An animated theme's card shows its sky.
                .when_some(sky, |d, (top, bottom)| {
                    d.bg(linear_gradient(
                        180.,
                        linear_color_stop(solid(top), 0.),
                        linear_color_stop(solid(bottom), 1.),
                    ))
                })
                .border_1()
                .border_color(solid(theme.border).opacity(0.7))
                .font_family(ui::MONO)
                .text_xs()
                .child(
                    h_flex()
                        .gap_1()
                        .overflow_hidden()
                        .child(div().text_color(solid(theme.accent)).child("❯"))
                        .child(div().text_color(solid(theme.fg)).child("Aa"))
                        .child(div().text_color(solid(theme.ansi[2])).child("ok"))
                        .child(div().text_color(solid(theme.muted)).child("~/src")),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .children(swatches.map(|c| div().size_2().rounded_full().bg(solid(c)))),
                ),
        )
        .child(
            h_flex()
                .gap_1()
                .px(px(2.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(if selected { ui::fg() } else { ui::muted() })
                        .child(theme.name.clone()),
                )
                .when(selected, |d| {
                    d.child(Icon::new(IconName::Check).xsmall().text_color(ui::accent()))
                })
                .when(shareable, |d| {
                    d.child(
                        Button::new(("theme-share", index))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(IconName::Share2))
                            .tooltip("Share: save it as one file")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.share_theme(index, cx);
                            })),
                    )
                }),
        )
}
