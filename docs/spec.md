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
| Audio-Part | Container, der mehrere Audio-Events gruppiert | Events relativ zum Part-Start (seit 0.9) |
| MIDI-Part | Container für Noten und Controller-Daten | Noten relativ zum Part-Start |
| Spur | Zeile im Projektfenster; Typ bestimmt Inhalt und Kanal | Siehe Spurtypen |
| Kanal | Mixer-Gegenstück einer Spur mit Inserts, Sends, Fader | Eine Audio-Spur hat genau einen Kanal |
| Tempo-Map | Tempo- und Taktart-Ereignisse über die Zeit | Fest oder variabel (Tempospur) |
| Position | Takt.Schlag.Sechzehntel.Ticks, Zeitcode, Sekunden oder Samples | Anzeigeformat wählbar pro Lineal |
| Locatoren | Linker und rechter Locator begrenzen Cycle, Punch und Export | Zwei Positionen im Projekt |
| Projekt-Cursor | Aktuelle Wiedergabeposition | Eine Position, folgt dem Transport |
| Marker | Benannte Position oder Bereich (Cycle-Marker) | Eigene Marker-Spur |

**Zeitbasis:** Jede Spur ist entweder musikalisch (Events hängen an Takten, Tempoänderungen verschieben sie) oder linear (Events hängen an Sekunden). Standard ist musikalisch für MIDI und linear für Audio. Einzelne Audio-Events können dem Projekttempo folgen (Info-Zeile „Folgt dem Tempo“ mit dem Tempo der Datei, vorbelegt aus einer BPM-Angabe im Dateinamen): sie hängen dann an Takten wie MIDI, und ihr Inhalt wird so gedehnt, dass eine Viertel der Datei eine Viertel im Projekt dauert – auch entlang der Tempospur; die Tonhöhe bleibt.

**Audio-Part (seit 0.9):** Bearbeiten → „Audio-Part bilden“ fasst gewählte Audio-Events einer Audiospur zu einem Part zusammen, „Audio-Part auflösen“ legt sie wieder einzeln auf die Spur (an derselben Stelle, auf den Part zugeschnitten). Der Part wird wie ein Event verschoben, kopiert, ausgeschnitten, stummgeschaltet, mit Fades und Lautstärke versehen und am Rand gekürzt; seine Events bleiben dabei an ihrer Stelle im Projekt. Was hinter dem Part-Ende liegt, ist nur ausgeblendet; rückt der Part-Anfang über den Anfang eines Events, wird dessen Kopf abgeschnitten. Teilen schneidet die Events am Schnitt mit; Kleben und Strecken gelten nicht für Audio-Parts. Wiedergabe und Export spielen die Events des Parts mit Mute, Lautstärke und Fades des Parts. Die Event-Anzeige zeigt ihre Wellenformen im Part.

**Undo:** Jede Bearbeitung ist ein Befehl im Undo-Verlauf. Der Verlauf ist unbegrenzt konfigurierbar und wird mit dem Projekt nicht gespeichert.

**Automatische Sicherung:** In einstellbarem Abstand (Datei → „Automatisch sichern“, Standard 5 Minuten, „Aus“ möglich) sichert StudioCraft geänderte Projekte als nummerierte Dateien neben der Projektdatei (`Name.backup-07.studiocraft`), ungespeicherte Projekte im Einstellungsordner. Die ältesten Sicherungen über der eingestellten Anzahl (Standard 10) werden gelöscht. Sicherungen schreiben atomar im Hintergrund und markieren das Projekt nicht als gespeichert. Wurde StudioCraft nicht sauber beendet, bietet es beim nächsten Start die neueste Sicherung an; sie öffnet sich als ungespeicherte Fassung des ursprünglichen Projekts.

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
- Bildlaufleisten am unteren und rechten Rand der Event-Anzeige: Griff ziehen verschiebt, Klick daneben blättert; dazu Mausrad (Spuren), Umschalt + Mausrad bzw. waagerechtes Wischen (Zeit)
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
| Lineal | zusätzliches Zeitlineal mit eigenem Format (Takte, Sekunden, Timecode 25/30 fps, Samples); beliebig viele, Projekt → „Lineal-Spur hinzufügen“ | nein | Anzeigeformat | 0.8 |
| Automation | Kurve eines Parameters, unter ihrer Spur | nein | Parameter-Auswahl, Modus | ja |
| Video | Videoframes als Thumbnails | nein | Offset, Ton an/aus | später (#174; Codec-Lizenzen: `docs/video-dekodierung.md`) |

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
| 0 | Zeitkorrektur | – | Eine Stelle packen (z. B. einen Schlag im Audio) und auf eine Zählzeit ziehen: die Tempo-Map passt sich seit dem letzten Tempowechsel an, Audio auf linearen Spuren behält seine Zeit | Alt: ohne Snap |

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

**An Ort und Stelle rendern**

Bearbeiten → „An Ort und Stelle rendern“ verwandelt die gewählte Audio- oder Instrument-Spur in Audio: die Bereichsauswahl, sonst die gewählten Events, sonst alle Events der Spur. „Nur Instrument und Inserts“ rendert ohne Fader, Panorama und EQ, „Mit Kanaleinstellungen“ mit ihnen; Sends fehlen. Das Ergebnis (mit 2 s Nachklang) liegt als neuer Clip auf einer neuen Audio-Spur unter der Quelle und wird beim Speichern als WAV im Projektordner abgelegt. Gerendert wird offline mit eigenen Plugin-Instanzen, schneller als Echtzeit; die Wiedergabe läuft dabei weiter.

**Einfrieren**

Bearbeiten → „Spur einfrieren“ rendert eine Audio- oder Instrument-Spur mit Instrument, Eingang, Pre-Sektion und Inserts (ohne Fader, Panorama, EQ und Sends, ausgeglichen um die Plugin-Latenz) offline in einen Clip und entlädt danach ihre Plugins. Die Spur spielt nur noch diesen Clip durch ihren Kanal; Fader, Panorama, EQ, Sends und deren Automation bleiben live. Events, Inserts und Instrument sind gesperrt, die Spur ist in der Event-Anzeige abgedeckt und mit „Eingefroren“ beschriftet, der Inspector zeigt statt der Inserts „Einfrieren aufheben“. Aufheben lädt die Plugins mit ihrem gespeicherten Zustand neu; Einfrieren und Aufheben lassen sich rückgängig machen. Der Clip wird beim Speichern als WAV im Projektordner abgelegt.

**Stems exportieren**

Datei → „Stems exportieren …“ schreibt je gewähltem Kanal eine Datei: Spuren, Gruppen und FX-Kanäle (samt allem, was in sie hineinläuft) und den Hauptmix. Alle Dateien decken denselben Bereich ab (zwischen den Locatoren, sonst das ganze Projekt, mit 2 s Nachklang) und sind sampelgenau gleich lang. „Mit Kanalbearbeitung“ nimmt Inserts, EQ, Fader und Panorama des Kanals mit, sonst nur, was hineinläuft. Dateiname nach Muster (`{projekt}`, `{kanal}`, `{nr}`), Format wie beim Mixdown (WAV, FLAC, AIFF mit 24 Bit, Projekt-Abtastrate).

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

**Tempospur:** Projekt → „Tempospur hinzufügen“ legt die Spur an und schaltet sie ein; „Spur“ in der Transportleiste schaltet zwischen Tempospur und festem Tempo um (die Punkte bleiben). In der Spur setzt der Stift einen Tempowechsel, die Objektauswahl zieht ihn (Takt mit Snap, Tempo nach Höhe), der Radiergummi löscht ihn. Im Inspector stehen alle Punkte mit Tempo und „Rampe“ (das Tempo gleitet vom vorigen Punkt). Mit Tempospur zeigt die Transportleiste das Tempo am Cursor. Taktwechsel stehen im selben Inspector („Taktwechsel am Cursor“, Zähler/Nenner je Takt) und als Beschriftung in der Tempospur; Lineal, Raster, Snap auf Takt/Zählzeit, Positionsanzeige, Nummernblock −/+ und Metronom folgen ihnen. Events behalten ihre Ticks; Dauern zählen in der Taktart von Takt 1. Audio auf Spuren mit linearer Zeitbasis behält bei jeder Tempoänderung seine Zeit, MIDI seine Takte; Metronom und Export folgen der Kurve.

**Vorzähler und Metronom:** Metronom an/aus mit C; Vorzähler in Takten einstellbar. Klick-Sound, Lautstärke und Ausgang sind Projekteinstellungen. Transport → Metronom: Vorzähler (aus, 1, 2, 4 Takte), Lautstärke, „Takt-Eins betonen“ und Klang (hell, dunkel). Der Vorzähler klickt auch bei ausgeschaltetem Metronom, der Cursor wartet, Mithören und Instrumente laufen; die Aufnahme beginnt sample-genau an seinem Ende, Stopp im Vorzähler verwirft sie. Was der Eingang während des Vorzählers liefert, gehört nicht zum Take. Ein eigener Ausgang für den Klick kommt mit dem Routing der Ausgänge.

**Aufnahmemodi**

| Situation | Audio | MIDI |
| --- | --- | --- |
| Neue Aufnahme über bestehende Events | Neue Lane, alte bleibt stumm erhalten | Neuer Part (Standard), alternativ Mischen oder Ersetzen |
| Aufnahme im Cycle | Pro Durchgang eine Lane (Takes) | Stacked (je Durchgang ein Part), Mix (alle in einem Part) oder Ersetzen |
| Nachträgliches Retrospective Record | – | Gespielte Noten ohne Aufnahme nachträglich als Part übernehmen (Umschalt + Nummernblock-Mal) |

Die MIDI-Modi stehen unter Transport → „MIDI-Aufnahme“ (Projekteinstellung; Standard: neuer Part, im Cycle gestapelt). „Nachträgliche Aufnahme“ (Umschalt + Nummernblock-Mal, auch im Menü Transport) übernimmt die zuletzt gespielten, nicht aufgenommenen Noten der Keyboard-Spur als neuen Part: während der Wiedergabe an ihrer Position (im Cycle der letzte Durchgang), im Stopp ab dem Cursor im gespielten Abstand.

**Takes und Lanes:** Takes einer Spur liegen in Lanes unter ihr. Mit dem Comp-Werkzeug wählt man pro Abschnitt den besten Take; die Spur spielt nur die gewählten Abschnitte. Comping ist seit 0.7 da (Comp-Werkzeug, kurze Crossfades an den Abschnittsgrenzen).

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

**Latenzausgleich:** Plugins melden ihre Latenz (CLAP `latency`, auch wenn sie sich ändert). Jeder Weg – Spur zum Ausgang oder zur Gruppe, Send zum FX-Kanal, Gruppe und FX-Kanal zum Ausgang – wird so verzögert, dass alles gleichzeitig am Ausgang ankommt. Die Latenz einer Spur steht im Inspector unter Inserts, der gesamte Ausgleich im Menü Transport. „Latenzausgleich begrenzen“ (Menü Transport) umgeht bei aufnahmebereiten und mitgehörten Spuren Plugins mit mehr als 256 Samples Latenz, ohne das Projekt zu ändern.

Die Insert-Aufteilung Pre/Post ist eine Projekt-Voreinstellung; jeder Slot ist einzeln schaltbar (an/aus, Bypass).

**Plugin im eigenen Prozess:** Im Menü „…“ eines Insert-Slots schaltet „In eigenem Prozess“ das Plugin in einen Hilfsprozess (nicht in der Web-Version). Stürzt es dort ab oder hängt es, verstummt nur dieser Insert; die Meldung nennt das Plugin, „Neu laden“ startet es mit dem Zustand aus dem Projekt neu. Im Kontextmenü der Plugin-Liste setzt „Immer in eigenem Prozess“ die Voreinstellung für neue Inserts dieses Plugins (in den Einstellungen gemerkt; bestehende Inserts bleiben, wie sie sind). Das Fenster eines solchen Plugins öffnet der Hilfsprozess (unter X11 über dem Hauptfenster); Reglerbewegungen darin kommen wie sonst im Projekt an. Beim Abspielen läuft es einen Block hinterher (Teil der gemeldeten Latenz), beim Export und Rendern wartet es auf jeden Block. Eine Plugin-Datei, die beim Scan abstürzt oder hängt, kommt auf die Sperrliste; hat der Scan ihre Plugins vor dem Absturz vollständig gelesen (etwa bei einem Absturz erst beim Entladen), stehen sie mit dem Zusatz „eigener Prozess“ in der Plugin-Liste und laufen nur dort.

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
  (Menü Transport → „Audio-Anschlüsse …“: Name, Mono/Stereo und erster Gerätekanal je Bus, bis zu acht je Richtung; der Hauptmix „Stereo Out“ bleibt immer. Spuren wählen im Inspector einen Eingangsbus und als Ausgang den Hauptmix, einen weiteren Ausgangsbus oder eine Gruppe. Entfernte Busse fallen auf den ersten Eingang bzw. den Hauptmix zurück.)
- Ein Projekt hat mindestens einen Stereo-Ausgangsbus (Main Mix)
- Gruppen können in Gruppen geroutet werden; das System verhindert Schleifen
- Direktes Routing auf mehrere Ziele gleichzeitig: Unter dem Ausgang fügt „+ Weiterer Ausgang“ bis zu sieben weitere Ziele hinzu (Ausgangsbusse und Gruppen). Sie bekommen dasselbe Signal nach dem Fader, ohne eigenen Pegel, jedes mit eigenem Latenzausgleich; Schleifen sind ausgeschlossen.

**Sichtbarkeit und Ansicht**

- Linke Zone: Liste aller Kanäle mit Sichtbarkeits-Haken
- Rack-Ansicht oberhalb der Fader blendet Inserts, EQ, Sends ein und aus
- Kanalbreite schmal/breit; Konfigurationen der Sichtbarkeit speicherbar

**Link-Gruppen (seit 0.6):** Ausgewählte Kanäle lassen sich verknüpfen, sodass Fader, Mute und Solo gemeinsam reagieren.

## Editoren

Doppelklick auf ein Event öffnet den Standard-Editor seines Typs: Audio → Sample-Editor, MIDI → Key-Editor (oder Drum-Editor, wenn der Spur eine Drum-Map zugewiesen ist). Jeder Editor kann als eigenes Fenster oder in der unteren Zone geöffnet werden (Strg + E bzw. Voreinstellung).

| Editor | Zeigt | Kernfunktionen | MVP |
| --- | --- | --- | --- |
| Key-Editor | Noten als Balken über einer Klaviatur, darunter Controller-Lanes | Zeichnen, Länge ziehen, Velocity in Lane, Quantisieren, Transponieren mit Pfeil hoch/runter | ja |
| Drum-Editor | Noten als Rauten pro Instrument-Zeile, Namen aus der Drum-Map | Eigene Quantisierung pro Zeile, Länge ignoriert, Mute/Solo pro Zeile | 0.6 (Drum-Maps bearbeiten: #161) |
| List-Editor | Alle Events als Tabelle (Position, Länge, Wert, Kanal) | Werte direkt eingeben, filtern nach Event-Typ | 0.6 |
| Sample-Editor | Wellenform eines Clips in voller Größe | Bereich wählen, Offline-Effekte, Hitpoints, Tempo-Erkennung | ja (ohne Hitpoints) |
| Noten-Editor | Partitur | Anzeige und Bearbeiten (0.9), MusicXML- und PDF-Export (#183) | 0.9 |

**Gemeinsames Verhalten**

- Eigene Werkzeugzeile mit den passenden Werkzeugen aus Abschnitt Werkzeuge
- Eigene Info-Zeile: Werte der Auswahl (Start, Länge, Tonhöhe, Velocity) sind direkt editierbar; bei Mehrfachauswahl relativ
- Akustisches Feedback: angeklickte oder verschobene Noten erklingen (abschaltbar)
- Mehrere Parts gleichzeitig öffnen; ein aktiver Part, die anderen gedämpft
- „Editor folgt Wiedergabe“ scrollt automatisch mit (Taste F)

**Noten-Editor (seit 0.9, Anzeige):** Zeigt den aktiven MIDI-Part als Partitur, in der unteren Zone (Reiter „Noten-Editor“) oder im eigenen Fenster (Ansicht → „Noten-Editor im eigenen Fenster“). Der Schlüssel folgt dem Tonumfang: Violin- oder Bassschlüssel, bei großem Umfang ein Klaviersystem mit Trennung beim eingestrichenen c. Taktarten kommen aus dem Projekt, die Tonart ist eine Projekteinstellung (Werkzeugzeile, rückgängig machbar). Die Anzeige-Quantisierung (1/4 bis 1/32, je Part gespeichert) rundet Anfang und Dauer nur für die Darstellung. Notenwerte zeigen den Schlag; was über einen Schlag oder Taktstrich hinausreicht, wird geteilt und übergebunden. Achtel und kürzere Werte werden je Schlag (bei 6/8, 9/8, 12/8 je drei Achtel) mit Balken verbunden. Vorzeichen gelten bis zum Taktende, Auflösungszeichen werden gesetzt. Leere Takte zeigen eine ganze Pause. Je Notensystem eine Stimme: Noten, die über den nächsten Einsatz hinaus klingen, werden dort beendet. Der Cursor läuft mit; „Folgen“ blättert beim Abspielen mit. Notenschrift: Leland (SIL OFL 1.1).

**Noten-Editor bearbeiten (seit 0.9):** Klick auf einen Notenkopf wählt die Note (Umschalt fügt hinzu oder nimmt weg); die Auswahl ist dieselbe wie im Key-Editor. Ziehen verschiebt die gewählten Noten in der Zeit (im Raster der Anzeige-Quantisierung) und auf den Linien (Stufen der Tonart). Das Zeichenwerkzeug setzt mit Klick eine Note an Linie und Zählzeit, mit dem Notenwert aus der Werkzeugzeile (Ganze bis Zweiunddreißigstel, punktiert); der Radierer und Entf löschen. Ein anderer Notenwert gilt auch für die gewählten Noten. „Kreuz“ und „Be“ verschieben um einen Halbton und schreiben die Note so, „Enharmonisch verwechseln“ tauscht Kreuz und Be (die Schreibweise wird je Note gespeichert). Jede Änderung ist ein Rückgängig-Schritt und landet im MIDI-Part; die Anzeige-Quantisierung ändert die gespeicherten Zeiten nicht.

**Controller-Lanes im Key-Editor:** Beliebig viele Lanes, jede zeigt einen Datentyp (Velocity, Pitchbend, CC 1–127, Aftertouch). Zeichnen mit Linien-, Parabel-, Sinus- und Rechteck-Formen (seit 0.6).

## Automation

Jeder Parameter eines Kanals oder Plugins kann automatisiert werden; die Kurve liegt auf einer Automationsspur direkt unter ihrer Spur. R (Read) spielt die Kurve ab, W (Write) zeichnet Bewegungen der Bedienelemente während der Wiedergabe auf.

**Schreibmodi (bei aktivem W)**

| Modus | Verhalten | MVP |
| --- | --- | --- |
| Touch | Schreibt, solange das Bedienelement gehalten wird; danach zurück zur bestehenden Kurve | ja |
| Auto-Latch | Schreibt ab Berührung bis zum Stopp mit dem letzten Wert | ja |
| Overwrite | Schreibt ab Start nur für die Lautstärke, unabhängig von Berührung | ja |
| Trim | Verändert die bestehende Kurve relativ | ja |

**Automationsspuren**

- Pfeil im Spurkopf klappt die erste Automationsspur auf; Plus darunter fügt die nächste hinzu
- Bei aktivem W werden Spuren für neu bewegte Parameter automatisch angelegt
- Eine Automationsspur kann stummgeschaltet werden; der Parameter steht dann auf seinem statischen Wert

**Kurvenbearbeitung**

- Klick mit Objektauswahl auf die Linie setzt einen Punkt; Ziehen bewegt Punkte
- Zeichnen-Werkzeug zeichnet frei oder mit Formen (Linie, Parabel, Sinus, Rechteck; Rechtsklick auf das Werkzeug)
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
| Audio-Backend | cpal: PipeWire, ALSA, JACK (Linux, FreeBSD), WASAPI (Windows; dazu eigene Hosts für WASAPI exklusiv und mit geringer Latenz), CoreAudio (macOS), Web Audio (WASM). ASIO nicht in den Paketen (SDK-Lizenz), nur als Feature im Code |
| Lizenzierung | keine Aktivierung; eigener Code MIT oder Apache-2.0, Lizenzhinweise aller Komponenten in THIRD-PARTY-LICENSES.txt |
| Projektformat | offenes, versioniertes Textformat plus Audio-Ordner; diff- und git-freundlich |
| Plattform | Linux, Windows, macOS, FreeBSD und Web (WASM), Pakete nach den Konventionen der Craft-Apps |

**MVP-Umfang**

Stand 2026-11-20 (Version 0.6.1, 0.7 in Arbeit); offen heißt: fehlt oder ist nicht auf echter
Hardware geprüft. Alle Punkte bis auf die Latenzmessung sind umgesetzt und durch Tests abgedeckt.

- [ ] Audio-Engine über cpal auf allen Systemen, Latenz ≤ 10 ms bei 48 kHz / 256 Samples
  (Engine, Blockgröße und Latenzausgleich vorhanden; die Latenz ist auf Hardware nicht gemessen,
  unter Windows fehlt der Test der eigenen WASAPI-Hosts: #152)
- [x] Projektfenster mit Spurliste, Event-Anzeige, Lineal, Inspector, Transportleiste
- [x] Spurtypen Audio, Instrument, MIDI, Gruppe, FX, Ordner, Marker, Tempo
- [x] Werkzeuge 1 bis 0, Snap, Raster, Fades, Crossfades (Werkzeug 0 Zeitkorrektur seit 0.7)
- [x] Aufnahme Audio und MIDI inkl. Cycle-Takes, Punch, Metronom
- [x] Mixer mit Inserts, Sends, Kanal-EQ, Gruppen, Bussen
- [x] Plugin-Hosting mit Plugin-GUI: CLAP, VST3, LV2, AU (nur macOS)
- [x] Key-Editor und Sample-Editor (dazu Drum- und List-Editor)
- [x] Automation Read/Write mit Touch und Auto-Latch (dazu Overwrite und Trim)
- [x] Frei belegbare Tastatur mit Standardbelegung
- [x] Export als WAV, FLAC und AIFF (Mixdown der Locator-Range, Stems)

**Über den MVP hinaus umgesetzt:** Standard-MIDI-Dateien (0.6), Audio folgt dem Tempo mit
Time-Stretch und Warp-Markern (0.7), Hitpoints und Tempo-Erkennung (0.7), Offline-Bearbeitung je
Clip (0.7), Pakete für alle Plattformen und den Browser (0.5), englische Oberfläche (0.5).

**Offene Fragen**

- [ ] Projektname: entschieden, StudioCraft (Repo unter Dokumente/GitHub/StudioCraft). Verworfen: SoundCraft (in der Familie vergeben), AudioCraft (Meta), MixCraft (bestehende DAW), RecordCraft (Rust-Recorder kaptaindieter/RecordCraft). Markenrecherche steht noch aus.
- [x] GUI-Toolkit: entschieden, egui/eframe mit craft-ui wie die Craft-Apps (nativ und Web). Prototyp bestätigt: 0,15 ms CPU pro Frame bei 100 und 1000 Spuren (Release-Build)
- [x] Plugin-GUIs unter Wayland (#175, 0.8): CLAP-Plugins öffnen zuerst ein eigenes
  Wayland-Fenster, sonst bettet StudioCraft den Editor über XWayland in ein eigenes Fenster ein
  (bleibt über dem Hauptfenster, merkt sich seine Position je Plugin-Fenster, solange die App
  läuft), zuletzt ein eigenes X11-Fenster des Plugins. LV2 und VST3 betten über XWayland ein.
  Unter X11 hängen Plugin-Fenster am Hauptfenster (`WM_TRANSIENT_FOR`). Ohne XWayland gibt es
  nur die Wayland-Fenster von CLAP-Plugins.
- [x] Standard-MIDI-Dateien: entschieden, nicht im MVP, sondern in 0.6 (Issue #47). Datei → MIDI-Datei importieren legt je Spur und Kanal der Datei eine MIDI-Spur mit einem Part an (Noten, CC, Pitchbend, Aftertouch, erstes Programm); in einem Projekt ohne Events gelten danach Tempo und Taktarten der Datei, sonst beginnen die Parts am Takt des Cursors. Datei → MIDI-Datei exportieren schreibt Format 1 mit Tempospur und je MIDI- und Instrument-Spur einer Spur (stumme Parts und ausgeblendete Noten fehlen, Tempo-Rampen als Stufen je Sechzehntel). AAF/OMF-Import kommt später (Issue #154).
- [ ] Wer übernimmt die Spec-Autor-Rolle, wer implementiert (Cleanroom-Trennung)?
