//! Offline-Bearbeitung pro Clip, nicht-destruktiv (Spec „Editoren“ → Sample-Editor;
//! Issue #53).
//!
//! Eine Bearbeitung (Normalisieren, Gain, Umkehren …) erzeugt einen neuen Clip im Pool; das
//! Event zeigt darauf. Welcher Clip woraus entstand, steht in [`Project::derived`]: der
//! Original-Clip und die Liste der Bearbeitungen in Reihenfolge. Die Liste lässt sich jederzeit
//! ändern (Einträge an/aus, umsortieren, entfernen); dann wird aus dem Original neu gerendert.
//! Das Rendern selbst macht die Oberfläche mit `studiocraft_engine::offline::apply`.

use crate::{ClipId, Project};

/// Höchstens so viele Bearbeitungen je Clip.
pub const MAX_PROCESSES: usize = 64;

/// Art einer Bearbeitung.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProcessKind {
    /// Spitze auf `target_db` heben (0 = Vollaussteuerung).
    Normalize {
        target_db: f32,
    },
    /// Um `db` verstärken.
    Gain {
        db: f32,
    },
    /// Stille.
    Silence,
    /// Zeitlich umkehren.
    Reverse,
    /// Phase drehen (Vorzeichen).
    InvertPhase,
    /// Linear ein- bzw. ausblenden.
    FadeIn,
    FadeOut,
    /// Gleichanteil entfernen.
    RemoveDc,
    /// Ein Effekt-Plugin offline (Plugin und Einstellungen in [`ClipProcess::plugin`]).
    Plugin,
}

impl ProcessKind {
    /// Deutscher Name (die Oberfläche übersetzt ihn).
    pub fn label(self) -> &'static str {
        match self {
            Self::Normalize { .. } => "Normalisieren",
            Self::Gain { .. } => "Gain",
            Self::Silence => "Stille",
            Self::Reverse => "Umkehren",
            Self::InvertPhase => "Phase drehen",
            Self::FadeIn => "Fade-In",
            Self::FadeOut => "Fade-Out",
            Self::RemoveDc => "DC-Offset entfernen",
            Self::Plugin => "Plugin",
        }
    }

    /// In gültige Grenzen (Werte aus Projektdateien sind fremde Eingabe).
    fn sanitized(self) -> Self {
        let db = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        match self {
            Self::Normalize { target_db } => Self::Normalize {
                target_db: db(target_db, -48.0, 0.0, 0.0),
            },
            Self::Gain { db: g } => Self::Gain {
                db: db(g, -48.0, 24.0, 0.0),
            },
            other => other,
        }
    }
}

/// Eine Bearbeitung im Frame-Bereich `from..to` des Original-Clips.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClipProcess {
    #[serde(flatten)]
    pub kind: ProcessKind,
    pub from: u64,
    pub to: u64,
    /// Ausgeschaltet: bleibt in der Liste, wirkt aber nicht.
    #[serde(default = "on", skip_serializing_if = "is_on")]
    pub on: bool,
    /// Bei [`ProcessKind::Plugin`]: das Plugin mit Parametern und Zustand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin: Option<crate::InsertSlot>,
}

impl ClipProcess {
    /// Eine eingebaute Bearbeitung im Bereich `from..to`.
    pub fn new(kind: ProcessKind, from: u64, to: u64) -> Self {
        Self {
            kind,
            from: from.min(to),
            to: from.max(to),
            on: true,
            plugin: None,
        }
    }

    /// Name für Listen: bei Plugins der Plugin-Name.
    pub fn name(&self) -> &str {
        match (&self.kind, &self.plugin) {
            (ProcessKind::Plugin, Some(p)) => &p.name,
            (k, _) => k.label(),
        }
    }
}

fn on() -> bool {
    true
}

fn is_on(v: &bool) -> bool {
    *v
}

/// Herkunft eines bearbeiteten Clips: das Original und die Bearbeitungen in Reihenfolge.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Derived {
    pub source: ClipId,
    pub steps: Vec<ClipProcess>,
}

impl Project {
    /// Original-Clip und Bearbeitungen eines Clips: für ein Original dieses selbst und keine.
    pub fn clip_origin(&self, clip: ClipId) -> (ClipId, Vec<ClipProcess>) {
        match self.derived.get(&clip) {
            Some(d) => (d.source, d.steps.clone()),
            None => (clip, Vec::new()),
        }
    }

    /// Hält fest, dass `clip` aus `source` mit `steps` entstand. Ist `source` selbst
    /// abgeleitet, zählt dessen Original (Ketten werden flach).
    pub fn set_derived(&mut self, clip: ClipId, source: ClipId, steps: Vec<ClipProcess>) {
        let (root, mut all) = self.clip_origin(source);
        all.extend(steps);
        all.truncate(MAX_PROCESSES);
        self.derived.insert(
            clip,
            Derived {
                source: root,
                steps: all,
            },
        );
    }

    /// Verweise auf Clips, die es nicht gibt, fallen weg; Werte und Längen werden begrenzt.
    /// Gibt zurück, ob sich etwas geändert hat.
    pub(crate) fn sanitize_derived(&mut self) -> bool {
        let n = self.pool.len();
        let before = self.derived.clone();
        self.derived
            .retain(|&clip, d| (clip as usize) < n && (d.source as usize) < n && d.source != clip);
        for (&clip, d) in &mut self.derived {
            let frames = self.pool.get(d.source as usize).map_or(0, |c| c.frames);
            let _ = clip;
            d.steps.truncate(MAX_PROCESSES);
            for s in &mut d.steps {
                s.kind = s.kind.sanitized();
                s.to = s.to.min(frames);
                s.from = s.from.min(s.to);
            }
        }
        self.derived != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PoolClip;

    fn clip(p: &mut Project, frames: u64) -> ClipId {
        p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames,
        })
    }

    fn step(kind: ProcessKind) -> ClipProcess {
        ClipProcess::new(kind, 0, 100)
    }

    #[test]
    fn chains_stay_flat_and_survive_save_and_load() {
        let mut p = Project::default();
        let a = clip(&mut p, 1000);
        let b = clip(&mut p, 1000);
        let c = clip(&mut p, 1000);
        p.set_derived(b, a, vec![step(ProcessKind::Reverse)]);
        p.set_derived(c, b, vec![step(ProcessKind::Gain { db: -3.0 })]);
        let (src, steps) = p.clip_origin(c);
        assert_eq!(src, a);
        assert_eq!(
            steps.iter().map(|s| s.kind).collect::<Vec<_>>(),
            [ProcessKind::Reverse, ProcessKind::Gain { db: -3.0 }]
        );
        assert_eq!(p.clip_origin(a), (a, Vec::new()));
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"kind\":\"reverse\""), "{json}");
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.derived, p.derived);
    }

    #[test]
    fn sanitize_drops_broken_links_and_clamps_values() {
        let mut p = Project::default();
        let a = clip(&mut p, 50);
        let b = clip(&mut p, 50);
        p.derived.insert(
            b,
            Derived {
                source: a,
                steps: vec![ClipProcess {
                    from: 80,
                    to: 1000,
                    ..ClipProcess::new(ProcessKind::Gain { db: f32::NAN }, 0, 0)
                }],
            },
        );
        p.derived.insert(
            7,
            Derived {
                source: a,
                steps: Vec::new(),
            },
        );
        p.derived.insert(
            a,
            Derived {
                source: a,
                steps: Vec::new(),
            },
        );
        assert!(p.sanitize_derived());
        assert_eq!(p.derived.len(), 1);
        let s = p.derived[&b].steps[0].clone();
        assert_eq!(s.kind, ProcessKind::Gain { db: 0.0 });
        assert_eq!((s.from, s.to), (50, 50));
        assert!(!p.sanitize_derived());
    }

    #[test]
    fn plugin_steps_keep_their_settings_through_save_and_load() {
        let mut p = Project::default();
        let a = clip(&mut p, 1000);
        let b = clip(&mut p, 1000);
        let slot = crate::InsertSlot {
            plugin_id: "org.example.delay".into(),
            name: "Delay".into(),
            bypass: false,
            params: vec![(3, 0.25)],
            state: Some("AAEC".into()),
        };
        let step = ClipProcess {
            plugin: Some(slot.clone()),
            ..ClipProcess::new(ProcessKind::Plugin, 0, 500)
        };
        assert_eq!(step.name(), "Delay");
        p.set_derived(b, a, vec![step]);
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.derived[&b].steps[0].plugin, Some(slot));
    }
}
