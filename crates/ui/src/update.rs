//! Update-Suche (nur wenn eingeschaltet) und Selbst-Update, nach dem Muster der anderen
//! ONEFOLD-LABS-Apps (docs/selbst-update.md).
//!
//! - Gefragt wird GitHub nach dem neuesten Release, höchstens einmal am Tag und nur, wenn die
//!   Suche im Menü Hilfe eingeschaltet ist. Gesendet wird nur die Anfrage; Entwürfe und
//!   Vorabversionen zählen nicht.
//! - Wie StudioCraft installiert ist, entscheidet, was geschieht: ein AppImage, die Programme aus
//!   dem Tarball und die portable Windows-Fassung werden an Ort und Stelle ersetzt, das
//!   Windows-MSI wird gestartet (Windows Installer ersetzt die Dateien); Paketverwaltung,
//!   Flatpak und die macOS-App bekommen nur einen Hinweis mit Link.
//! - Installiert wird nur, was mit dem eingebauten öffentlichen Schlüssel
//!   (`packaging/update-key.pub`) signiert ist und dessen signierter Kommentar genau Datei und
//!   Version nennt. Builds ohne Schlüssel melden nur.
//! - Eine neue Datei wird neben ihr Ziel geschrieben und in einem Umbenennen eingesetzt; die
//!   bisherige bleibt als `.old` bis zum nächsten Start.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Wo nach dem neuesten Release gefragt wird.
pub(crate) const RELEASES: &str =
    "https://api.github.com/repos/ONEFOLD-LABS/StudioCraft/releases/latest";
/// Die Release-Seite für Hinweise.
pub(crate) const RELEASE_PAGE: &str = "https://github.com/ONEFOLD-LABS/StudioCraft/releases/latest";
/// Der Schlüssel, mit dem Updates signiert sein müssen; leer (nur Kommentare) ohne Schlüssel.
const PUBLIC_KEY: &str = include_str!("../../../packaging/update-key.pub");
/// Größter angenommener Download (ein AppImage hat einige zehn MB).
const MAX_DOWNLOAD: u64 = 512 * 1024 * 1024;
/// Größte angenommene Release-Beschreibung.
const MAX_JSON: u64 = 2 * 1024 * 1024;
/// Größte angenommene Signaturdatei.
const MAX_SIGNATURE: u64 = 64 * 1024;
/// Die Programme, die ein Update ersetzt (ohne `.exe`).
const PROGRAMS: [&str; 2] = ["studiocraft", "studiocraft-cli"];

/// Eine Version, wie StudioCraft sie vergleicht: drei Zahlen, dann ein Zusatz wie `-rc.1`, der
/// vor der Version selbst einsortiert wird.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Version {
    numbers: (u64, u64, u64),
    pre: Option<String>,
}

impl Version {
    pub(crate) fn parse(text: &str) -> Option<Version> {
        let text = text.trim().trim_start_matches('v');
        let (core, pre) = match text.split_once('-') {
            Some((c, p)) => (c, Some(p.to_owned())),
            None => (text, None),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let numbers = (parts.next()??, parts.next()??, parts.next()??);
        if parts.next().is_some() {
            return None;
        }
        Some(Version { numbers, pre })
    }

    pub(crate) fn current() -> Option<Version> {
        Version::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (a, b, c) = self.numbers;
        write!(f, "{a}.{b}.{c}")?;
        if let Some(p) = &self.pre {
            write!(f, "-{p}")?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.numbers
            .cmp(&other.numbers)
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asset {
    pub(crate) name: String,
    pub(crate) url: String,
}

/// Ein veröffentlichtes Release, neuer als dieser Build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) version: Version,
    pub(crate) page: String,
    pub(crate) assets: Vec<Asset>,
}

/// Wie diese Kopie installiert ist und was ein Update daher tun kann.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Install {
    /// Ein AppImage an diesem Ort: wird durch das neue ersetzt.
    AppImage(PathBuf),
    /// Das Programm aus dem Linux- oder FreeBSD-Tarball in einem beschreibbaren Ordner.
    Tarball(PathBuf),
    /// Die portable Windows-Fassung (zip) in einem beschreibbaren Ordner.
    Portable(PathBuf),
    /// Vom Windows-MSI installiert: das neue MSI wird gestartet.
    Msi,
    /// Paketverwaltung, Flatpak, die macOS-App oder ein schreibgeschützter Ordner: nur ein Hinweis.
    Managed,
}

fn writable_dir(exe: &Path) -> bool {
    exe.parent()
        .and_then(|d| std::fs::metadata(d).ok())
        .is_some_and(|m| !m.permissions().readonly())
}

/// Entscheidet aus dem Programmort, wie StudioCraft installiert ist.
pub(crate) fn classify(exe: &Path, appimage: Option<&Path>, flatpak: bool, os: &str) -> Install {
    if flatpak {
        return Install::Managed;
    }
    if let Some(a) = appimage {
        return Install::AppImage(a.to_path_buf());
    }
    let shown = exe.to_string_lossy().to_lowercase().replace('\\', "/");
    match os {
        "windows" if shown.contains("/program files") => Install::Msi,
        "windows" if writable_dir(exe) => Install::Portable(exe.to_path_buf()),
        "linux" | "freebsd" => {
            let system = [
                "/usr/bin/",
                "/usr/local/bin/",
                "/usr/lib/",
                "/opt/",
                "/app/",
                "/nix/",
            ]
            .iter()
            .any(|p| shown.starts_with(p));
            if system || !writable_dir(exe) {
                Install::Managed
            } else {
                Install::Tarball(exe.to_path_buf())
            }
        }
        _ => Install::Managed,
    }
}

/// Die Installationsart dieses Prozesses.
pub(crate) fn this_install() -> Install {
    let Ok(exe) = std::env::current_exe() else {
        return Install::Managed;
    };
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let flatpak = std::env::var_os("FLATPAK_ID").is_some();
    classify(&exe, appimage.as_deref(), flatpak, std::env::consts::OS)
}

/// Name der Release-Datei für eine Installationsart auf dieser Architektur.
pub(crate) fn file_name(
    version: &Version,
    install: &Install,
    os: &str,
    arch: &str,
) -> Option<String> {
    let windows = if arch == "aarch64" { "arm64" } else { "x64" };
    Some(match (install, os) {
        (Install::AppImage(_), _) => format!("studiocraft-{version}-linux-{arch}.AppImage"),
        (Install::Tarball(_), "freebsd") => format!("studiocraft-{version}-freebsd-{arch}.tar.gz"),
        (Install::Tarball(_), _) => format!("studiocraft-{version}-linux-{arch}.tar.gz"),
        (Install::Portable(_), _) => {
            format!("studiocraft-{version}-windows-{windows}-portable.zip")
        }
        (Install::Msi, _) => format!("studiocraft-{version}-windows-{windows}.msi"),
        (Install::Managed, _) => return None,
    })
}

/// Release-Datei und ihre Signatur für eine Installationsart.
pub(crate) fn assets_for(
    release: &Release,
    install: &Install,
    os: &str,
    arch: &str,
) -> Option<(Asset, Asset)> {
    let name = file_name(&release.version, install, os, arch)?;
    let file = release.assets.iter().find(|a| a.name == name)?.clone();
    let sig_name = format!("{name}.minisig");
    let sig = release.assets.iter().find(|a| a.name == sig_name)?.clone();
    Some((file, sig))
}

/// Liest die Release-Beschreibung von GitHub; `None` für Entwürfe, Vorabversionen und alles,
/// was nicht neuer als `current` ist.
pub(crate) fn parse_release(json: &[u8], current: &Version) -> Result<Option<Release>, String> {
    let v: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let Some(version) = v["tag_name"].as_str().and_then(Version::parse) else {
        return Err("release without a version".into());
    };
    if version <= *current {
        return Ok(None);
    }
    let page = v["html_url"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or(RELEASE_PAGE)
        .to_owned();
    let assets = v["assets"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    Some(Asset {
                        name: a["name"].as_str()?.to_owned(),
                        url: a["browser_download_url"].as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Release {
        version,
        page,
        assets,
    }))
}

/// Die Stammzertifikate des Systems, einmal gelesen.
fn roots() -> &'static [ureq::tls::Certificate<'static>] {
    static ROOTS: OnceLock<Vec<ureq::tls::Certificate<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        rustls_native_certs::load_native_certs()
            .certs
            .iter()
            .map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned())
            .collect()
    })
}

/// Ein HTTP-Agent mit festen Fristen und TLS über rustls mit den Zertifikaten des Systems.
pub(crate) fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::new_with_certs(roots()))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    ureq::Agent::config_builder()
        .proxy(ureq::Proxy::try_from_env())
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(600)))
        .user_agent("StudioCraft")
        .max_redirects(5)
        .build()
        .into()
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut resp = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    resp.body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|e| e.to_string())
}

/// Fragt `url` nach dem neuesten Release; `None`, wenn es nichts Neueres gibt.
pub(crate) fn check(agent: &ureq::Agent, url: &str) -> Result<Option<Release>, String> {
    let current = Version::current().ok_or("unreadable own version")?;
    parse_release(&fetch(agent, url, MAX_JSON)?, &current)
}

/// Der eingebaute öffentliche Schlüssel, falls dieser Build einen hat.
pub(crate) fn public_key() -> Option<minisign_verify::PublicKey> {
    key_from(PUBLIC_KEY)
}

fn key_from(text: &str) -> Option<minisign_verify::PublicKey> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("untrusted comment") && !l.starts_with('#'))
        .find_map(|l| minisign_verify::PublicKey::from_base64(l).ok())
}

/// Prüft `data` gegen die Signatur: gültig für `key`, und der signierte Kommentar lautet genau
/// `studiocraft <version> <dateiname>` (so lässt sich keine alte oder andere Datei unterschieben).
pub(crate) fn verify(
    key: &minisign_verify::PublicKey,
    data: &[u8],
    signature: &str,
    version: &Version,
    name: &str,
) -> Result<(), String> {
    let sig = minisign_verify::Signature::decode(signature).map_err(|e| e.to_string())?;
    key.verify(data, &sig, false).map_err(|e| e.to_string())?;
    let expected = format!("studiocraft {version} {name}");
    if sig.trusted_comment() != expected {
        return Err(format!(
            "signed for {:?}, expected {expected:?}",
            sig.trusted_comment()
        ));
    }
    Ok(())
}

/// Was nach dem Installieren zu tun ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Installed {
    /// Diese Datei starten, um die neue Version zu benutzen.
    Restart(PathBuf),
    /// Das Installationsprogramm läuft; StudioCraft sollte sich schließen.
    InstallerStarted,
}

/// Ersetzt `target` durch `bytes` in einem Umbenennen; die alte Datei bleibt als `<target>.old`.
pub(crate) fn replace(target: &Path, bytes: &[u8]) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", target.display());
    let name = target
        .file_name()
        .ok_or_else(|| format!("{}: no file name", target.display()))?
        .to_string_lossy()
        .into_owned();
    let dir = target.parent().unwrap_or(Path::new("."));
    let new = dir.join(format!("{name}.new"));
    let old = dir.join(format!("{name}.old"));
    std::fs::write(&new, bytes).map_err(io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755)).map_err(io)?;
    }
    let _ = std::fs::remove_file(&old);
    // Ein laufendes Programm darf man umbenennen (auch unter Windows), nicht überschreiben.
    std::fs::rename(target, &old).map_err(io)?;
    if let Err(e) = std::fs::rename(&new, target) {
        // Lieber die alte Datei zurück als gar keine.
        let _ = std::fs::rename(&old, target);
        return Err(io(e));
    }
    Ok(())
}

/// Die Programme aus einem Release-Tarball: `<ordner>/bin/<programm>`.
pub(crate) fn programs_from_tarball(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut found = Vec::new();
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.len() == 3 && parts[1] == "bin" && PROGRAMS.contains(&parts[2].as_str()) {
            let mut out = Vec::new();
            entry
                .by_ref()
                .take(MAX_DOWNLOAD)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            found.push((parts[2].clone(), out));
        }
    }
    if !found.iter().any(|(n, _)| n == PROGRAMS[0]) {
        return Err("no bin/studiocraft in the archive".into());
    }
    Ok(found)
}

/// Die Programme aus dem portablen Windows-Zip: `<ordner>/<programm>.exe`.
pub(crate) fn programs_from_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(path) = file.enclosed_name() else {
            continue;
        };
        let parts: Vec<String> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let Some(program) = parts.last().and_then(|n| n.strip_suffix(".exe")) else {
            continue;
        };
        if parts.len() == 2 && PROGRAMS.contains(&program) {
            let mut out = Vec::new();
            file.by_ref()
                .take(MAX_DOWNLOAD)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            found.push((format!("{program}.exe"), out));
        }
    }
    if !found.iter().any(|(n, _)| n == "studiocraft.exe") {
        return Err("no studiocraft.exe in the archive".into());
    }
    Ok(found)
}

/// Setzt die Programme neben `exe` ein: `exe` selbst immer, die übrigen nur, wo sie schon liegen.
fn replace_programs(exe: &Path, programs: &[(String, Vec<u8>)]) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let own = exe.file_name().map(|n| n.to_string_lossy().into_owned());
    for (name, bytes) in programs {
        let target = dir.join(name);
        if Some(name) == own.as_ref() {
            replace(exe, bytes)?;
        } else if target.exists() {
            replace(&target, bytes)?;
        }
    }
    Ok(())
}

/// Lädt, prüft und installiert `release` für `install`.
pub(crate) fn install(
    agent: &ureq::Agent,
    release: &Release,
    install: &Install,
) -> Result<Installed, String> {
    let key = public_key().ok_or("this build has no update key")?;
    let (file, sig) = assets_for(
        release,
        install,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
    .ok_or("no file for this installation in the release")?;
    let signature = String::from_utf8(fetch(agent, &sig.url, MAX_SIGNATURE)?)
        .map_err(|_| "unreadable signature".to_owned())?;
    let bytes = fetch(agent, &file.url, MAX_DOWNLOAD)?;
    verify(&key, &bytes, &signature, &release.version, &file.name)?;
    match install {
        Install::AppImage(path) => {
            replace(path, &bytes)?;
            Ok(Installed::Restart(path.clone()))
        }
        Install::Tarball(exe) => {
            replace_programs(exe, &programs_from_tarball(&bytes)?)?;
            Ok(Installed::Restart(exe.clone()))
        }
        Install::Portable(exe) => {
            replace_programs(exe, &programs_from_zip(&bytes)?)?;
            Ok(Installed::Restart(exe.clone()))
        }
        Install::Msi => {
            let path = std::env::temp_dir().join(&file.name);
            std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            std::process::Command::new("msiexec")
                .arg("/i")
                .arg(&path)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(Installed::InstallerStarted)
        }
        Install::Managed => Err("updates come from the package manager".into()),
    }
}

/// Entfernt die `.old`-Dateien, die ein früheres Update neben den Programmen gelassen hat.
pub(crate) fn clean_up() {
    let target = match this_install() {
        Install::AppImage(p) | Install::Tarball(p) | Install::Portable(p) => p,
        _ => return,
    };
    let Some(dir) = target.parent() else { return };
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let mut names: Vec<String> = PROGRAMS.iter().map(|p| format!("{p}{ext}")).collect();
    if let Some(n) = target.file_name() {
        names.push(n.to_string_lossy().into_owned());
    }
    for name in names {
        let _ = std::fs::remove_file(dir.join(format!("{name}.old")));
    }
}

/// Seit dem 1.1.1970 vergangene Sekunden.
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Ist eine tägliche Suche fällig?
pub(crate) fn due(last: u64, now: u64) -> bool {
    now.saturating_sub(last) >= 24 * 3600
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/update");

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sc-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn versions_compare_like_releases() {
        assert!(v("v0.10.0") > v("0.9.9"));
        assert!(v("0.9.0") > v("0.9.0-rc.2"));
        assert!(v("0.9.0-rc.2") > v("0.9.0-rc.1"));
        assert_eq!(v("1.2.3").to_string(), "1.2.3");
        assert_eq!(Version::parse("1.2"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::parse("x"), None);
        assert!(Version::current().is_some());
    }

    #[test]
    fn install_kinds() {
        let tmp = temp("kinds");
        let exe = tmp.join("studiocraft");
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(
            classify(&exe, None, false, "linux"),
            Install::Tarball(exe.clone())
        );
        assert_eq!(
            classify(&exe, None, false, "freebsd"),
            Install::Tarball(exe.clone())
        );
        assert_eq!(
            classify(Path::new("/usr/bin/studiocraft"), None, false, "linux"),
            Install::Managed
        );
        assert_eq!(
            classify(
                Path::new("/opt/studiocraft/bin/studiocraft"),
                None,
                false,
                "linux"
            ),
            Install::Managed
        );
        assert_eq!(
            classify(
                &exe,
                Some(Path::new("/home/u/StudioCraft.AppImage")),
                false,
                "linux"
            ),
            Install::AppImage(PathBuf::from("/home/u/StudioCraft.AppImage"))
        );
        assert_eq!(
            classify(&exe, None, true, "linux"),
            Install::Managed,
            "Flatpak"
        );
        assert_eq!(
            classify(
                Path::new(r"C:\Program Files\StudioCraft\studiocraft.exe"),
                None,
                false,
                "windows"
            ),
            Install::Msi
        );
        assert_eq!(
            classify(&exe, None, false, "windows"),
            Install::Portable(exe.clone())
        );
        assert_eq!(
            classify(
                Path::new("/Applications/StudioCraft.app/Contents/MacOS/studiocraft"),
                None,
                false,
                "macos"
            ),
            Install::Managed
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn file_names_match_the_packages() {
        let r = v("1.2.3");
        let p = PathBuf::from("/x");
        let name = |i: &Install, os, arch| file_name(&r, i, os, arch);
        assert_eq!(
            name(&Install::AppImage(p.clone()), "linux", "aarch64").as_deref(),
            Some("studiocraft-1.2.3-linux-aarch64.AppImage")
        );
        assert_eq!(
            name(&Install::Tarball(p.clone()), "linux", "riscv64").as_deref(),
            Some("studiocraft-1.2.3-linux-riscv64.tar.gz")
        );
        assert_eq!(
            name(&Install::Tarball(p.clone()), "freebsd", "x86_64").as_deref(),
            Some("studiocraft-1.2.3-freebsd-x86_64.tar.gz")
        );
        assert_eq!(
            name(&Install::Portable(p), "windows", "aarch64").as_deref(),
            Some("studiocraft-1.2.3-windows-arm64-portable.zip")
        );
        assert_eq!(
            name(&Install::Msi, "windows", "x86_64").as_deref(),
            Some("studiocraft-1.2.3-windows-x64.msi")
        );
        assert_eq!(name(&Install::Managed, "macos", "aarch64"), None);
    }

    fn release_json(tag: &str, prerelease: bool) -> String {
        format!(
            r#"{{"tag_name":"{tag}","draft":false,"prerelease":{prerelease},
            "html_url":"https://github.com/ONEFOLD-LABS/StudioCraft/releases/tag/{tag}",
            "assets":[
              {{"name":"studiocraft-9.9.9-linux-x86_64.tar.gz","browser_download_url":"https://x/a.tgz"}},
              {{"name":"studiocraft-9.9.9-linux-x86_64.tar.gz.minisig","browser_download_url":"https://x/a.sig"}},
              {{"name":"studiocraft-9.9.9-windows-x64.msi","browser_download_url":"https://x/b.msi"}}
            ]}}"#
        )
    }

    #[test]
    fn releases_are_read_and_filtered() {
        let now = v("0.9.0");
        let r = parse_release(release_json("v9.9.9", false).as_bytes(), &now)
            .unwrap()
            .unwrap();
        assert_eq!(r.version, v("9.9.9"));
        assert!(r.page.contains("/tag/v9.9.9"));
        let (file, sig) =
            assets_for(&r, &Install::Tarball("/x".into()), "linux", "x86_64").unwrap();
        assert_eq!(file.url, "https://x/a.tgz");
        assert_eq!(sig.url, "https://x/a.sig");
        assert_eq!(
            assets_for(&r, &Install::Msi, "windows", "x86_64"),
            None,
            "no MSI signature, no install"
        );
        assert_eq!(assets_for(&r, &Install::Managed, "linux", "x86_64"), None);
        assert_eq!(
            parse_release(release_json("v9.9.9", true).as_bytes(), &now).unwrap(),
            None
        );
        assert_eq!(
            parse_release(release_json("v0.9.0", false).as_bytes(), &now).unwrap(),
            None
        );
        assert!(parse_release(b"not json", &now).is_err());
        assert!(parse_release(br#"{"tag_name":"latest"}"#, &now).is_err());
        // A page outside GitHub is not shown.
        let odd = release_json("v9.9.9", false).replace("https://github.com/", "https://evil/");
        let r = parse_release(odd.as_bytes(), &now).unwrap().unwrap();
        assert_eq!(r.page, RELEASE_PAGE);
    }

    #[test]
    fn signatures_must_match_key_file_and_version() {
        let key = key_from(&std::fs::read_to_string(format!("{TESTDATA}/test-key.pub")).unwrap())
            .unwrap();
        let data = std::fs::read(format!("{TESTDATA}/payload.tar.gz")).unwrap();
        let sig = std::fs::read_to_string(format!("{TESTDATA}/payload.tar.gz.minisig")).unwrap();
        verify(&key, &data, &sig, &v("9.9.9"), "payload.tar.gz").unwrap();
        // Another version or file name in the trusted comment: refused.
        assert!(verify(&key, &data, &sig, &v("9.9.8"), "payload.tar.gz").is_err());
        assert!(verify(&key, &data, &sig, &v("9.9.9"), "other.tar.gz").is_err());
        // Changed data: refused.
        let mut bad = data.clone();
        bad[10] ^= 1;
        assert!(verify(&key, &bad, &sig, &v("9.9.9"), "payload.tar.gz").is_err());
        assert!(verify(&key, &data, "garbage", &v("9.9.9"), "payload.tar.gz").is_err());
        assert_eq!(key_from("untrusted comment: none\n# no key yet\n"), None);
        // The programs inside the signed tarball.
        let programs = programs_from_tarball(&data).unwrap();
        assert_eq!(
            programs,
            vec![
                ("studiocraft".to_owned(), b"#!/bin/sh\necho new\n".to_vec()),
                (
                    "studiocraft-cli".to_owned(),
                    b"#!/bin/sh\necho cli\n".to_vec()
                ),
            ]
        );
    }

    #[test]
    fn the_shipped_key_file_has_no_key_yet_or_a_valid_one() {
        // Either empty (only notices) or a key minisign-verify reads.
        let lines = PUBLIC_KEY
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();
        assert!(lines == 0 || public_key().is_some());
    }

    #[test]
    fn archives_without_the_program_are_refused() {
        assert!(programs_from_tarball(b"not a tarball").is_err());
        assert!(programs_from_zip(b"not a zip").is_err());
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        zip.start_file("studiocraft-1-windows-x64-portable/README.md", opts)
            .unwrap();
        zip.write_all(b"x").unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(programs_from_zip(&bytes).is_err());
    }

    #[test]
    fn the_portable_zip_yields_both_programs() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in [
            (
                "studiocraft-1-windows-x64-portable/studiocraft.exe",
                b"app".as_slice(),
            ),
            (
                "studiocraft-1-windows-x64-portable/studiocraft-cli.exe",
                b"cli",
            ),
            ("studiocraft-1-windows-x64-portable/README.md", b"readme"),
            ("../studiocraft.exe", b"evil"),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(body).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let programs = programs_from_zip(&bytes).unwrap();
        assert_eq!(
            programs,
            vec![
                ("studiocraft.exe".to_owned(), b"app".to_vec()),
                ("studiocraft-cli.exe".to_owned(), b"cli".to_vec()),
            ]
        );
    }

    #[test]
    fn replacing_keeps_the_old_file_until_the_next_start() {
        let tmp = temp("replace");
        let target = tmp.join("studiocraft");
        std::fs::write(&target, b"old").unwrap();
        replace(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(tmp.join("studiocraft.old")).unwrap(), b"old");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&target).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "executable");
        }
        // The command-line program is replaced only where it already lies.
        replace_programs(
            &target,
            &[
                ("studiocraft".into(), b"newer".to_vec()),
                ("studiocraft-cli".into(), b"cli".to_vec()),
            ],
        )
        .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"newer");
        assert!(!tmp.join("studiocraft-cli").exists());
        std::fs::write(tmp.join("studiocraft-cli"), b"old cli").unwrap();
        replace_programs(&target, &[("studiocraft-cli".into(), b"cli".to_vec())]).unwrap();
        assert_eq!(std::fs::read(tmp.join("studiocraft-cli")).unwrap(), b"cli");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn the_check_is_daily() {
        assert!(due(0, 24 * 3600));
        assert!(!due(1000, 1000 + 3600));
        // A clock set back does not check on every start.
        assert!(!due(10_000, 5_000));
    }

    /// Die Suche gegen einen lokalen Server statt GitHub; ohne Verbindung ein Fehler, keine Panik.
    #[test]
    fn check_online_and_offline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body = release_json("v99.0.0", false);
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .build()
            .into();
        let found = check(&agent, &format!("http://{addr}/latest"))
            .unwrap()
            .unwrap();
        assert_eq!(found.version, v("99.0.0"));
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let gone = closed.local_addr().unwrap();
        drop(closed);
        assert!(check(&agent, &format!("http://{gone}/latest")).is_err());
    }
}
