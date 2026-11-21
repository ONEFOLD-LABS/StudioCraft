# Selbst-Update

StudioCraft sucht auf Wunsch nach neuen Versionen und installiert sie selbst, wo es seine
Dateien selbst verwaltet. Der Ablauf ist derselbe wie in den anderen Apps von ONEFOLD LABS.

## Suche

- Hilfe → „Täglich nach Updates suchen“ ist aus, bis man es einschaltet. Dann fragt StudioCraft
  höchstens einmal am Tag `https://api.github.com/repos/ONEFOLD-LABS/StudioCraft/releases/latest`.
  Gesendet wird nur diese Anfrage mit dem Programmnamen als User-Agent: keine Kennung, keine
  Systemdaten.
- Hilfe → „Jetzt nach Updates suchen“ fragt sofort und meldet auch „ist aktuell“ oder einen Fehler.
  Eine tägliche Suche ohne Netz bleibt still.
- Entwürfe und Vorabversionen zählen nicht.

## Was bei einer neuen Version geschieht

Unter dem Menü erscheint eine Leiste mit der Version, „Release-Seite“ und „Später“. Installiert
wird nur nach einem Klick auf „Installieren“.

| Installiert als | Bei einer neuen Version |
|---|---|
| AppImage (`APPIMAGE` gesetzt) | laden, prüfen, das AppImage ersetzen, Neustart anbieten |
| Tarball (Linux, FreeBSD) in einem beschreibbaren Ordner | laden, prüfen, `studiocraft` und `studiocraft-cli` ersetzen, Neustart anbieten |
| Portables Zip (Windows) in einem beschreibbaren Ordner | laden, prüfen, `studiocraft.exe` und `studiocraft-cli.exe` ersetzen, Neustart anbieten |
| Windows-MSI (unter Program Files) | laden, prüfen, das MSI starten; Windows Installer ersetzt die Dateien |
| deb, rpm, Arch, FreeBSD-pkg, Flatpak, macOS-App, Systemordner | nur Hinweis und Link; das Update kommt über die Paketverwaltung |

„Neu starten“ beendet StudioCraft wie Datei → Beenden (mit Rückfrage bei ungesicherten
Änderungen) und startet die neue Version. Eine ersetzte Datei bleibt bis zum nächsten Start als
`.old` liegen; im Browser gibt es kein Selbst-Update.

## Prüfung

Jede Release-Datei hat eine minisign-Signatur (`<datei>.minisig`, Ed25519). Der öffentliche
Schlüssel ist in StudioCraft eingebaut (`packaging/update-key.pub`). Installiert wird nur, wenn
die Signatur mit diesem Schlüssel stimmt und der signierte Kommentar genau
`studiocraft <version> <datei>` lautet, damit keine ältere oder andere Datei untergeschoben werden
kann. Builds ohne Schlüssel melden neue Versionen nur.

## Für Maintainer

- Schlüsselpaar einmal anlegen: `minisign -G -p update-key.pub -s update-key.key`.
- Den Inhalt von `update-key.pub` nach `packaging/update-key.pub` (per Pull Request).
- Den Inhalt von `update-key.key` als Secret `MINISIGN_SECRET_KEY` und das Passwort als
  `MINISIGN_PASSWORD` im Repository hinterlegen. Der geheime Schlüssel gehört nie ins Repository.
- Der Release-Workflow signiert dann jede Datei mit `packaging/sign-release.sh`. Ohne Secret
  entsteht nur `SHA256SUMS.txt`, und StudioCraft meldet das Release nur.
- Ein gestohlener Schlüssel erlaubt bösartige Updates für alle, die die Suche eingeschaltet haben.
  Gewechselt wird er mit einem Release, das den neuen öffentlichen Schlüssel enthält und noch mit
  dem alten signiert ist.
