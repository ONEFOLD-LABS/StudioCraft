//! StudioCraft's look: the design tokens of its dark theme and the egui visuals built from them.
//! StudioCraft has only a dark theme.
//!
//! The token set follows SoundCraft's DAW theme (`crates/ui-egui/src/theme.rs` in
//! storytold/soundcraft, MIT OR Apache-2.0). The colours come from the ONEFOLD LABS brand book
//! (ONEFOLD-LABS/brand, MIT): surfaces, lines and text from its neutrals (`paper`, `surface`,
//! `surface-sunken`, `line`, `ink`, `ink-muted`), the accent from `apps/palette.json` (a lighter
//! wine than `app-studiocraft`, with near-black text) and the
//! semantic colours (success, warning, danger, info) from the same file. The DAW's own colours
//! (lanes, grid, ruler, keys) are mixed from those neutrals; record, solo, mute, monitor and the
//! meters keep their conventional hues. Because a DAW already uses red for recording, `rec` is a
//! brighter orange red and the playhead is neutral, so the accent and a recording are never
//! mistaken for one another.
//!
//! Use: call [`install`] once at start-up and read colours with [`tokens`] wherever the UI paints.

use egui::{Color32, Context, CornerRadius, Stroke, Theme, ThemePreference, Visuals};

/// StudioCraft's app colour, the same wine red as the app icon.
pub const APP_COLOR: Color32 = Color32::from_rgb(0x9e, 0x23, 0x43);

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

const fn rgba(hex: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied_const((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, alpha)
}

/// Every colour the StudioCraft UI paints with. Track colours stay the user's own and are not here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    // surfaces
    pub window_bg: Color32,
    pub panel_bg: Color32,
    pub panel_bg2: Color32,
    pub toolbar_bg: Color32,
    pub border: Color32,
    pub border_light: Color32,
    // text
    pub text: Color32,
    pub text_dim: Color32,
    pub text_on_accent: Color32,
    // accent and controls
    pub accent: Color32,
    /// App-Zeichen (Scheibe mit ausklingender Welle): Verlauf von hell nach dunkel, in beiden
    /// Themes gleich (wie das App-Icon).
    pub brand_light: Color32,
    pub brand_dark: Color32,
    pub accent_dark: Color32,
    pub button: Color32,
    pub button_hi: Color32,
    // ruler and arrangement
    pub ruler_bg: Color32,
    pub ruler_text: Color32,
    pub ruler_tick: Color32,
    pub lane_bg: Color32,
    pub lane_alt: Color32,
    pub grid_line: Color32,
    pub bar_line: Color32,
    pub selection: Color32,
    pub row_selected: Color32,
    pub cycle_fill: Color32,
    pub punch_fill: Color32,
    pub playhead: Color32,
    // events
    pub event_text_muted: Color32,
    pub event_muted: Color32,
    pub event_shade: Color32,
    pub event_selected: Color32,
    pub handle: Color32,
    // transport and track states
    pub rec: Color32,
    pub solo: Color32,
    pub mute: Color32,
    pub monitor: Color32,
    pub counter_bg: Color32,
    pub counter_text: Color32,
    // mixer
    pub meter_green: Color32,
    pub meter_yellow: Color32,
    pub meter_red: Color32,
    /// Edge of the fader rail in the mixer (3:1 against the channel strip).
    pub fader_track: Color32,
    /// Namensfeld der Kanalzüge von Eingangs- und Ausgangsbussen im Mixer.
    pub bus_input: Color32,
    pub bus_output: Color32,
    pub strip_bg: Color32,
    // key editor
    pub key_white: Color32,
    pub key_black: Color32,
    pub key_line: Color32,
    pub key_pressed: Color32,
    pub key_row: Color32,
    pub note_selected: Color32,
    /// Overlay on the rows of black keys in the note grid.
    pub key_black_row: Color32,
    /// Overlay on the note grid outside the edited part.
    pub outside_part: Color32,
    // states in text and icons (brand `semantic`)
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        window_bg: rgb(0x0b0e10),
        panel_bg: rgb(0x171d20),
        panel_bg2: rgb(0x202628),
        toolbar_bg: rgb(0x101416),
        border: rgb(0x3a454a),
        border_light: rgb(0x9aa8ae),
        text: rgb(0xeef2f1),
        text_dim: rgb(0x9aa8ae),
        text_on_accent: rgb(0x08181b),
        accent: rgb(0xeb7287),
        brand_light: rgb(0xd9587a),
        brand_dark: rgb(0x8e1f3c),
        accent_dark: rgb(0xfe93a2),
        button: rgb(0x252d30),
        button_hi: rgb(0x333c40),
        ruler_bg: rgb(0x101416),
        ruler_text: rgb(0x9aa8ae),
        ruler_tick: rgb(0x5c686d),
        lane_bg: rgb(0x111618),
        lane_alt: rgb(0x171d20),
        grid_line: rgb(0x272f33),
        bar_line: rgb(0x3a454a),
        selection: rgba(0xeb7287, 77),
        row_selected: rgb(0x282e31),
        cycle_fill: rgba(0xeb7287, 56),
        punch_fill: rgba(0xf0452e, 77),
        playhead: rgb(0xeef2f1),
        event_text_muted: rgb(0xc8d0d3),
        event_muted: rgb(0x4b5458),
        event_shade: rgb(0x0b0e10),
        event_selected: rgb(0xeef2f1),
        handle: rgb(0xffffff),
        rec: rgb(0xf0452e),
        solo: rgb(0xdec434),
        mute: rgb(0xe89428),
        monitor: rgb(0x46aa5a),
        counter_bg: rgb(0x0b0e10),
        counter_text: rgb(0xfe93a2),
        meter_green: rgb(0x3cc850),
        meter_yellow: rgb(0xe6d23c),
        meter_red: rgb(0xf0452e),
        fader_track: rgb(0x6a767c),
        bus_input: rgb(0x7fa37a),
        bus_output: rgb(0x8f97a8),
        strip_bg: rgb(0x1b2225),
        key_white: rgb(0xe2e7e6),
        key_black: rgb(0x161b1e),
        key_line: rgb(0x6a767c),
        key_pressed: rgb(0xeb7287),
        key_row: rgb(0x472026),
        note_selected: rgb(0xffffff),
        key_black_row: rgba(0x000000, 60),
        outside_part: rgba(0x000000, 90),
        success: rgb(0x319751),
        warning: rgb(0xb57a08),
        danger: rgb(0xd2655b),
        info: rgb(0x258bd2),
    };

    /// egui visuals for this token set, on egui's dark base.
    pub fn visuals(&self) -> Visuals {
        let mut v = Visuals::dark();
        v.panel_fill = self.panel_bg;
        v.window_fill = self.panel_bg2;
        v.window_stroke = Stroke::new(1.0, self.border_light);
        v.extreme_bg_color = self.window_bg;
        v.faint_bg_color = self.lane_alt;
        v.code_bg_color = self.toolbar_bg;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.selection;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.override_text_color = Some(self.text);
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);

        // The same widget-state assignment as craft-ui's WidgetPalette; swap for it once craft-ui is a dependency.
        let radius = CornerRadius::same(3);
        for (w, fill, fg) in [
            (&mut v.widgets.noninteractive, self.panel_bg, self.text_dim),
            (&mut v.widgets.inactive, self.button, self.text),
            (&mut v.widgets.hovered, self.button_hi, self.text),
            // Pressed: neutral ground with an accent border. egui draws `strong` text in the
            // pressed text colour, so it must be the body text, not the text on the accent.
            (&mut v.widgets.active, self.button_hi, self.text),
            (&mut v.widgets.open, self.button_hi, self.text),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = radius;
            w.fg_stroke = Stroke::new(w.fg_stroke.width, fg);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        // Brand book: borders of buttons and inputs in `ink-muted`, at least 3:1.
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, self.border_light);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.5, self.accent);
        v
    }
}

/// Install StudioCraft's visuals. StudioCraft is always dark, also when the system asks for a
/// light theme: egui's light slot gets the dark visuals too.
pub fn install(ctx: &Context) {
    let visuals = Tokens::DARK.visuals();
    ctx.set_visuals_of(Theme::Dark, visuals.clone());
    ctx.set_visuals_of(Theme::Light, visuals);
    ctx.set_theme(ThemePreference::Dark);
}

/// The colours to paint with (the dark theme; `ctx` keeps call sites free to change later).
pub fn tokens(_ctx: &Context) -> &'static Tokens {
    &Tokens::DARK
}

/// WCAG relative luminance of a colour.
pub(crate) fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio of two colours (1 to 21).
pub(crate) fn contrast(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Black or white, whichever reads better on `bg`: text on track colours (event heads, name
/// plates, the inspector head). Works for the user's own track colours too.
pub(crate) fn text_on(bg: Color32) -> Color32 {
    if contrast(Color32::BLACK, bg) >= contrast(Color32::WHITE, bg) {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn distance(a: Color32, b: Color32) -> i32 {
        (i32::from(a.r()) - i32::from(b.r())).abs()
            + (i32::from(a.g()) - i32::from(b.g())).abs()
            + (i32::from(a.b()) - i32::from(b.b())).abs()
    }

    #[test]
    fn accent_and_recording_stay_apart() {
        let t = Tokens::DARK;
        assert!(
            distance(t.accent, t.rec) > 60,
            "accent {:?} too close to rec {:?}",
            t.accent,
            t.rec
        );
        assert!(distance(t.playhead, t.rec) > 200);
    }

    /// Text and state colours stay readable (WCAG AA 4.5:1) on every ground they are drawn on;
    /// control borders, the playhead and the accent as a mark hold 3:1 (brand book).
    #[test]
    fn text_and_state_colours_are_readable() {
        let t = Tokens::DARK;
        let grounds = [
            ("panel", t.panel_bg),
            ("toolbar", t.toolbar_bg),
            ("lane", t.lane_bg),
            ("lane_alt", t.lane_alt),
        ];
        for (ground, bg) in grounds {
            for (name, fg, min) in [
                ("text", t.text, 4.5),
                ("text_dim", t.text_dim, 4.5),
                ("success", t.success, 4.5),
                ("warning", t.warning, 4.5),
                ("danger", t.danger, 4.5),
                ("info", t.info, 4.5),
                ("accent", t.accent, 3.0),
                ("border_light", t.border_light, 3.0),
                ("playhead", t.playhead, 3.0),
            ] {
                let c = contrast(fg, bg);
                assert!(c >= min, "{name} on {ground}: {c:.2}");
            }
        }
        for (name, fg, bg) in [
            ("text on accent", t.text_on_accent, t.accent),
            ("text on selected row", t.text, t.row_selected),
            ("text on raised panel", t.text, t.panel_bg2),
            ("ruler text", t.ruler_text, t.ruler_bg),
            ("counter", t.counter_text, t.counter_bg),
        ] {
            let c = contrast(fg, bg);
            assert!(c >= 4.5, "{name}: {c:.2}");
        }
    }

    /// The dark theme in detail: muted event names, state colours used as text on every ground
    /// they appear on, and the graphics a user reads (fader rail, piano key lines, ruler ticks,
    /// meters) hold WCAG contrast.
    #[test]
    fn dark_theme_details_are_readable() {
        let t = Tokens::DARK;
        let grounds = [
            ("panel", t.panel_bg),
            ("raised panel", t.panel_bg2),
            ("toolbar", t.toolbar_bg),
            ("strip", t.strip_bg),
            ("lane", t.lane_bg),
            ("lane_alt", t.lane_alt),
        ];
        for (ground, bg) in grounds {
            for (name, fg) in [("text", t.text), ("text_dim", t.text_dim)] {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{name} on {ground}: {c:.2}");
            }
        }
        let states = [
            ("rec", t.rec),
            ("solo", t.solo),
            ("mute", t.mute),
            ("monitor", t.monitor),
        ];
        // As text, the state colours appear in the transport bar and the status line.
        for (ground, bg) in [("panel", t.panel_bg), ("toolbar", t.toolbar_bg)] {
            for (name, fg) in states {
                let c = contrast(fg, bg);
                assert!(c >= 4.5, "{name} as text on {ground}: {c:.2}");
            }
        }
        // Switched-on state buttons (M, S, R, W) fill with the state colour and write black.
        for (name, fill) in states {
            let c = contrast(Color32::BLACK, fill);
            assert!(c >= 4.5, "black on {name}: {c:.2}");
        }
        let c = contrast(t.event_text_muted, t.event_muted);
        assert!(c >= 4.5, "muted event name: {c:.2}");
        for (name, fg, bg) in [
            ("fader rail edge on strip", t.fader_track, t.strip_bg),
            ("fader rail edge on panel", t.fader_track, t.panel_bg),
            ("lines between white keys", t.key_line, t.key_white),
            ("ruler ticks", t.ruler_tick, t.ruler_bg),
            ("green meter", t.meter_green, t.strip_bg),
            ("yellow meter", t.meter_yellow, t.strip_bg),
            ("red meter", t.meter_red, t.strip_bg),
        ] {
            let c = contrast(fg, bg);
            assert!(c >= 3.0, "{name}: {c:.2}");
        }
    }

    /// Text on track colours: black or white by contrast. A very dark own colour gets white, a
    /// very light one black, and every colour keeps at least 4.5:1.
    #[test]
    fn text_on_track_colours_is_readable() {
        assert_eq!(text_on(Color32::from_rgb(0x12, 0x10, 0x24)), Color32::WHITE);
        assert_eq!(text_on(Color32::from_rgb(0xf4, 0xf0, 0xd8)), Color32::BLACK);
        let presets = studiocraft_project::TRACK_COLORS
            .iter()
            .map(|c| Color32::from_rgb(c[0], c[1], c[2]));
        let greys = (0..=255).step_by(5).map(|v| Color32::from_gray(v as u8));
        for bg in presets.chain(greys) {
            let c = contrast(text_on(bg), bg);
            assert!(c >= 4.5, "{bg:?}: {c:.2}");
        }
    }

    /// The neutrals are the brand's (ONEFOLD-LABS/brand `tokens/tokens.json`).
    #[test]
    fn surfaces_and_text_are_the_brand_neutrals() {
        assert_eq!(Tokens::DARK.panel_bg, rgb(0x171d20));
        assert_eq!(Tokens::DARK.toolbar_bg, rgb(0x101416));
        assert_eq!(Tokens::DARK.window_bg, rgb(0x0b0e10));
        assert_eq!(Tokens::DARK.text, rgb(0xeef2f1));
    }

    /// egui draws `strong` text (headings in lists and dialogs) in the pressed widget's text
    /// colour: it must be readable on the panels.
    #[test]
    fn strong_text_is_readable() {
        let t = Tokens::DARK;
        let v = t.visuals();
        for bg in [t.panel_bg, t.panel_bg2] {
            let c = contrast(v.strong_text_color(), bg);
            assert!(c >= 4.5, "strong text: {c:.2}");
        }
    }

    #[test]
    fn install_is_always_dark() {
        let ctx = Context::default();
        // Even a system that prefers light shows StudioCraft's dark visuals.
        ctx.set_theme(ThemePreference::Light);
        install(&ctx);
        assert_eq!(ctx.theme(), Theme::Dark);
        assert_eq!(tokens(&ctx).panel_bg, Tokens::DARK.panel_bg);
        assert_eq!(
            ctx.style_of(Theme::Light).visuals.panel_fill,
            Tokens::DARK.panel_bg
        );
    }
}
