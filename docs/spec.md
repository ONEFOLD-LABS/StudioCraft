# Cleanroom-Spezifikation: DAW-Bedienkonzept

Stand: 2026-11-18

> Diese Datei ist ein Export. Änderungen bitte zuerst in der lebenden Fassung machen und danach hier nachziehen.

## Zweck und Cleanroom-Regeln

Diese Spezifikation beschreibt das Bedienkonzept von StudioCraft, einer plattformübergreifenden DAW in Rust, die bewährter DAW-Bedienung folgt. Sie ist die einzige Vorlage für die Implementierung: Wer Code schreibt, arbeitet nur gegen dieses Dokument.

**Erlaubte Quellen**

- Öffentliche Handbücher, Hilfeseiten und Release Notes, zusammengefasst in eigenen Worten
- Tutorials, Videos und eigenes Beobachten der laufenden Software
- Offene Standards und SDKs: VST3 (MIT), CLAP, LV2, MIDI 1.0/2.0, Standard MIDI File, MusicXML

**Tabu**

- Dekompilieren, Disassemblieren oder Debuggen fremder Programme
- Reverse Engineering fremder Projektformate
- Namen, Logos, Icons, Skins, Farbschemata und Sounds anderer Hersteller; fremde Produktnamen im Projekt
- Wörtlich übernommene Handbuchtexte und Screenshots im Repo

**Rollen**

| Rolle | Darf andere DAWs ansehen | Schreibt |
| --- | --- | --- |
| Spec-Autor | ja | dieses Dokument |
| Implementierer | nein, nur die Spec | Code und Tests |

Begriffe wie Inspector, Insert oder Send sind allgemeine Fachsprache und dürfen verwendet werden. Für Fenster mit markenähnlichen Namen wählt das Projekt eigene, allgemeine Namen; siehe Abschnitt Grundsatzentscheidungen.

## Begriffe und Datenmodell

Ein Projekt besteht aus Spuren, auf denen Events liegen; Audio-Events verweisen auf Clips im Pool, MIDI-Daten liegen in Parts. Alle Zeitangaben werden intern in einer tempo-unabhängigen Einheit gespeichert und erst über die Tempo-Map in Takte oder Sekunden umgerechnet.

| Begriff | Bedeutung | Implementierungshinweis |
| --- | --- | --- |
| Projekt | Eine Datei mit Spuren, Mixer, Tempo-Map, Markern und Einstellungen | Eigenes, versioniertes Format plus Projektordner |
| Pool | Liste aller Audio-Dateien und Clips des Projekts | Clips sind unveränderliche Referenzen auf Dateien |
| Audio-Clip | Datei plus optionaler Bearbeitungsverlauf (Offline-Effekte) | Destruktive Effekte erzeugen neue Dateien |
| Audio-Event | Ausschnitt eines Clips auf einer Spur: Start, Offset, Länge, Fades, Lautstärke | Nicht-destruktiv |
| Audio-Part | Container, der mehrere Audio-Events gruppiert | Optional, nach MVP |
| MIDI-Part | Container für Noten und Controller-Daten | Noten relativ zum Part-Start |
| Spur | Zeile im Projektfenster; Typ bestimmt Inhalt und Kanal | Siehe Spurtypen |
| Kanal | Mixer-Gegenstück einer Spur mit Inserts, Sends, Fader | Eine Audio-Spur hat genau einen Kanal |
| Tempo-Map | Tempo- und Taktart-Ereignisse über die Zeit | Fest oder variabel (Tempospur) |
| Position | Takt.Schlag.Sechzehntel.Ticks, Zeitcode, Sekunden oder Samples | Anzeigeformat wählbar pro Lineal |
| Locatoren | Linker und rechter Locator begrenzen Cycle, Punch und Export | Zwei Positionen im Projekt |
| Projekt-Cursor | Aktuelle Wiedergabeposition | Eine Position, folgt dem Transport |
| Marker | Benannte Position oder Bereich (Cycle-Marker) | Eigene Marker-Spur |

**Zeitbasis:** Jede Spur ist entweder musikalisch (Events hängen an Takten, Tempoänderungen verschieben sie) oder linear (Events hängen an Sekunden). Standard ist musikalisch für MIDI und linear für Audio.

**Undo:** Jede Bearbeitung ist ein Befehl im Undo-Verlauf. Der Verlauf ist unbegrenzt konfigurierbar und wird mit dem Projekt nicht gespeichert.

## Fenster-Architektur

Alles passiert in einem Projektfenster: In der Mitte liegt die Event-Anzeige, um sie herum liegen Zonen, die einzeln ein- und ausgeblendet werden. Editoren und Mixer laufen wahlweise in der unteren Zone oder als eigene Fenster.

```
+--------------------------------------------------------------------------+
| Werkzeugzeile: Werkzeuge 1 bis 0, Snap (J), Raster, Quantisierung        |
+--------------+-------------+-------------------------+-------------------+
| Linke Zone   | Spurliste   | Lineal: Locatoren,Cycle | Rechte Zone       |
| Inspector    | Spurköpfe:  +-------------------------+ Browser, Plugins, |
| der gewähl-  | Name, M, S, | EVENT-ANZEIGE           | Medien            |
| ten Spur     | R, Monitor, | Events, Parts, Lanes    |                   |
|              | R/W         | Automationsspuren       |                   |
| Alt+I, Alt+L |             | unter jeder Spur        | Alt+R             |
+--------------+-------------+-------------------------+-------------------+
| Untere Zone: Key-Editor, Sample-Editor oder Mixer im Projektfenster      |
| Strg+E öffnet Editoren, F3 den Mixer als eigenes Fenster                 |
+--------------------------------------------------------------------------+
| Transportleiste (F2): Start, Stopp, Aufnahme, Locatoren, Cycle, Punch,   |
| Metronom                                                                 |
+--------------------------------------------------------------------------+
```

- Spurliste und Event-Anzeige scrollen vertikal gemeinsam; Lineal und Event-Anzeige horizontal gemeinsam
- Zonengrößen sind per Trenner ziehbar und werden im Projekt gespeichert
- Fensterlayouts (welche Zonen und Fenster offen sind) lassen sich benennen und per Tastenkombination abrufen
- Mehrere Monitore: jedes eigenständige Fenster merkt sich Bildschirm und Position

## Spurtypen und Inspector

Der Spurtyp legt fest, welche Events eine Spur trägt und ob sie einen Mixer-Kanal hat. Der Inspector links zeigt immer die Einstellungen der ausgewählten Spur in aufklappbaren Sektionen.

| Spurtyp | Inhalt | Mixer-Kanal | Inspector-Sektionen | MVP |
| --- | --- | --- | --- | --- |
| Audio | Audio-Events, Parts | ja | Eingang/Ausgang, Lautstärke/Pan, Inserts, EQ, Sends, Notizen | ja |
| Instrument | MIDI-Parts, steuert ein Instrument-Plugin | ja (Plugin-Ausgang) | Instrument, MIDI-Eingang, Inserts, Sends, MIDI-Modifier | ja |
| MIDI | MIDI-Parts an externes Gerät oder Rack-Instrument | nein | MIDI-Eingang/-Ausgang, Kanal, Programm, MIDI-Inserts | ja |
| Gruppe | keine Events, sammelt Ausgänge anderer Kanäle | ja | wie Audio ohne Eingang | ja |
| FX-Kanal | keine Events, Ziel für Sends | ja | Inserts, Ausgang | ja |
| Ordner | andere Spuren, ein-/ausklappbar | nein | Mute/Solo für alle enthaltenen | ja |
| Marker | Marker und Cycle-Marker | nein | Marker-Liste | ja |
| Tempo/Taktart | Tempo- und Taktart-Ereignisse | nein | Tempo-Modus fest/variabel | ja |
| Lineal | zusätzliches Zeitlineal mit eigenem Format | nein | Anzeigeformat | später |
| Automation | Kurve eines Parameters, unter ihrer Spur | nein | Parameter-Auswahl, Modus | ja |
| Video | Videoframes als Thumbnails | nein | Offset, Ton an/aus | später |

**Spurkopf in der Spurliste:** links eine Farbleiste mit der Spurnummer, daneben zwei Zeilen. Oben: Automationspfeil, Mute, Solo, Name und ein Lautstärkeregler (Strg+Klick: 0 dB). Unten: Aufnahme bereit, Monitor, Automation lesen, Automation schreiben, Zeitbasis, Lanes ein/aus. Breite und sichtbare Elemente sind pro Spurtyp einstellbar.

**Farbe:** Jede Spur hat eine Farbe; neue Events erben sie. Die Farbe erscheint auch auf dem Mixer-Kanal.

## Werkzeuge

Genau ein Werkzeug ist aktiv; es gilt in der Event-Anzeige und in den Editoren. Die Zifferntasten 1 bis 0 der Haupttastatur wählen die Werkzeuge in der Reihenfolge der Werkzeugzeile; ein Rechtsklick in die Event-Anzeige öffnet eine Werkzeugpalette am Mauszeiger.

| Taste | Werkzeug | Klick | Ziehen | Mit Modifier |
| --- | --- | --- | --- | --- |
| 1 | Objektauswahl | Event wählen | Verschieben; an Rändern Länge ändern | Strg: kopieren statt verschieben; Umschalt: Auswahl erweitern; Alt: verschieben ohne Snap |
| 2 | Bereichsauswahl | – | Zeitbereich über Spuren wählen | Umschalt: Bereich erweitern |
| 3 | Trennen | Event an Position teilen | – | Alt: in gleich lange Stücke teilen |
| 4 | Kleben | Event mit dem nächsten verbinden | – | Alt: alle folgenden verbinden |
| 5 | Löschen | Event löschen | über Events streichen löscht alle | – |
| 6 | Zoom | einzoomen | Rechteck zoomen | Alt: auszoomen |
| 7 | Stummschalten | Event stumm/laut | über Events streichen | – |
| 8 | Zeichnen | leeren Part oder Note anlegen | Länge aufziehen | – |
| 9 | Wiedergabe | Event ab Position anhören | Scrubben | – |
| 0 | Zeitkorrektur | – | Taktposition an Audio-Position ziehen (Tempo-Map anpassen) | später |

**Varianten pro Werkzeug:** Objektauswahl hat drei Modi für das Ziehen am Rand: normal (Länge), Inhalt mitziehen (Time-Stretch) und Größe mit Verschieben des Inhalts. Der Modus wird über ein Untermenü am Werkzeugknopf gewählt.

**Smart-Bereiche eines Events:** Die obere Hälfte am Rand sind Fade-Griffe, die Mitte oben der Lautstärkegriff, die untere Hälfte am Rand ändert die Länge. Der Mauszeiger zeigt, welcher Bereich aktiv ist.

## Raster, Snap und Bearbeitung

Snap ist global ein- und ausschaltbar (Taste J); wohin eingerastet wird, bestimmen Snap-Modus und Raster-Typ. Alt beim Ziehen hebt Snap für diese eine Aktion auf.

**Snap-Modi**

| Modus | Rastet ein auf |
| --- | --- |
| Raster | Rasterlinien |
| Raster relativ | Rasterabstand, Abstand des Events zur Linie bleibt |
| Events | Anfang und Ende anderer Events |
| Cursor | Projekt-Cursor |
| Raster + Cursor, Events + Cursor, Events + Raster + Cursor | Kombinationen |
| Schieben (Shuffle) | Events tauschen die Plätze, keine Lücken |

**Raster-Typen:** Takt, Zählzeit, „Quantisierung verwenden“ (folgt dem Quantisierungswert), Sekunden, Frames, Samples. „An Zoom anpassen“ verfeinert das Raster beim Einzoomen.

**Quantisierung:** Ein globaler Quantisierungswert (1/1 bis 1/64, triolisch, punktiert) gilt für MIDI-Noten und Audio-Hitpoints. Taste Q quantisiert die Auswahl, Swing und Stärke (0–100 %) stellt ein eigenes Panel ein.

**Fades und Crossfades**

- Fade-In und Fade-Out am Griff oben links bzw. rechts ziehen; Doppelklick öffnet den Kurveneditor
- Überlappen sich zwei Events auf einer Spur, erzeugt X einen Crossfade; beide Events klingen durch die Überlappung. Doppelklick auf einen seiner Fade-Griffe öffnet den Crossfade-Editor: eine Kurve für beide Seiten („Gleiche Lautstärke“, „Gleiche Leistung“, S-Kurve, Exponentiell) oder je Seite einzeln, mit Vorschau
- Fades sind nicht-destruktiv und Teil des Events

**Verschieben und Kopieren**

| Aktion | Eingabe |
| --- | --- |
| Verschieben | Ziehen mit Objektauswahl |
| Kopieren | Strg + Ziehen |
| Geteilte Kopie (Änderungen wirken auf alle Kopien) | Strg + Umschalt + Ziehen |
| Nur horizontal oder vertikal | Ziehen, dann Strg halten (Richtung wird beim Loslassen fixiert) |
| Wiederholen | Strg + K öffnet den Dialog (Anzahl, geteilte Kopien), Strg + D dupliziert einmal; Alt + Ziehen am rechten Rand wiederholt so oft wie gezogen |
| Nudge | Strg + Pfeil links/rechts verschiebt die Auswahl um einen Rasterschritt, Strg + Umschalt + Pfeil ändert ihre Länge (Pfeil allein wählt Event bzw. Spur); im Key-Editor gilt das für die gewählten Noten, Pfeil hoch/runter transponiert sie |

## Transport und Aufnahme

Die Transportleiste unten im Projektfenster bedient Wiedergabe, Aufnahme und Locatoren; dieselben Funktionen liegen auf dem Nummernblock. Leertaste startet und stoppt.

**Locatoren und Cycle**

- Linker (L) und rechter (R) Locator begrenzen den Cycle-Bereich; Cycle an/aus mit Nummernblock-Geteilt (/)
- Strg/Alt + Klick ins Lineal setzt den linken bzw. rechten Locator
- Ziehen in der oberen Hälfte des Lineals zieht einen neuen Cycle-Bereich auf; Klicken oder Ziehen in der unteren Hälfte setzt den Cursor
- P setzt die Locatoren auf die aktuelle Auswahl

**Stopp-Verhalten:** Optional springt der Cursor beim Stoppen zur Startposition zurück. Zweimal Stopp springt immer zum Anfang (Position 1.1.1.0 bzw. linker Locator, einstellbar). Beides steht im Menü Transport und gilt für alle Projekte.

**Punch:** Punch-In und Punch-Out an den Locatoren (Tasten I und O). Aufnahme startet bzw. endet dort automatisch, auch während laufender Wiedergabe.

**Audio- und MIDI-Geräte:** Menü Transport → „Audio- und MIDI-Geräte …“: Host, Aus- und Eingang, Abtastrate, Blockgröße (mit Latenz) und MIDI-Eingänge an/aus. Die Wahl liegt in den Einstellungen. Ein Wechsel von Gerät oder Blockgröße gilt sofort, eine andere Abtastrate nach einem Neustart. Verschwindet das Gerät, meldet die Statuszeile es.

**Vorzähler und Metronom:** Metronom an/aus mit C; Vorzähler in Takten einstellbar. Klick-Sound, Lautstärke und Ausgang sind Projekteinstellungen.

**Aufnahmemodi**

| Situation | Audio | MIDI |
| --- | --- | --- |
| Neue Aufnahme über bestehende Events | Neue Lane, alte bleibt stumm erhalten | Neuer Part (Standard), alternativ Mischen oder Ersetzen |
| Aufnahme im Cycle | Pro Durchgang eine Lane (Takes) | Stacked (je Durchgang ein Part), Mix (alle in einem Part) oder Ersetzen |
| Nachträgliches Retrospective Record | – | Gespielte Noten ohne Aufnahme nachträglich als Part übernehmen (Umschalt + Nummernblock-Mal) |

**Takes und Lanes:** Takes einer Spur liegen in Lanes unter ihr. Mit dem Comp-Werkzeug wählt man pro Abschnitt den besten Take; die Spur spielt nur die gewählten Abschnitte. Comping kommt nach dem MVP.

**Nummernblock-Belegung**

| Taste | Funktion |
| --- | --- |
| Enter | Start |
| 0 | Stopp |
| Mal (\*) | Aufnahme |
| Komma | Zum Anfang |
| 1 / 2 | Zum linken / rechten Locator |
| 3 bis 9 | Zu Marker 3 bis 9 |
| Minus / Plus | Zurück- / Vorspulen |
| Geteilt (/) | Cycle an/aus |

Die Zahlenreihe wählt weiter die Werkzeuge; nur Tasten, die das System als Nummernblock meldet, bedienen den Transport. Mit Strg oder Alt bleiben sie normale Kürzel; gehalten wiederholen nur Minus und Plus. Im Web fehlt dieser Abgriff noch; dort verhält sich der Nummernblock wie die Zahlenreihe.

## Mixer

Der Mixer zeigt zu jeder Spur mit Kanal einen Kanalzug, links Eingänge, rechts Ausgangsbusse; er öffnet mit F3 als eigenes Fenster oder in der unteren Zone des Projektfensters. Spur und Kanal sind dasselbe Objekt: Mute, Solo, Name, Farbe und Auswahl sind synchron.

**Signalfluss eines Kanalzugs (von oben nach unten)**

1. Eingang (Hardware-Eingang, Gruppe oder keiner) mit Eingangsgain und Phasendrehung
2. Pre-Sektion: Hochpass- und Tiefpassfilter
3. Inserts 1–6 (Pre-Fader)
4. Kanal-EQ, 4 Bänder, abschaltbar
5. Inserts 7–8 (Post-Fader)
6. Fader und Panorama
7. Sends 1–8, je Pre- oder Post-Fader, mit eigenem Pegel und Ziel (FX-Kanal oder Gruppe)
8. Ausgang (Bus oder Gruppe)

Die Insert-Aufteilung Pre/Post ist eine Projekt-Voreinstellung; jeder Slot ist einzeln schaltbar (an/aus, Bypass).

**Kanalzug-Elemente**

| Element | Verhalten |
| --- | --- |
| Fader | Ziehen; Strg + Klick setzt 0 dB; Umschalt + Ziehen fein |
| Panorama | Stereo-Balance (Standard) oder Stereo-Kombination; Strg + Klick zentriert |
| Mute / Solo | Alt + Klick auf Solo: Solo exklusiv; Strg + Klick auf Solo: Solo-Schutz (Kanal bleibt hörbar) |
| Pegelanzeige | Peak mit Halten, Klick setzt Clip-Anzeige zurück |
| Monitor / Aufnahme | Wie im Spurkopf |

**Routing und Busse**

- Ausgangsbusse und Eingangsbusse werden in einem eigenen Dialog angelegt und auf Hardware-Ports gelegt (Taste F4)
- Ein Projekt hat mindestens einen Stereo-Ausgangsbus (Main Mix)
- Gruppen können in Gruppen geroutet werden; das System verhindert Schleifen
- Direktes Routing auf mehrere Ziele gleichzeitig kommt nach dem MVP

**Sichtbarkeit und Ansicht**

- Linke Zone: Liste aller Kanäle mit Sichtbarkeits-Haken
- Rack-Ansicht oberhalb der Fader blendet Inserts, EQ, Sends ein und aus
- Kanalbreite schmal/breit; Konfigurationen der Sichtbarkeit speicherbar

**Link-Gruppen (später):** Ausgewählte Kanäle lassen sich verknüpfen, sodass Fader, Mute und Solo gemeinsam reagieren.

## Editoren

Doppelklick auf ein Event öffnet den Standard-Editor seines Typs: Audio → Sample-Editor, MIDI → Key-Editor (oder Drum-Editor, wenn der Spur eine Drum-Map zugewiesen ist). Jeder Editor kann als eigenes Fenster oder in der unteren Zone geöffnet werden (Strg + E bzw. Voreinstellung).

| Editor | Zeigt | Kernfunktionen | MVP |
| --- | --- | --- | --- |
| Key-Editor | Noten als Balken über einer Klaviatur, darunter Controller-Lanes | Zeichnen, Länge ziehen, Velocity in Lane, Quantisieren, Transponieren mit Pfeil hoch/runter | ja |
| Drum-Editor | Noten als Rauten pro Instrument-Zeile, Namen aus der Drum-Map | Eigene Quantisierung pro Zeile, Länge ignoriert, Mute/Solo pro Zeile | später |
| List-Editor | Alle Events als Tabelle (Position, Länge, Wert, Kanal) | Werte direkt eingeben, filtern nach Event-Typ | später |
| Sample-Editor | Wellenform eines Clips in voller Größe | Bereich wählen, Offline-Effekte, Hitpoints, Tempo-Erkennung | ja (ohne Hitpoints) |
| Noten-Editor | Partitur | Notensatz, Druck | später |

**Gemeinsames Verhalten**

- Eigene Werkzeugzeile mit den passenden Werkzeugen aus Abschnitt Werkzeuge
- Eigene Info-Zeile: Werte der Auswahl (Start, Länge, Tonhöhe, Velocity) sind direkt editierbar; bei Mehrfachauswahl relativ
- Akustisches Feedback: angeklickte oder verschobene Noten erklingen (abschaltbar)
- Mehrere Parts gleichzeitig öffnen; ein aktiver Part, die anderen gedämpft
- „Editor folgt Wiedergabe“ scrollt automatisch mit (Taste F)

**Controller-Lanes im Key-Editor:** Beliebig viele Lanes, jede zeigt einen Datentyp (Velocity, Pitchbend, CC 1–127, Aftertouch). Zeichnen mit Linien-, Parabel- und Sinus-Formen; die Formen kommen nach dem MVP.

## Automation

Jeder Parameter eines Kanals oder Plugins kann automatisiert werden; die Kurve liegt auf einer Automationsspur direkt unter ihrer Spur. R (Read) spielt die Kurve ab, W (Write) zeichnet Bewegungen der Bedienelemente während der Wiedergabe auf.

**Schreibmodi (bei aktivem W)**

| Modus | Verhalten | MVP |
| --- | --- | --- |
| Touch | Schreibt, solange das Bedienelement gehalten wird; danach zurück zur bestehenden Kurve | ja |
| Auto-Latch | Schreibt ab Berührung bis zum Stopp mit dem letzten Wert | ja |
| Overwrite | Schreibt ab Start nur für die Lautstärke, unabhängig von Berührung | später |
| Trim | Verändert die bestehende Kurve relativ | später |

**Automationsspuren**

- Pfeil im Spurkopf klappt die erste Automationsspur auf; Plus darunter fügt die nächste hinzu
- Bei aktivem W werden Spuren für neu bewegte Parameter automatisch angelegt
- Eine Automationsspur kann stummgeschaltet werden; der Parameter steht dann auf seinem statischen Wert

**Kurvenbearbeitung**

- Klick mit Objektauswahl auf die Linie setzt einen Punkt; Ziehen bewegt Punkte
- Zeichnen-Werkzeug zeichnet frei; Linie mit Formen (Linie, Parabel, Sinus, Rechteck) kommt später
- Bereichsauswahl auf einer Automationsspur skaliert die Punkte im Bereich per Ziehen nach oben/unten
- Punkte werden beim Schreiben ausgedünnt (Reduktionsgrad einstellbar)

**Engine-Hinweis:** Automation wird sample-genau ausgewertet; Plugins bekommen Parameteränderungen als zeitgestempelte Events (CLAP/VST3 sample-accurate), nicht einmal pro Puffer.

## Tastaturkonzept

Alle Befehle laufen über eine zentrale Befehlsliste mit frei belegbaren Tasten; die Standardbelegung unten entspricht dem gewohnten Workflow. Belegungen sind als Preset exportierbar, und Makros reihen mehrere Befehle aneinander. Acht Makroplätze werden im Dialog „Tastaturbefehle“ benannt und mit Befehlen gefüllt, bekommen eine Taste wie jeder Befehl und stehen unter Bearbeiten → Makros; Presets enthalten sie mit.

**Modifier-Logik (gilt überall)**

| Modifier | Bedeutung beim Ziehen/Klicken |
| --- | --- |
| Strg | Kopieren, Richtung fixieren, Standardwert setzen |
| Umschalt | Auswahl erweitern, Feineinstellung |
| Alt | Snap aufheben, Werkzeug-Variante |
| Strg + Umschalt | Geteilte Kopie |

**Standardbelegung**

| Gruppe | Taste | Befehl |
| --- | --- | --- |
| Datei | Strg + N / O / S | Neu / Öffnen / Speichern |
| Datei | Strg + Alt + S | Neue Version speichern (Name mit Zähler) |
| Bearbeiten | Strg + Z / Strg + Umschalt + Z | Rückgängig / Wiederholen |
| Bearbeiten | Strg + X / C / V, Entf | Ausschneiden / Kopieren / Einfügen, Löschen |
| Bearbeiten | Strg + D | Duplizieren |
| Bearbeiten | Alt + X | An Cursor teilen |
| Bearbeiten | Strg + A | Alles auswählen |
| Navigation | H / G | Horizontal ein- / auszoomen |
| Navigation | Umschalt + F | Auf Projekt zoomen |
| Navigation | Pfeil links/rechts | Vorheriges/nächstes Event wählen |
| Navigation | Pfeil hoch/runter | Vorherige/nächste Spur wählen |
| Spur | M / S / R | Mute / Solo / Aufnahme bereit (ausgewählte Spur) |
| Spur | Strg + T | Spur hinzufügen |
| Fenster | F2 / F3 / F4 | Transportleiste / Mixer / Audio-Verbindungen |
| Fenster | Alt + I / Alt + L / Alt + R | Inspector / linke Zone / rechte Zone |
| Transport | siehe Abschnitt Transport | Nummernblock und Leertaste |
| Werkzeuge | 1 bis 0 | siehe Abschnitt Werkzeuge |
| Bearbeitung | J / Q / X | Snap / Quantisieren / Crossfade |
| Transport | C / I / O | Metronom / Punch-In / Punch-Out |

**Fokus-Regel:** Tasten wirken auf das Fenster unter dem Fokus. Hat ein Editor in der unteren Zone den Fokus, beziehen sich Bearbeitungsbefehle auf seine Noten, nicht auf die Events im Projektfenster. Ein schmaler Rahmen zeigt, welche Zone den Fokus hat.

## Grundsatzentscheidungen, MVP und offene Fragen

Der MVP ist erreicht, wenn man ein Lied mit Audio- und Instrument-Spuren aufnehmen, arrangieren, mischen und als WAV exportieren kann. Alles mit „später“ in den Tabellen oben gehört nicht dazu.

**Grundsatzentscheidungen**

| Thema | Entscheidung |
| --- | --- |
| Fensternamen | allgemeine Begriffe: Mixer, Browser, Inspector |
| Plugin-Formate | CLAP zuerst, dann VST3, AU (nur macOS) und LV2; kein VST2 |
| Audio-Backend | cpal: PipeWire, ALSA, JACK (Linux, FreeBSD), WASAPI, ASIO (Windows), CoreAudio (macOS), Web Audio (WASM) |
| Lizenzierung | keine Aktivierung |
| Projektformat | offenes, versioniertes Textformat plus Audio-Ordner; diff- und git-freundlich |
| Plattform | Linux, Windows, macOS, FreeBSD und Web (WASM), Pakete wie die Craft-Familie |

**MVP-Umfang**

Stand 2026-11-19; offen heißt: fehlt oder ist nicht auf echter Hardware geprüft.

- [ ] Audio-Engine über cpal auf allen Systemen, Latenz ≤ 10 ms bei 48 kHz / 256 Samples
  (Engine, Blockgröße und Latenzausgleich vorhanden; die Latenz ist auf Hardware nicht gemessen)
- [x] Projektfenster mit Spurliste, Event-Anzeige, Lineal, Inspector, Transportleiste
- [ ] Spurtypen Audio, Instrument, MIDI, Gruppe, FX, Ordner, Marker, Tempo
  (Audio, Instrument, MIDI, Gruppe und FX fertig; Ordner nur als Typ ohne Funktion; Marker und
  Tempo fehlen, das Tempo ist fest)
- [ ] Werkzeuge 1 bis 9, Snap, Raster, Fades, Crossfades (alles bis auf Crossfades)
- [x] Aufnahme Audio und MIDI inkl. Cycle-Takes, Punch, Metronom
- [ ] Mixer mit Inserts, Sends, EQ, Gruppen, Bussen (alles bis auf den Kanal-EQ)
- [x] CLAP-Hosting mit Plugin-GUI
- [ ] Key-Editor und Sample-Editor (Basis) (Key-Editor fertig, Sample-Editor fehlt)
- [x] Automation Read/Write mit Touch und Auto-Latch (dazu Overwrite und Trim)
- [ ] Frei belegbare Tastatur mit Standardbelegung (Standardbelegung fertig, Belegen fehlt)
- [x] Export als WAV/FLAC (Mixdown der Locator-Range)

**Offene Fragen**

- [ ] Projektname: entschieden, StudioCraft (Repo unter Dokumente/GitHub/StudioCraft). Verworfen: SoundCraft (in der Familie vergeben), AudioCraft (Meta), MixCraft (bestehende DAW), RecordCraft (Rust-Recorder kaptaindieter/RecordCraft). Markenrecherche steht noch aus.
- [ ] GUI-Toolkit: entschieden, egui/eframe mit craft-ui wie die übrigen Craft-Apps (nativ und Web). Prototyp bestätigt: 0,15 ms CPU pro Frame bei 100 und 1000 Spuren (Release-Build)
- [ ] Plugin-GUIs unter Wayland: Fremdfenster einbetten geht nur über X11/XWayland; als eigene Fenster öffnen?
- [ ] Soll ein Standard-MIDI-File- und AAF/OMF-Import für den Umstieg von anderen DAWs ins MVP?
- [ ] Wer übernimmt die Spec-Autor-Rolle, wer implementiert (Cleanroom-Trennung)?
