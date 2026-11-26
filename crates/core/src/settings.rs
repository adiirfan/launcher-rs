//! The owner's choices from the settings screen, and their lines in the
//! layout file (`set <name> <value>`).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Blue,
    Purple,
    Green,
    Orange,
}

impl Accent {
    pub const ALL: [Accent; 4] = [Accent::Blue, Accent::Purple, Accent::Green, Accent::Orange];
    pub fn rgb(self) -> u32 {
        match self {
            Accent::Blue => 0x33B5E5,
            Accent::Purple => 0xAA66CC,
            Accent::Green => 0x99CC00,
            Accent::Orange => 0xFFBB33,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Accent::Blue => "Holo blue",
            Accent::Purple => "Purple",
            Accent::Green => "Green",
            Accent::Orange => "Orange",
        }
    }

    /// Its word in a settings file.
    pub fn word(self) -> &'static str {
        match self {
            Accent::Blue => "blue",
            Accent::Purple => "purple",
            Accent::Green => "green",
            Accent::Orange => "orange",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Quick,
    Smooth,
    Off,
}

impl Speed {
    /// How much longer than today's slides (0: no slide at all).
    pub fn factor(self) -> f32 {
        match self {
            Speed::Quick => 1.0,
            Speed::Smooth => 1.6,
            Speed::Off => 0.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Speed::Quick => "Quick",
            Speed::Smooth => "Smooth",
            Speed::Off => "Off",
        }
    }

    pub fn next(self) -> Speed {
        match self {
            Speed::Quick => Speed::Smooth,
            Speed::Smooth => Speed::Off,
            Speed::Off => Speed::Quick,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Speed::Quick => "quick",
            Speed::Smooth => "smooth",
            Speed::Off => "off",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub accent: Accent,
    /// Names under icons on Home.
    pub labels: bool,
    /// Newly installed apps also go on Home.
    pub add_new_apps: bool,
    pub dots: bool,
    pub animations: Speed,
    /// A glow at the end of a list pushed past its end.
    pub glow: bool,
    /// The next calendar event under the date.
    pub events: bool,
    /// Music controls under the date while something plays.
    pub music: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { accent: Accent::Blue, labels: true, add_new_apps: false, dots: true, animations: Speed::Quick, glow: true, events: false, music: true }
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn parse_on_off(v: &str) -> Option<bool> {
    match v {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    pub fn to_text(&self) -> String {
        format!(
            "set accent {}\nset labels {}\nset add_new_apps {}\nset dots {}\nset animations {}\nset glow {}\nset events {}\nset music {}\n",
            self.accent.word(),
            on_off(self.labels),
            on_off(self.add_new_apps),
            on_off(self.dots),
            self.animations.word(),
            on_off(self.glow),
            on_off(self.events),
            on_off(self.music),
        )
    }

    /// Reads the `set` lines of a layout file. Anything missing, unknown
    /// or unreadable keeps its default.
    pub fn from_text(s: &str) -> Settings {
        let mut out = Settings::default();
        for line in s.lines() {
            let mut parts = line.split_whitespace();
            let (Some("set"), Some(name), Some(value), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else { continue };
            match name {
                "accent" => {
                    if let Some(a) = Accent::ALL.into_iter().find(|a| a.word() == value) {
                        out.accent = a;
                    }
                }
                "animations" => {
                    if let Some(sp) = [Speed::Quick, Speed::Smooth, Speed::Off].into_iter().find(|sp| sp.word() == value) {
                        out.animations = sp;
                    }
                }
                "labels" => out.labels = parse_on_off(value).unwrap_or(out.labels),
                "add_new_apps" => out.add_new_apps = parse_on_off(value).unwrap_or(out.add_new_apps),
                "dots" => out.dots = parse_on_off(value).unwrap_or(out.dots),
                "glow" => out.glow = parse_on_off(value).unwrap_or(out.glow),
                "events" => out.events = parse_on_off(value).unwrap_or(out.events),
                "music" => out.music = parse_on_off(value).unwrap_or(out.music),
                _ => {}
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = Settings::default();
        assert_eq!(s.accent, Accent::Blue);
        assert!(s.labels && !s.add_new_apps && s.dots && s.glow);
        assert!(!s.events && s.music);
        assert_eq!(s.animations, Speed::Quick);
        assert_eq!(Accent::Blue.rgb(), 0x33B5E5);
        assert_eq!(Accent::Purple.rgb(), 0xAA66CC);
        assert_eq!(Accent::Green.rgb(), 0x99CC00);
        assert_eq!(Accent::Orange.rgb(), 0xFFBB33);
        assert_eq!(Accent::Blue.name(), "Holo blue");
    }

    #[test]
    fn round_trip_every_field() {
        let s = Settings { accent: Accent::Orange, labels: false, add_new_apps: true, dots: false, animations: Speed::Off, glow: false, events: true, music: false };
        let text = format!("v1\ngrid 4 4\n{}slot 0 0 0 a/A\n", s.to_text());
        assert_eq!(Settings::from_text(&text), s);
    }

    #[test]
    fn old_files_and_bad_lines_give_defaults() {
        assert_eq!(Settings::from_text("v1\ngrid 4 4\n"), Settings::default());
        let s = Settings::from_text("set accent pink\nset wobble yes\nset labels maybe\nset glow off\n");
        assert_eq!(s, Settings { glow: false, ..Settings::default() });
    }

    #[test]
    fn speeds() {
        assert_eq!(Speed::Quick.factor(), 1.0);
        assert_eq!(Speed::Smooth.factor(), 1.6);
        assert_eq!(Speed::Off.factor(), 0.0);
        assert_eq!(Speed::Quick.next(), Speed::Smooth);
        assert_eq!(Speed::Smooth.next(), Speed::Off);
        assert_eq!(Speed::Off.next(), Speed::Quick);
    }
}
