# Video-Dekodierung: Formate, Bibliotheken, Plattformen

Entscheidungsvorlage für die Video-Spur (#174), erstellt für #185. **Keine Rechtsberatung:**
Die Angaben beruhen auf öffentlichen Quellen (Stand 2026-11-21). Punkte mit **[Anwalt]** sollte
ein Fachanwalt für Patentrecht prüfen, bevor StudioCraft sie ausliefert.

## Worum es geht

Die Video-Spur zeigt Bilder eines Videos synchron zum Projekt (Thumbnails in der Spur, ein
Videofenster) und spielt optional dessen Ton. StudioCraft muss dafür **dekodieren**, nicht
kodieren; Export von Video ist nicht geplant.

Zwei getrennte Fragen:

1. **Urheberrecht/Lizenz der Bibliothek**: Sie muss auf der Liste in `AGENTS.md` stehen (MIT,
   Apache-2.0, BSD …). GPL/LGPL (etwa FFmpeg, libmpeg2, x264) scheidet aus.
2. **Patente auf das Format**: Auch eine eigene oder permissiv lizenzierte Implementierung kann
   Patente verletzen. Wer einen Decoder ausliefert, kann lizenzpflichtig sein. Nutzt die App den
   Decoder des Betriebssystems oder Browsers, hat in der Regel dessen Hersteller die Lizenz.

## Formate

| Format | Patentlage | Einschätzung |
| --- | --- | --- |
| AV1 | Lizenzfreie Patentlizenz der Alliance for Open Media. Sisvel betreibt dennoch einen AV1-Pool, der Gebühren je Gerät verlangt (Hardware, Anzeige- und andere Geräte); Forderungen für Software-Decoder in Anwendungen sind nicht bekannt. | **geeignet**, Restrisiko Sisvel **[Anwalt]** |
| VP8, VP9 | Lizenzfreie Patentzusage von Google. Sisvel-Pool für VP9 wie bei AV1 (zielt auf Geräte). | **geeignet**, Restrisiko wie AV1 **[Anwalt]** |
| Theora | Xiph, lizenzfrei. Kaum noch verbreitet. | geeignet, geringer Nutzen |
| Motion-JPEG | JPEG-Grundverfahren gelten als patentfrei. | geeignet (Bildschirmaufnahmen, ältere Kameras) |
| MPEG-2 Video | Patente weltweit abgelaufen. Es gibt aber keine permissiv lizenzierte Bibliothek (libmpeg2 ist GPL); ein eigener Decoder wäre nötig. | rechtlich möglich, Aufwand hoch |
| MPEG-4 Part 2 (DivX/Xvid) | Letztes Patent (Brasilien) am 19.07.2026 abgelaufen; US-Patente bis 2023, europäische bis 2021. | rechtlich möglich, keine permissive Bibliothek bekannt |
| H.264/AVC | Pool von Via LA. Außerhalb der USA sind die Patente laut öffentlichen Listen 2024–2025 abgelaufen. In den USA gilt der 29.11.2027 als wahrscheinliches Ende (US 7,826,532, Schätzung). Mindestens ein weiteres Pool-Patent läuft womöglich bis 2030. | **nicht selbst ausliefern**; nur über System-Decoder **[Anwalt]** für den Zeitpunkt danach |
| H.265/HEVC | Mehrere Pools (Access Advance, Via LA) und Einzellizenzgeber, Patente bis weit in die 2030er. | **nicht selbst ausliefern**; nur über System-Decoder |
| H.266/VVC | Wie HEVC, jünger. | nicht selbst ausliefern |
| ProRes, DNxHD | Herstellerformate; Lizenzbedingungen der Hersteller. | nur über System-Decoder (macOS) |

Ton in Videodateien:

| Format | Einschätzung |
| --- | --- |
| Opus, Vorbis, FLAC, PCM | lizenzfrei, geeignet |
| MP3, AC-3 | Patente abgelaufen (2017), geeignet |
| AAC | Pool von Via LA besteht weiter; ob für AAC-LC noch Patente in Kraft sind, ist nicht geklärt **[Anwalt]**. Bis dahin nur über System-Decoder. |

Container (MP4/ISOBMFF, Matroska/WebM, Ogg) sind als Dateiformate ohne bekannte Patentforderungen;
patentrelevant sind die enthaltenen Codecs.

## Bibliotheken (nur Lizenzen aus `AGENTS.md`)

| Zweck | Kandidat | Lizenz | Bemerkung |
| --- | --- | --- | --- |
| AV1 | `rav1d` (Rust-Portierung von dav1d) | BSD-2-Clause | reines Rust auf x86_64/aarch64 mit stabilem Compiler; arm und riscv64 brauchen derzeit nightly. AOM-Patentlizenz in die Hinweise aufnehmen. |
| AV1 | `dav1d` (C) über Bindings | BSD-2-Clause | Alternative; C-Bau in allen Paketen nötig |
| VP8/VP9 | libvpx über Bindings | BSD-3-Clause | C-Bau nötig; reine Rust-Decoder nicht geprüft |
| Motion-JPEG | `zune-jpeg` oder `jpeg-decoder` | MIT/Apache-2.0 bzw. MIT/Apache-2.0 | vor Aufnahme mit `cargo xtask licenses` prüfen |
| Container MP4 | `mp4` | MIT | vor Aufnahme prüfen |
| Container Matroska/WebM | `matroska-demuxer` | MIT/Apache-2.0 | vor Aufnahme prüfen |
| Ton | `symphonia` | MPL-2.0 (zulässig, unverändert) | enthält auch einen AAC-Decoder: diesen **nicht** einschalten, solange AAC ungeklärt ist (Cargo-Features) |
| H.264 | OpenH264 (Cisco) | BSD-2-Clause | Ciscos Übernahme der Gebühren gilt nur für **Ciscos eigene Binärdateien**, die beim Anwender heruntergeladen werden (so macht es Firefox). Ein Bau aus dem Quelltext ist nicht abgedeckt. Download zur Laufzeit bräuchte Netz und Zustimmung **[Anwalt]**. |
| alles | FFmpeg, GStreamer | LGPL/GPL | nicht einbinden; GStreamer allenfalls als Systembibliothek zur Laufzeit, siehe unten |

## Plattformen

| Plattform | System-Decoder | Was er abdeckt | Weg für StudioCraft |
| --- | --- | --- | --- |
| Windows | Media Foundation (`windows`-Crate, MIT/Apache) | H.264, AAC, MP4 ab Werk (Lizenz bei Microsoft); HEVC nur mit kostenpflichtiger Store-Erweiterung; AV1/VP9 je nach installierten Erweiterungen | System-Decoder für H.264/HEVC/AAC, eigene Decoder für AV1/VP9 |
| macOS | AVFoundation/VideoToolbox (`objc2`-Crates, schon im Baum) | H.264, HEVC, ProRes, AAC (Lizenz bei Apple); AV1 in Hardware erst auf neueren Geräten | System-Decoder für alles außer AV1/VP9 |
| Linux | GStreamer (LGPL), VA-API (Treiber) | hängt von der Distribution ab; manche (z. B. Fedora) liefern H.264/HEVC nicht aus oder nur über Ciscos OpenH264 | eigene Decoder für AV1/VP9; H.264/HEVC nur, wenn der Maintainer GStreamer als Laufzeit-Systembibliothek zulässt **[Maintainer]** |
| FreeBSD | wie Linux (Ports) | wie Linux | wie Linux |
| Web | `HTMLVideoElement`/WebCodecs im Browser | was der Browser kann (Lizenz beim Browser) | Browser-Decoder über `web-sys`, kein eigener |

## Empfehlung

1. **Erste Stufe:** AV1 und VP9 (WebM/Matroska, MP4) mit `rav1d` und libvpx bzw. einem
   späteren Rust-Decoder, Ton Opus/Vorbis/FLAC/MP3/PCM über `symphonia` ohne AAC; im Web der
   Browser-Decoder. Damit funktioniert die Video-Spur überall gleich und ohne Patentlizenzen
   (bis auf das Sisvel-Restrisiko).
2. **Zweite Stufe:** H.264, HEVC, ProRes und AAC über die System-Decoder unter Windows und
   macOS. StudioCraft liefert dafür keinen eigenen Decoder aus.
3. **Linux/FreeBSD H.264/HEVC:** offen, bis der Maintainer über GStreamer zur Laufzeit
   entscheidet. Bis dahin zeigt die App bei nicht unterstützten Formaten einen Hinweis
   („in AV1 oder VP9 umwandeln“), ohne ein bestimmtes Programm zu nennen.
4. **Nach Ablauf der US-Patente (frühestens Ende 2027):** H.264 mit eigenem oder permissivem
   Decoder neu bewerten **[Anwalt]**.

## Umsetzung (0.10, #174)

Die erste Stufe nutzt keine C-Bibliotheken und kein GStreamer: `craft-video` aus craft-libs
(MIT OR Apache-2.0, reines Rust, ohne `unsafe`) bringt Decoder für AV1, VP9, VP8 und MPEG-2,
`craft-media` die Container WebM/Matroska und MP4/MOV. Eingeschaltet sind nur diese Decoder
(Cargo-Features `av1-decode`, `vp9-decode`, `vp8-decode`, `mpeg2-decode`); die H.264-, HEVC-,
ProRes- und DNx-Module bleiben aus. Damit erledigen sich die beiden Maintainer-Fragen unten
(GStreamer, C-Abhängigkeiten) für die erste Stufe; der Ton eines Videos läuft über den
vorhandenen Audio-Import. Im Browser gibt es die Videospur noch nicht.

## Offene Risiken und Fragen

- **[Anwalt]** Sisvel-Pools für AV1/VP9: Gelten sie für Software-Decoder in einer kostenlosen
  Anwendung? Bisher sind nur Geräteforderungen bekannt.
- **[Anwalt]** AAC-LC: Sind noch Patente in Kraft, und in welchen Ländern?
- **[Anwalt]** H.264 in den USA: tatsächliches Ende des letzten Pool-Patents (Liste von Via LA
  prüfen; mögliches Patent bis 2030).
- **[Anwalt]** OpenH264-Download zur Laufzeit: Bedingungen von Cisco für Drittanwendungen.
- **[Maintainer]** GStreamer (LGPL) als Systembibliothek zur Laufzeit: vereinbar mit Regel 3/4 in
  `AGENTS.md`? (Kein Code wird kopiert oder gebündelt, aber die Regel nennt LGPL ausdrücklich.)
- **[Maintainer]** C-Abhängigkeiten (dav1d, libvpx) in allen Paketen, auch riscv64 und FreeBSD,
  oder nur reines Rust (`rav1d`, nightly auf arm/riscv64)?
- Flathub und Distributionen haben eigene Regeln für patentbehaftete Codecs; für Flatpak gibt
  es dafür Erweiterungen. Vor der zweiten Stufe prüfen.
- Die Patentlage ändert sich; dieses Dokument vor jedem Release mit Video neu prüfen.

## Quellen

- H.264-Patentlisten und Ablaufdaten: [Meta-Wiki: Have the patents for H.264 MPEG-4 AVC expired yet?](https://meta.wikimedia.org/wiki/Have_the_patents_for_H.264_MPEG-4_AVC_expired_yet%3F),
  [OSnews](https://www.osnews.com/story/24954/us-patent-expiration-for-mp3-mpeg-2-h264/),
  [Doom9-Forum](https://forum.doom9.org/showthread.php?t=184869),
  [Via-LA (Wikipedia)](https://en.wikipedia.org/wiki/Via-LA)
- MPEG-4 Part 2: [Lexology](https://www.lexology.com/library/detail.aspx?g=6dc8f39d-08b8-4d54-87e9-e1eadfcb72c3),
  [Linuxiac](https://linuxiac.com/the-last-mpeg-4-visual-patent-has-expired/)
- Sisvel AV1/VP9: [Streaming Media](https://www.streamingmedia.com/Articles/ReadArticle.aspx?ArticleID=139636),
  [Mux](https://www.mux.com/blog/did-sisvel-just-catch-aom-with-their-patents-down),
  [Lumenci](https://lumenci.com/blogs/hevc-vs-av1-vs-vvc-royalties-licensing-risk/)
- rav1d: [docs.rs](https://docs.rs/crate/rav1d/latest), [memorysafety.org](https://www.memorysafety.org/blog/safer-av1-decoder/),
  [GitHub](https://github.com/memorysafety/rav1d)
