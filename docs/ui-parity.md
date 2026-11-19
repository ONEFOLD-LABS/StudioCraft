# UI-Parität: Anordnung des Projektfensters

Stand: 2026-11-19 (Status nach #62–#69)

Ziel: Wer klassische DAWs kennt, findet sich in StudioCraft ohne Suchen zurecht. Dazu folgt das
Projektfenster der Anordnung, die `docs/spec.md` (Abschnitt „Fenster-Architektur“) beschreibt,
Element für Element. Diese Datei hält fest, **wo** welches Element sitzt und **wie** es aussieht;
das Verhalten steht in der Spec.

**Quellen:** nur öffentliches Material (Handbuch, Produktseiten, Videos), in eigenen Worten
zusammengefasst. Keine Screenshots, Icons, Farben oder Texte aus klassischen DAWs im Repo; Icons zeichnet
StudioCraft selbst im Code (`crates/ui/src/icons.rs`). Farben sind eigene: neutrale Grautöne,
Weinrot (`theme::APP_COLOR`) als Akzent, sparsam eingesetzt.

Status: ✅ umgesetzt · 🟡 teilweise · ⬜ offen

## Gesamtaufbau (von oben nach unten)

| # | Element | Inhalt | Status |
|---|---|---|---|
| 1 | Titelleiste | App-Symbol, Menü (Datei, Bearbeiten, Projekt, Ansicht), Projekt-Titel, Fensterknöpfe; `craft-window` entscheidet pro Desktop, ob die App sie zeichnet | ✅ |
| 2 | Werkzeugzeile | Zonen-Schalter links, Transport-Kurzform, Werkzeuge, Snap/Raster, Zonen-Schalter rechts | ✅ |
| 3 | Info-Zeile | Werte des gewählten Events, direkt editierbar; ein-/ausblendbar; Mehrfachauswahl relativ | ✅ |
| 4 | Mitte | linke Zone · Spurliste · Lineal/Event-Anzeige · rechte Zone | ✅ |
| 5 | Untere Zone | Reiter (Mixer, Editor, …) am unteren Rand der Zone | ✅ |
| 6 | Transportleiste | fest am unteren Fensterrand, F2 blendet aus | ✅ |
| 7 | Statuszeile | Audio-Gerät, Abtastrate, Eingang, MIDI, Meldungen, Audio-Last, Bildzeit; Ansicht → Statuszeile | ✅ |

## Werkzeugzeile

Eine Zeile, Gruppen durch schmale Trenner getrennt, Knöpfe quadratisch mit Symbol und Tooltip
(Name und Taste). Reihenfolge von links:

1. **Zonen-Schalter links:** linke Zone (Inspector, Alt+I), untere Zone, rechte Zone (Alt+R).
   Hervorgehoben, wenn die Zone sichtbar ist.
2. **Transport-Kurzform:** Zum Anfang, Stopp, Start, Aufnahme, Cycle (gleiches Verhalten wie die
   Transportleiste).
3. **Werkzeuge 1–0** als Symbolknöpfe, aktiver Knopf in Akzentfarbe hinterlegt:
   Pfeil (Auswahl), Pfeil mit Rahmen (Bereich), Comping (ohne Taste, Lanes mit hervorgehobenem
   Abschnitt), Schere (Trennen), Tube/Klebstoff (Kleben), Radierer (Löschen), Lupe (Zoom),
   X (Stumm), Stift (Zeichnen), Lautsprecher (Wiedergabe), Metronom-ähnliche Uhr (Zeitkorrektur).
4. **Snap-Gruppe:** Snap an/aus (Magnet-Symbol, J), Snap-Modus, Raster-Typ, Quantisierungswert,
   Knopf für das Quantisierungs-Panel (Swing, Stärke).
5. **Rechts:** Zonen-Schalter für die rechte Zone.

## Info-Zeile

Direkt unter der Werkzeugzeile, eine Zeile mit beschrifteten Feldern. Ohne Auswahl ein grauer
Hinweis „Keine Auswahl“. Bei einem Event: Name, Anfang, Ende, Länge, Versatz, Lautstärke,
Fade-In, Fade-Out, Stumm. Zeitwerte im Format des Lineals (Takt.Zählzeit.Sechzehntel.Ticks).
Bei Mehrfachauswahl: Werte des frühesten Events, Beschriftungen gelb, davor die Anzahl; Zeitwerte
ändern alle relativ, Name und Stumm gelten für alle; ein Rückgängig-Schritt je Änderung.

## Spurliste

- Kopfzeile über den Spuren: Anzahl Spuren, Knopf „Spur hinzufügen“ (+), globale
  Mute-/Solo-Anzeige (zeigt, ob irgendwo Mute oder Solo aktiv ist; Klick hebt auf).
- **Spurkopf** (Spec „Spurtypen und Inspector“): links eine Farbleiste in Spurfarbe über die
  ganze Höhe (Klick öffnet die Farbpalette), davor der Pfeil, der die Automationsspuren auf- und zuklappt, daneben Name; die
  Schalter sitzen in einer Reihe: Mute (M), Solo (S), Aufnahme bereit (●), Monitor
  (Lautsprecher), Automation lesen (R), Automation schreiben (W), Zeitbasis (Note
  musikalisch, Uhr linear), Lanes ein/aus.
  Aktive Farben: Mute orange, Solo gelb, Aufnahme rot, Monitor grün, R grün, W rot.
- **Lanes:** aufgeklappt liegen überlappende Takes untereinander; verdeckte Abschnitte sind
  abgedunkelt, das Comp-Werkzeug macht einen Abschnitt hörbar.
- **Automationsspuren** unter ihrer Spur (und unter den Lanes), je eine Zeile: Kopf mit
  Parametername, Mute und Entfernen; Kurve in Spurfarbe, gedimmt bei Mute oder ohne R.
- Gewählte Spur: Kopf heller hinterlegt, nicht in Akzentfarbe vollflächig.

## Event-Anzeige und Lineal

- Lineal über der Event-Anzeige: Taktzahlen, Cycle-Bereich zwischen den Locatoren als Balken
  (aktiv: Akzentfarbe, inaktiv: grau), Projekt-Cursor als senkrechte Linie.
- Events: Kopfzeile mit Name in der Spurfarbe, Inhalt (Wellenform/Noten) darunter.
- Rasterlinien: Takte kräftig, Zählzeiten schwächer, das feinere Snap-Raster (1/16, Frames,
  „An Zoom anpassen“) als zarte Linien, sobald die Abstände mindestens 6 px betragen.

## Linke Zone (Inspector)

Kopf mit Spurname und Spurfarbe, darunter aufklappbare Sektionen; die oberste Sektion ohne Titel
enthält Lautstärke, Pan, M/S, Aufnahme, Monitor, R/W, Zeitbasis, Lanes, Eingang, Ausgang. Danach
Inserts, Sends, Automation (Spuren zeigen, stummschalten, entfernen, „+ Parameter“ für Kanal,
Sends und Plugin-Parameter, Ausdünnen), EQ, Notizen (Spec „Spurtypen und Inspector“).

## Untere Zone

Inhalt oben, eine schmale Reiterleiste am **unteren** Rand der Zone: Mixer, Editor, Sample-Editor
(später). Reiter mit Text, aktiver Reiter hervorgehoben.

## Rechte Zone

Reiter oben: Medien (Pool), Dateien (Dateibrowser: Ordner öffnen, ein Klick hört vor, Doppelklick
oder Ziehen legt die Datei ins Projekt), Plugins (Doppelklick oder Ziehen: Instrument auf eine
leere Fläche gibt eine neue Instrument-Spur, auf eine Instrument-Spur ersetzt es deren Instrument;
Effekt auf eine Spur wird Insert) (Issue #27). Ein-/ausblendbar über Alt+R und den
Zonen-Schalter.

## Transportleiste

Gruppen von links nach rechts, durch Trenner getrennt:

1. Locatoren: L und R mit Position, Punch-In, Cycle, Punch-Out dazwischen
2. Hauptanzeige: Position in Takten groß, darunter/daneben Zeit
3. Transportknöpfe: Zum Anfang, Zurückspulen, Vorspulen, Stopp, Start, Aufnahme, Cycle
4. Automationsmodus „Auto“: Touch, Auto-Latch, Overwrite, Trim; rot während geschrieben wird
5. Tempo und Taktart, Metronom (Klick), Vorzähler

Gerät, Last und Meldungen stehen in der Statuszeile darunter; ist sie ausgeblendet, erscheinen
Meldungen rechts in der Transportleiste.

Die Positionsanzeige hat einen dunklen Hintergrund (`counter_bg`) mit heller Schrift.

## Farben

- Neutrale Flächen in reinem Grau (kaum Farbstich), Zonen durch etwas hellere oder dunklere
  Flächen und 1-px-Ränder getrennt.
- Weinrot nur für: aktives Werkzeug, aktive Reiter, Auswahlrahmen, Cycle-Bereich, Fokusrahmen.
  Nicht für ganze Zeilen- oder Kanalhintergründe.
- Statusfarben (Mute, Solo, Aufnahme, Monitor) bleiben eindeutig und unabhängig vom Akzent.

## Reihenfolge der Umsetzung

1. ✅ Werkzeugzeile mit eigenen Symbolen und Zonen-Schaltern (#62)
2. ✅ Info-Zeile (#63), Mehrfachauswahl mit relativen Änderungen
3. ✅ Spurköpfe: Aufnahme-Punkt und neutrale Auswahl (#64), Monitor und R/W (#73), Zeitbasis
   und Lanes (#75)
4. ✅ Transportleiste (#64) und Reiter der unteren Zone (#65)
5. ✅ Farben: neutrale Grautöne, Akzent sparsamer (#65)
6. ✅ Rechte Zone: Medien und Plugins (#67), Dateibrowser mit Vorhören (#86), Dateien und Plugins
   ins Projekt ziehen (Issue #27)
7. ✅ Inspector-Kopf in Spurfarbe (#69)
8. ✅ Quantisierungswert in der Werkzeugzeile, Taste Q, Panel für Swing und Stärke (#71, Issue #21)
9. ✅ Snap-Modi, Raster-Typen Sekunden/Frames/Samples, „An Zoom anpassen“ (#72, Issue #12)
10. ✅ Monitor (Eingang mithören) und Automation R/W in Spurkopf und Inspector (#73, Issue #19)
11. ✅ Fokusrahmen: Klick gibt einer Zone den Fokus, schmaler Rahmen in Akzentfarbe (#74)
12. ✅ Zeitbasis (musikalisch/linear) und Lanes ein/aus in Spurkopf und Inspector (#75, Issue #19)
13. ✅ Feine Rasterlinien in Event-Anzeige und Key-Editor (#76)
14. ✅ Automation (Issue #34): Kurven abspielen mit R (#77), unter der Spur bearbeiten (#78),
    schreiben mit W in Touch und Auto-Latch (#81), Sends (#83), Overwrite und Trim
15. ✅ Statusanzeige der Transportleiste läuft nicht mehr über Tempo und Takt (#79)
16. ✅ Titelleiste wie in den Craft-Apps: Symbol, Menü, Titel und Fensterknöpfe in einer Zeile,
    `craft-window` entscheidet pro Desktop (#80)
17. ✅ Comping: Comp-Werkzeug, Abschnitte eines Takes per Klick oder Ziehen hörbar machen (#82)
18. ✅ Farben: Klick auf die Farbleiste im Spurkopf öffnet die Palette; Events folgen der Spurfarbe,
    über Bearbeiten → Event-Farbe bekommen sie eine eigene (Issue #19)

**Als Nächstes:** Aus dieser Liste ist nichts mehr offen; die Automationsmodi Overwrite und Trim
kommen mit #91. Weitere Angleichungen ergeben sich aus `docs/spec.md`.
