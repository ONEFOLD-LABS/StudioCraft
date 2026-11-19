# UI-Parität: Anordnung des Projektfensters

Stand: 2026-11-19

Ziel: Wer klassische DAWs kennt, findet sich in StudioCraft ohne Suchen zurecht. Dazu folgt das
Projektfenster der Anordnung, die `docs/spec.md` (Abschnitt „Fenster-Architektur“) beschreibt,
Element für Element. Diese Datei hält fest, **wo** welches Element sitzt und **wie** es aussieht;
das Verhalten steht in der Spec.

**Quellen:** nur öffentliches Material (Handbuch, Produktseiten, Videos), in eigenen Worten
zusammengefasst. Keine Screenshots, Icons, Farben, Texte oder Produktnamen anderer Programme im Repo; Icons zeichnet
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
(Name und Taste). Drei Bereiche:

1. **Links: globale Zustände** M, S, R, W. Sie leuchten in der Statusfarbe, sobald irgendeine Spur
   gemutet, solo, auf Lesen oder auf Schreiben steht. Klick hebt Mute, Solo bzw. Schreiben für
   alle Spuren auf; R schaltet Lesen für alle Kanäle ein bzw. aus. Jeder Klick ist ein
   Undo-Schritt. Daneben die **Transport-Kurzform:** Zum Anfang, Stopp, Start, Aufnahme, Cycle
   (gleiches Verhalten wie die Transportleiste).
2. **Mitte, auf die Fensterbreite zentriert** (in schmalen Fenstern direkt im Anschluss):
   **Werkzeuge 1–0** als Symbolknöpfe, aktiver Knopf in Akzentfarbe hinterlegt:
   Pfeil (Auswahl), Pfeil mit Rahmen (Bereich), Comping (ohne Taste, Lanes mit hervorgehobenem
   Abschnitt), Schere (Trennen), Tube/Klebstoff (Kleben), Radierer (Löschen), Lupe (Zoom),
   X (Stumm), Stift (Zeichnen), Lautsprecher (Wiedergabe), Metronom-ähnliche Uhr (Zeitkorrektur).
   Danach die **Snap-Gruppe:** Snap an/aus (Magnet-Symbol, J), Snap-Modus, Raster-Typ,
   Quantisierungswert, Knopf für das Quantisierungs-Panel (Swing, Stärke).
3. **Rechts:** die Zonen-Schalter linke Zone (Alt+L), untere Zone, rechte Zone (Alt+R),
   hervorgehoben, wenn die Zone sichtbar ist.

## Info-Zeile

Direkt unter der Werkzeugzeile, eine Zeile mit beschrifteten Feldern. Ohne Auswahl ein grauer
Hinweis „Keine Auswahl“. Bei einem Event: Name, Anfang, Ende, Länge, Versatz, Lautstärke,
Fade-In, Fade-Out, Stumm. Zeitwerte im Format des Lineals (Takt.Zählzeit.Sechzehntel.Ticks).
Bei Mehrfachauswahl: Werte des frühesten Events, Beschriftungen gelb, davor die Anzahl; Zeitwerte
ändern alle relativ, Name und Stumm gelten für alle; ein Rückgängig-Schritt je Änderung.

## Spurliste

- Kopfzeile über den Spuren: Anzahl Spuren, Knopf „Spur hinzufügen“ (+), globale
  Mute-/Solo-Anzeige (zeigt, ob irgendwo Mute oder Solo aktiv ist; Klick hebt auf).
- **Spurkopf** (Spec „Spurtypen und Inspector“): links eine breite Farbleiste in Spurfarbe über
  die ganze Höhe mit der Spurnummer (Klick öffnet die Farbpalette). Daneben zwei Zeilen:
  oben der Pfeil, der die Automationsspuren auf- und zuklappt, Mute (M), Solo (S), Name und
  rechts ein schmaler Lautstärkeregler in Spurfarbe (Klick oder Ziehen setzt den Wert,
  Strg+Klick 0 dB); unten Aufnahme bereit (●), Monitor (Lautsprecher), Automation lesen (R),
  Automation schreiben (W), Zeitbasis (Note musikalisch, Uhr linear), Lanes ein/aus, rechts
  klein der Spurtyp. Schalter, die ein Spurtyp nicht hat, lassen ihre Lücke, damit die
  Spalten über alle Spuren gleich bleiben.
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

## Linke Zone (Kanalzug und Inspector)

Zwei Spalten. Links ein schmaler **Kanalzug** der gewählten Spur: Name in Spurfarbe, Liste der
Inserts und Sends (umgangene gedimmt), Aufnahme bereit und Monitor, R/W, Panorama, M/S, Wert
in dB, senkrechter Fader mit Pegelanzeige, unten der Name in Spurfarbe. Spuren ohne Kanal (MIDI)
zeigen „Kein Kanal“. Rechts daneben der Inspector:

Kopf mit Spurname und Spurfarbe, darunter ein Bereich ohne Titel mit Name, M/S, Aufnahme,
Monitor, R/W, Zeitbasis, Lanes, Lautstärke und Pan. Danach aufklappbare Sektionen, jede mit einer
hinterlegten Titelleiste über die ganze Breite (Klick klappt auf und zu): **Routing** (Eingang,
Ausgang), Instrument, Inserts, Sends, EQ, Automation (Spuren zeigen, stummschalten, entfernen,
„+ Parameter“ für Kanal, Sends und Plugin-Parameter, Ausdünnen), Notizen (Spec „Spurtypen und
Inspector“). Sektionen, die ein Spurtyp nicht hat, fehlen.

## Untere Zone

Inhalt oben, eine schmale Reiterleiste am **unteren** Rand der Zone: Mixer, Editor, Sample-Editor
(später). Reiter mit Text, aktiver Reiter hervorgehoben.

Mixer und Editoren laufen wahlweise im eigenen Fenster: F3 öffnet bzw. schließt das
Mixer-Fenster, Strg+E öffnet den Editor des gewählten Events (Key-Editor für MIDI-Parts,
Sample-Editor für Audio). Solange ein Inhalt im eigenen Fenster liegt, zeigt die untere Zone
an seiner Stelle „Läuft im eigenen Fenster.“ mit „Zurückholen“; im Fenster holt „In die untere
Zone“ ihn zurück. Jedes Fenster merkt sich Größe und Position in den Einstellungen (unter
Wayland nur die Größe, weil das System die Position nicht verrät). Wo keine eigenen Fenster
möglich sind (Web, Tests), erscheinen sie eingebettet im Projektfenster.

## Rechte Zone

Reiter oben: **Start** (Standard), Medien (Pool), Dateien (Dateibrowser: Ordner öffnen, ein Klick hört vor, Doppelklick
oder Ziehen legt die Datei ins Projekt), Plugins (Doppelklick oder Ziehen: Instrument auf eine
leere Fläche gibt eine neue Instrument-Spur, auf eine Instrument-Spur ersetzt es deren Instrument;
Effekt auf eine Spur wird Insert; oben ein Filter Alle/Instrumente/Effekte). Ein-/ausblendbar
über Alt+R und den Zonen-Schalter.

Die Startseite zeigt große Kacheln mit eigenen Symbolen, je zwei nebeneinander: Instrumente,
Effekte, Medien, Dateien. Ein Klick öffnet den passenden Reiter; „Instrumente“ und „Effekte“
zeigen die Plugin-Liste schon gefiltert. Auf der Startseite gibt es kein Suchfeld.

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

1. ✅ Werkzeugzeile mit eigenen Symbolen und Zonen-Schaltern
2. ✅ Info-Zeile, Mehrfachauswahl mit relativen Änderungen
3. ✅ Spurköpfe: Aufnahme-Punkt und neutrale Auswahl, Monitor und R/W, Zeitbasis
   und Lanes
4. ✅ Transportleiste und Reiter der unteren Zone
5. ✅ Farben: neutrale Grautöne, Akzent sparsamer
6. ✅ Rechte Zone: Medien und Plugins, Dateibrowser mit Vorhören, Dateien und Plugins
   ins Projekt ziehen
7. ✅ Inspector-Kopf in Spurfarbe
8. ✅ Quantisierungswert in der Werkzeugzeile, Taste Q, Panel für Swing und Stärke
9. ✅ Snap-Modi, Raster-Typen Sekunden/Frames/Samples, „An Zoom anpassen“
10. ✅ Monitor (Eingang mithören) und Automation R/W in Spurkopf und Inspector
11. ✅ Fokusrahmen: Klick gibt einer Zone den Fokus, schmaler Rahmen in Akzentfarbe
12. ✅ Zeitbasis (musikalisch/linear) und Lanes ein/aus in Spurkopf und Inspector
13. ✅ Feine Rasterlinien in Event-Anzeige und Key-Editor
14. ✅ Automation (Issue #25): Kurven abspielen mit R, unter der Spur bearbeiten,
    schreiben mit W in Touch und Auto-Latch, Sends, Overwrite und Trim
15. ✅ Statusanzeige der Transportleiste läuft nicht mehr über Tempo und Takt
16. ✅ Titelleiste wie in den Craft-Apps: Symbol, Menü, Titel und Fensterknöpfe in einer Zeile,
    `craft-window` entscheidet pro Desktop
17. ✅ Comping: Comp-Werkzeug, Abschnitte eines Takes per Klick oder Ziehen hörbar machen
18. ✅ Farben: Klick auf die Farbleiste im Spurkopf öffnet die Palette; Events folgen der Spurfarbe,
    über Bearbeiten → Event-Farbe bekommen sie eine eigene
19. ✅ Linke Zone mit Kanalzug-Spalte neben dem Inspector
20. ✅ Spurkopf zweizeilig mit Farbleiste, Spurnummer und Lautstärkeregler
21. ✅ Inspector: Sektionen mit Titelleiste, Eingang und Ausgang unter „Routing“
22. ✅ Werkzeugzeile: globale Zustände links, Werkzeuge zentriert, Zonen-Schalter rechts
23. ✅ Rechte Zone: Startseite mit Kacheln, Plugin-Filter Instrumente/Effekte
24. ✅ Zonen: Größen und Sichtbarkeit im Projekt gespeichert, Alt+L linke Zone, Alt+I Inspector-Spalte, benannte Layouts (Ansicht → Layouts, Strg+Alt+1 bis 9)
25. ✅ Eigene Fenster: Mixer (F3) und Editoren (Strg+E), Größe und Position gemerkt

**Als Nächstes:** Aus dieser Liste ist nichts mehr offen. Weitere Angleichungen ergeben sich aus
`docs/spec.md`.
