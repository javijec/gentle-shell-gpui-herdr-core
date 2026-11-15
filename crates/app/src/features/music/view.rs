//! The player as it shows: the ♪ button in the title bar and the island it
//! opens (the piece over a moving equalizer, controls, volume and the mood
//! field).

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::slider::Slider;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Disableable as _, Icon, Selectable, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Job, Music};
use crate::shared::ui;

/// The equalizer's height and bar count.
const EQ_H: f32 = 40.;
const EQ_BARS: usize = 34;

/// A stable 0..1 per bar, so each bar keeps its own pace.
fn bar_seed(i: usize, salt: u32) -> f32 {
    let mut x = (i as u32).wrapping_mul(0x9E37_79B9) ^ salt.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65536.
}

/// Bar `i`'s height (0..1) at time `t`: two slow waves rolling across the
/// bars and a quicker flicker of its own. gentle-stream's `ctl --json`
/// reports no levels, so this follows the music's pulse, not its sound.
fn bar_level(i: usize, t: f32) -> f32 {
    let x = i as f32 / EQ_BARS as f32;
    let swell = 0.5 + 0.5 * (t * 1.1 - x * 5.5).sin();
    let roll = 0.5 + 0.5 * (t * 0.6 + x * 9.0 + 1.3).sin();
    let own = 0.5 + 0.5 * (t * (2.2 + 2.6 * bar_seed(i, 1)) + 6.3 * bar_seed(i, 2)).sin();
    // Lower in the middle-low range less, so the shape reads as music.
    let tilt = 0.75 + 0.25 * (std::f32::consts::PI * x).sin();
    ((0.15 + 0.45 * swell * roll + 0.4 * own * swell) * tilt).clamp(0.06, 1.)
}

impl Music {
    /// The piece: a petal disc with a note, its name and what it is, and the
    /// stop button; or the invitation to start.
    fn render_header(&self, cx: &mut Context<Self>) -> Div {
        let (title, details) = match &self.state {
            Some(state) => {
                let mut details = state.details();
                if !state.mood.is_empty() {
                    details = format!("{details} · \u{201c}{}\u{201d}", state.mood);
                }
                (state.name.clone(), details)
            }
            None => (
                "Music".to_string(),
                "Composed live by gentle-stream".to_string(),
            ),
        };
        h_flex()
            .gap_2p5()
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::ISLAND_ROW))
                    .bg(ui::petal().opacity(0.14))
                    .text_color(ui::petal())
                    .child(Icon::new(IconName::Music).small()),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ui::fg())
                            .truncate()
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(ui::muted())
                            .truncate()
                            .child(details),
                    ),
            )
            .when(self.state.is_some(), |d| {
                d.child(
                    Button::new("music-stop")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Square))
                        .tooltip("Stop the music")
                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                )
            })
    }

    /// The equalizer, in a sunken well: bars moving with the music while it
    /// plays (at the slow live rate, still with reduced motion), resting low
    /// when paused.
    fn render_equalizer(&self, playing: bool, cx: &App) -> Div {
        let moving = playing && !cx.reduce_motion();
        let t = ui::live_clock();
        div()
            .px_3()
            .py_2p5()
            .rounded(px(ui::ISLAND_ROW))
            .bg(ui::well())
            .child(
                h_flex()
                    .h(px(EQ_H))
                    .items_center()
                    .gap(px(3.))
                    .children((0..EQ_BARS).map(move |i| {
                        let level = if moving {
                            bar_level(i, t)
                        } else if playing {
                            bar_level(i, 0.)
                        } else {
                            0.08
                        };
                        // Centered bars, mirrored up and down like a waveform.
                        div()
                            .flex_1()
                            .h(px(3. + level * (EQ_H - 3.)))
                            .rounded_full()
                            .bg(if playing {
                                ui::petal().opacity(0.4 + 0.6 * level)
                            } else {
                                ui::faint().opacity(0.5)
                            })
                    }))
                    .when(moving, |d| {
                        d.child(
                            canvas(
                                |_, window, cx| ui::request_slow_frame(window, cx),
                                |_, _, _, _| {},
                            )
                            .size_0(),
                        )
                    }),
            )
    }

    fn render_now(&self, cx: &mut Context<Self>) -> Div {
        let Some(state) = &self.state else {
            return v_flex()
                .py_2()
                .gap_2()
                .items_center()
                .child(self.render_equalizer(false, cx).w_full())
                .child(
                    Button::new("music-start")
                        .small()
                        .primary()
                        .icon(Icon::new(IconName::Play))
                        .label("Start music")
                        .disabled(self.job.is_some())
                        .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                );
        };
        let playing = state.playing;
        let volume = self.volume.read(cx).value().start();
        v_flex()
            .gap_3()
            .child(self.render_equalizer(playing, cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        // The main control: a round petal button.
                        div()
                            .id("music-toggle")
                            .size(px(32.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .cursor_pointer()
                            .bg(ui::petal())
                            .text_color(ui::popover_bg())
                            .hover(|d| d.bg(ui::petal_active()))
                            .tooltip(move |window, cx| {
                                Tooltip::new(if playing { "Pause" } else { "Play" })
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
                            .child(
                                Icon::new(if playing {
                                    IconName::Pause
                                } else {
                                    IconName::Play
                                })
                                .small(),
                            ),
                    )
                    .child(
                        Button::new("music-next")
                            .ghost()
                            .small()
                            .icon(Icon::new(IconName::SkipForward))
                            .tooltip("Next piece, same mood")
                            .on_click(cx.listener(|this, _, _, cx| this.next(cx))),
                    )
                    .child(div().w(px(1.)).h(px(16.)).mx_1().bg(ui::border()))
                    .child(Icon::new(IconName::Volume2).small().text_color(ui::faint()))
                    .child(
                        div().flex_1().child(
                            Slider::new(&self.volume)
                                .horizontal()
                                .accessibility_label("Music volume"),
                        ),
                    )
                    .child(
                        div()
                            .w(px(30.))
                            .text_right()
                            .text_xs()
                            .font_family(ui::MONO)
                            .text_color(ui::faint())
                            .child(format!("{volume:.0}%")),
                    ),
            )
    }

    fn render_mood(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap_1p5()
            .child(
                ui::field()
                    .child(
                        Icon::new(IconName::Sparkles)
                            .small()
                            .text_color(ui::faint()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.mood).appearance(false).small()),
                    )
                    .child(
                        ui::key_button("music-mood-send", "↵").on_click(
                            cx.listener(|this, _, window, cx| this.submit_mood(window, cx)),
                        ),
                    ),
            )
            .child(match (&self.job, &self.note) {
                (Some(job), _) => h_flex()
                    .gap_2()
                    .text_xs()
                    .text_color(ui::muted())
                    .child(Spinner::new().xsmall().color(ui::accent()))
                    .child(match job {
                        Job::Starting => "Starting the player…".to_string(),
                        Job::Composing(mood) => format!("Composing “{mood}”…"),
                    }),
                (None, Some((text, ok))) => div()
                    .text_xs()
                    .text_color(if *ok { ui::success() } else { ui::danger() })
                    .child(text.clone()),
                (None, None) => div()
                    .text_xs()
                    .text_color(ui::faint())
                    .child("Any language; Gentle Shell's model reads it."),
            })
    }
}

impl Render for Music {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .p_3()
            .gap_3()
            .child(self.render_header(cx))
            .child(self.render_now(cx))
            .child(self.render_mood(cx))
    }
}

/// The title bar's ♪ button, once gentle-stream is installed. It wears a
/// small equalizer while the music plays.
pub fn title_button(music: &Entity<Music>, cx: &App) -> Option<AnyElement> {
    let model = music.read(cx);
    if !model.available() {
        return None;
    }
    let trigger = MusicTrigger {
        playing: model.playing(),
        open: false,
    };
    let content = music.clone();
    let watch = music.clone();
    Some(
        div()
            // Clicks here are not title-bar drags.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Popover::new("music")
                    .anchor(Anchor::TopRight)
                    .trigger(trigger)
                    .on_open_change(move |open, _, cx| {
                        let open = *open;
                        watch.update(cx, |music, cx| music.set_watching(open, cx));
                    })
                    .w(px(340.))
                    // The island, as the composer's menus and chip cards.
                    .appearance(false)
                    .content(move |_, _, cx| {
                        ui::island(cx).w_full().child(ui::popover_in(
                            ElementId::Name("music-in".into()),
                            div().child(content.clone()),
                        ))
                    }),
            )
            .into_any_element(),
    )
}

#[derive(IntoElement)]
struct MusicTrigger {
    playing: bool,
    open: bool,
}

impl Selectable for MusicTrigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl RenderOnce for MusicTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        const BARS: [f32; 3] = [6., 11., 8.];
        div()
            .id("music-button")
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .cursor_pointer()
            .hover(|s| s.bg(ui::surface_hover()))
            .when(self.open, |d| d.bg(ui::surface_hover()))
            .when(!self.open, |d| {
                d.tooltip(|window, cx| Tooltip::new("Music").build(window, cx))
            })
            .child(if self.playing {
                h_flex()
                    .gap(px(2.))
                    .items_end()
                    .h(px(12.))
                    .children(
                        BARS.map(|h| div().w(px(2.5)).h(px(h)).rounded_full().bg(ui::petal())),
                    )
                    .into_any_element()
            } else {
                Icon::new(IconName::Music)
                    .small()
                    .text_color(ui::fg())
                    .into_any_element()
            })
    }
}
