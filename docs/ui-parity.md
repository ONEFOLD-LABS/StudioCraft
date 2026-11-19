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
| 1 | Menüzeile | Datei, Bearbeiten, Projekt, Ansicht, … | ✅ |
| 2 | Werkzeugzeile | Zonen-Schalter links, Transport-Kurzform, Werkzeuge, Snap/Raster, Zonen-Schalter rechts | ✅ |
| 3 | Info-Zeile | Werte des gewählten Events, direkt editierbar; ein-/ausblendbar | ✅ (Mehrfachauswahl relativ: offen) |
| 4 | Mitte | linke Zone · Spurliste · Lineal/Event-Anzeige · rechte Zone | ✅ |
| 5 | Untere Zone | Reiter (Mixer, Editor, …) am unteren Rand der Zone | ✅ |
| 6 | Transportleiste | fest am unteren Fensterrand, F2 blendet aus | ✅ |
| 7 | Statuszeile | Audio-Gerät, Abtastrate, Last, Meldungen | 🟡 (in Transportleiste) |

## Werkzeugzeile

Eine Zeile, Gruppen durch schmale Trenner getrennt, Knöpfe quadratisch mit Symbol und Tooltip
(Name und Taste). Reihenfolge von links:

1. **Zonen-Schalter links:** linke Zone (Inspector, Alt+I), untere Zone, rechte Zone (Alt+R).
   Hervorgehoben, wenn die Zone sichtbar ist.
2. **Transport-Kurzform:** Zum Anfang, Stopp, Start, Aufnahme, Cycle (gleiches Verhalten wie die
   Transportleiste).
3. **Werkzeuge 1–0** als Symbolknöpfe, aktiver Knopf in Akzentfarbe hinterlegt:
   Pfeil (Auswahl), Pfeil mit Rahmen (Bereich), Schere (Trennen), Tube/Klebstoff (Kleben),
   Radierer (Löschen), Lupe (Zoom), X (Stumm), Stift (Zeichnen), Lautsprecher (Wiedergabe),
   Metronom-ähnliche Uhr (Zeitkorrektur).
4. **Snap-Gruppe:** Snap an/aus (Magnet-Symbol, J), Snap-Modus, Raster-Typ, Quantisierungswert.
5. **Rechts:** Zonen-Schalter für die rechte Zone.

## Info-Zeile

Direkt unter der Werkzeugzeile, eine Zeile mit beschrifteten Feldern. Ohne Auswahl ein grauer
Hinweis „Keine Auswahl“. Bei einem Event: Name, Anfang, Ende, Länge, Versatz, Lautstärke,
Fade-In, Fade-Out, Stumm. Zeitwerte im Format des Lineals (Takt.Zählzeit.Sechzehntel.Ticks).
Bei Mehrfachauswahl gelb hervorgehobene Felder, Änderungen relativ (später).

## Spurliste

- Kopfzeile über den Spuren: Anzahl Spuren, Knopf „Spur hinzufügen“ (+), globale
  Mute-/Solo-Anzeige (zeigt, ob irgendwo Mute oder Solo aktiv ist; Klick hebt auf).
- **Spurkopf** (Spec „Spurtypen und Inspector“): links eine Farbleiste in Spurfarbe über die
  ganze Höhe, daneben Name; die Schalter sitzen in einer Reihe: Mute (M), Solo (S), Aufnahme
  bereit (●), Monitor (Lautsprecher), Automation lesen (R), Automation schreiben (W).
  Aktive Farben: Mute orange, Solo gelb, Aufnahme rot, Monitor grün, R grün, W rot.
- Gewählte Spur: Kopf heller hinterlegt, nicht in Akzentfarbe vollflächig.

## Event-Anzeige und Lineal

- Lineal über der Event-Anzeige: Taktzahlen, Cycle-Bereich zwischen den Locatoren als Balken
  (aktiv: Akzentfarbe, inaktiv: grau), Projekt-Cursor als senkrechte Linie.
- Events: Kopfzeile mit Name in der Spurfarbe, Inhalt (Wellenform/Noten) darunter.

## Linke Zone (Inspector)

Kopf mit Spurname und Spurfarbe, darunter aufklappbare Sektionen; die oberste Sektion ohne Titel
enthält Lautstärke, Pan, M/S/R/Monitor, Eingang, Ausgang. Danach Inserts, EQ, Sends, Notizen
(Spec „Spurtypen und Inspector“).

## Untere Zone

Inhalt oben, eine schmale Reiterleiste am **unteren** Rand der Zone: Mixer, Editor, Sample-Editor
(später). Reiter mit Text, aktiver Reiter hervorgehoben.

## Rechte Zone

Reiter oben: Medien, Plugins (Issue #27). Ein-/ausblendbar über Alt+R und den Zonen-Schalter.

## Transportleiste

Gruppen von links nach rechts, durch Trenner getrennt:

1. Leistung/Status (Audio-Last, Gerät)
2. Locatoren: L und R mit Position, Punch-In, Cycle, Punch-Out dazwischen
3. Hauptanzeige: Position in Takten groß, darunter/daneben Zeit
4. Transportknöpfe: Zum Anfang, Zurückspulen, Vorspulen, Stopp, Start, Aufnahme, Cycle
5. Tempo und Taktart, Metronom (Klick), Vorzähler

Die Positionsanzeige hat einen dunklen Hintergrund (`counter_bg`) mit heller Schrift.

## Farben

- Neutrale Flächen in reinem Grau (kaum Farbstich), Zonen durch etwas hellere oder dunklere
  Flächen und 1-px-Ränder getrennt.
- Weinrot nur für: aktives Werkzeug, aktive Reiter, Auswahlrahmen, Cycle-Bereich, Fokusrahmen.
  Nicht für ganze Zeilen- oder Kanalhintergründe.
- Statusfarben (Mute, Solo, Aufnahme, Monitor) bleiben eindeutig und unabhängig vom Akzent.

## Reihenfolge der Umsetzung

1. ✅ Werkzeugzeile mit eigenen Symbolen und Zonen-Schaltern (#62)
2. ✅ Info-Zeile (#63)
3. ✅ Spurköpfe: Aufnahme-Punkt und neutrale Auswahl (#64), Monitor und R/W (Punkt 10),
   Zeitbasis und Lanes (Punkt 12); R/W wirken mit der Automation (#34)
4. ✅ Transportleiste (#64) und Reiter der unteren Zone (#65)
5. ✅ Farben: neutrale Grautöne, Akzent sparsamer (#65)
6. 🟡 Rechte Zone: Medien und Plugins (#67); Dateibrowser und Vorhören folgen (Issue #27)
7. ✅ Inspector-Kopf in Spurfarbe (#69)
8. ✅ Quantisierungswert in der Werkzeugzeile, Taste Q, Panel für Swing und Stärke (#21)
9. ✅ Snap-Modi, Raster-Typen Sekunden/Frames/Samples, „An Zoom anpassen“ (#12)
10. ✅ Spurkopf und Inspector: Monitor (Eingang mithören) und Automation R/W (#19)
11. ✅ Fokusrahmen: Klick gibt einer Zone den Fokus, schmaler Rahmen in Akzentfarbe
12. ✅ Zeitbasis (musikalisch/linear) und Lanes ein/aus in Spurkopf und Inspector (#19)

**Als Nächstes:** Rasterlinien für feinere Raster, Automation (#34), Comping in den Lanes.
