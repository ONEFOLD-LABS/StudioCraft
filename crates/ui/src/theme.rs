//! StudioCraft's look: design tokens for dark and light, egui visuals built from them, and the
//! choice between Dark, Light and System.
//!
//! The token set and most dark/light values follow SoundCraft's DAW theme
//! (`crates/ui-egui/src/theme.rs` in storytold/soundcraft, MIT OR Apache-2.0), with plain neutral
//! greys and wine red (`APP_COLOR`) as a sparing accent (`docs/ui-parity.md`, „Farben“). Because
//! a DAW already uses red for recording, `rec` is a brighter orange red and the playhead is
//! neutral, so the accent and a recording are never mistaken for one another.
//!
//! Use: call [`install`] once at start-up with the saved [`ThemePreference`] (egui then follows the
//! system for `System`), and read colours with [`tokens`] wherever the UI paints.

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
    pub event_text: Color32,
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
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        window_bg: rgb(0x1e1e1e),
        panel_bg: rgb(0x2a2a2a),
        panel_bg2: rgb(0x323232),
        toolbar_bg: rgb(0x242424),
        border: rgb(0x121212),
        border_light: rgb(0x454545),
        text: rgb(0xd8d8d8),
        text_dim: rgb(0x9a9a9a),
        text_on_accent: rgb(0xffffff),
        accent: rgb(0xc8466a),
        accent_dark: rgb(0x9e2343),
        button: rgb(0x3c3c3c),
        button_hi: rgb(0x4e4e4e),
        ruler_bg: rgb(0x2e2e2e),
        ruler_text: rgb(0xb4b4b4),
        ruler_tick: rgb(0x626262),
        lane_bg: rgb(0x262626),
        lane_alt: rgb(0x2a2a2a),
        grid_line: rgb(0x363636),
        bar_line: rgb(0x474747),
        selection: rgba(0xc8466a, 77),
        row_selected: rgb(0x383838),
        cycle_fill: rgba(0xc8466a, 64),
        punch_fill: rgba(0xf0452e, 77),
        playhead: rgb(0xf2e8ea),
        event_text: rgb(0x0a0a0a),
        event_text_muted: rgb(0xc8c8c8),
        event_muted: rgb(0x777777),
        event_shade: rgb(0x121212),
        event_selected: rgb(0xf2e8ea),
        handle: rgb(0xffffff),
        rec: rgb(0xf0452e),
        solo: rgb(0xdec434),
        mute: rgb(0xe89428),
        monitor: rgb(0x46aa5a),
        counter_bg: rgb(0x0c0c0c),
        counter_text: rgb(0xef9fb4),
        meter_green: rgb(0x3cc850),
        meter_yellow: rgb(0xe6d23c),
        meter_red: rgb(0xf0452e),
        fader_track: rgb(0x0c0c0c),
        bus_input: rgb(0x7fa37a),
        bus_output: rgb(0x8f97a8),
        strip_bg: rgb(0x303030),
        key_white: rgb(0xe4e4e4),
        key_black: rgb(0x1c1c1c),
        key_line: rgb(0x949494),
        key_pressed: rgb(0xc8466a),
        key_row: rgb(0x5a2a38),
        note_selected: rgb(0xffffff),
    };

    pub const LIGHT: Tokens = Tokens {
        window_bg: rgb(0xbcbcbc),
        panel_bg: rgb(0xcfcfcf),
        panel_bg2: rgb(0xdadada),
        toolbar_bg: rgb(0xc5c5c5),
        border: rgb(0x6a6a6a),
        border_light: rgb(0xa0a0a0),
        text: rgb(0x1a1a1a),
        text_dim: rgb(0x484848),
        text_on_accent: rgb(0xffffff),
        accent: rgb(0x9e2343),
        accent_dark: rgb(0x7c1a34),
        button: rgb(0xdddddd),
        button_hi: rgb(0xcbcbcb),
        ruler_bg: rgb(0xc8c8c8),
        ruler_text: rgb(0x313131),
        ruler_tick: rgb(0x888888),
        lane_bg: rgb(0xd5d5d5),
        lane_alt: rgb(0xcdcdcd),
        grid_line: rgb(0xbcbcbc),
        bar_line: rgb(0xa0a0a0),
        selection: rgba(0x9e2343, 51),
        row_selected: rgb(0xe3e3e3),
        cycle_fill: rgba(0x9e2343, 46),
        punch_fill: rgba(0xd1321c, 56),
        playhead: rgb(0x1b181a),
        event_text: rgb(0x0a0a0a),
        event_text_muted: rgb(0xf4f2f3),
        event_muted: rgb(0x8d8d8d),
        event_shade: rgb(0x383838),
        event_selected: rgb(0x1b181a),
        handle: rgb(0xffffff),
        rec: rgb(0xd1321c),
        solo: rgb(0xb2911a),
        mute: rgb(0xc26a16),
        monitor: rgb(0x24a03a),
        counter_bg: rgb(0xe3e3e3),
        counter_text: rgb(0x7c1a34),
        meter_green: rgb(0x24a03a),
        meter_yellow: rgb(0xc9ad1c),
        meter_red: rgb(0xd1321c),
        fader_track: rgb(0x8c8c8c),
        bus_input: rgb(0x9cc296),
        bus_output: rgb(0xaab2c4),
        strip_bg: rgb(0xc8c8c8),
        key_white: rgb(0xf6f6f6),
        key_black: rgb(0x292929),
        key_line: rgb(0x8c8c8c),
        key_pressed: rgb(0x9e2343),
        key_row: rgb(0xecd3da),
        note_selected: rgb(0x1b181a),
    };

    pub const fn of(theme: Theme) -> &'static Tokens {
        match theme {
            Theme::Dark => &Tokens::DARK,
            Theme::Light => &Tokens::LIGHT,
        }
    }

    /// egui visuals for this token set; `dark` picks egui's matching base.
    pub fn visuals(&self, dark: bool) -> Visuals {
        let mut v = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
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
            (&mut v.widgets.active, self.accent, self.text_on_accent),
            (&mut v.widgets.open, self.button_hi, self.text),
        ] {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.corner_radius = radius;
            w.fg_stroke = Stroke::new(w.fg_stroke.width, fg);
        }
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        v.widgets.active.bg_stroke = Stroke::new(1.0, self.accent_dark);
        v
    }
}

/// Set StudioCraft's dark and light visuals and follow `preference` (System tracks the OS).
pub fn install(ctx: &Context, preference: ThemePreference) {
    ctx.set_visuals_of(Theme::Dark, Tokens::DARK.visuals(true));
    ctx.set_visuals_of(Theme::Light, Tokens::LIGHT.visuals(false));
    ctx.set_theme(preference);
}

/// The tokens for the theme egui is showing right now.
pub fn tokens(ctx: &Context) -> &'static Tokens {
    Tokens::of(ctx.theme())
}

/// German label for the settings menu.
pub fn label(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::Dark => "Dunkel",
        ThemePreference::Light => "Hell",
        ThemePreference::System => "Wie das System",
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
        for t in [Tokens::DARK, Tokens::LIGHT] {
            assert!(
                distance(t.accent, t.rec) > 60,
                "accent {:?} too close to rec {:?}",
                t.accent,
                t.rec
            );
            assert!(distance(t.playhead, t.rec) > 200);
        }
    }

    #[test]
    fn text_contrasts_with_its_panels() {
        for t in [Tokens::DARK, Tokens::LIGHT] {
            assert!(distance(t.text, t.panel_bg) > 300);
            assert!(distance(t.text, t.lane_bg) > 300);
        }
    }

    #[test]
    fn install_follows_the_preference() {
        let ctx = Context::default();
        install(&ctx, ThemePreference::Light);
        assert_eq!(ctx.theme(), Theme::Light);
        assert_eq!(tokens(&ctx).accent, APP_COLOR);
        install(&ctx, ThemePreference::Dark);
        assert_eq!(ctx.theme(), Theme::Dark);
        assert_eq!(tokens(&ctx).panel_bg, Tokens::DARK.panel_bg);
    }
}
