//! CLAP-Presets: Suche über die Preset-Fabrik der Plugin-Datei und Laden über die
//! `preset-load`-Erweiterung des Plugins. Alles im Hauptthread.
//!
//! Der Host stellt einen Indexer bereit; die Anbieter der Fabrik melden darüber, wo ihre Presets
//! liegen (Ordner, Dateien oder „im Plugin“) und welche Dateitypen dazugehören. Ordner werden
//! nach diesen Typen durchsucht, und für jede Fundstelle beschreibt der Anbieter seine Presets
//! über einen Empfänger. Behalten werden die Presets, die zum gewünschten Plugin gehören.

use std::{
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    ptr,
};

use clap_sys::{
    factory::preset_discovery::{
        CLAP_PRESET_DISCOVERY_FACTORY_ID, CLAP_PRESET_DISCOVERY_FACTORY_ID_COMPAT,
        CLAP_PRESET_DISCOVERY_LOCATION_FILE, CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
        clap_preset_discovery_factory, clap_preset_discovery_filetype,
        clap_preset_discovery_indexer, clap_preset_discovery_location,
        clap_preset_discovery_metadata_receiver, clap_preset_discovery_soundpack,
    },
    timestamp::clap_timestamp,
    universal_plugin_id::clap_universal_plugin_id,
    version::CLAP_VERSION,
};

use crate::{clap::Library, info::Preset};

/// Kennung, unter der CLAP-Presets in [`Preset::uri`] stehen (Art, Ort, Ladeschlüssel).
const URI_PREFIX: &str = "clap-preset:";
/// Trenner im URI (kommt in Pfaden und Schlüsseln nicht vor).
const SEP: char = '\u{1f}';
/// Grenzen gegen entgleiste Anbieter und riesige Ordner.
const MAX_FILES: usize = 20_000;
const MAX_DEPTH: u32 = 8;
const MAX_PRESETS: usize = 20_000;

/// Ein gefundenes Preset mit allem, was zum Laden nötig ist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Found {
    pub name: String,
    pub kind: u32,
    pub location: Option<String>,
    pub load_key: Option<String>,
    /// CLAP-Ids, für die das Preset gilt (leer: keine Angabe).
    pub plugins: Vec<String>,
}

impl Found {
    /// Als formatneutrales Preset (Ladeangaben im URI).
    pub(crate) fn preset(&self, label: String) -> Preset {
        Preset {
            uri: format!(
                "{URI_PREFIX}{}{SEP}{}{SEP}{}",
                self.kind,
                self.location.as_deref().unwrap_or(""),
                self.load_key.as_deref().unwrap_or("")
            ),
            name: label,
        }
    }
}

/// Ladeangaben aus einem URI von [`Found::preset`]: (Art, Ort, Schlüssel).
pub(crate) fn parse_uri(uri: &str) -> Option<(u32, Option<CString>, Option<CString>)> {
    let rest = uri.strip_prefix(URI_PREFIX)?;
    let mut parts = rest.split(SEP);
    let kind: u32 = parts.next()?.parse().ok()?;
    let opt = |s: &str| -> Option<Option<CString>> {
        if s.is_empty() {
            Some(None)
        } else {
            CString::new(s).ok().map(Some)
        }
    };
    let location = opt(parts.next()?)?;
    let key = opt(parts.next()?)?;
    parts.next().is_none().then_some((kind, location, key))
}

fn text(p: *const c_char) -> Option<String> {
    // SAFETY: Anbieter übergeben nullterminierte Zeichenketten oder null.
    (!p.is_null()).then(|| unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

/// Was der Indexer während `init` des Anbieters sammelt.
#[derive(Default)]
struct Declared {
    locations: Vec<(u32, Option<String>)>,
    extensions: Vec<String>,
}

unsafe extern "C" fn declare_filetype(
    indexer: *const clap_preset_discovery_indexer,
    filetype: *const clap_preset_discovery_filetype,
) -> bool {
    if indexer.is_null() || filetype.is_null() {
        return false;
    }
    // SAFETY: `indexer_data` ist unser `Declared` (lebt während init); Dateityp gültig.
    unsafe {
        let d = &mut *((*indexer).indexer_data as *mut Declared);
        if let Some(ext) = text((*filetype).file_extension).filter(|e| !e.is_empty()) {
            d.extensions
                .push(ext.trim_start_matches('.').to_lowercase());
        }
    }
    true
}

unsafe extern "C" fn declare_location(
    indexer: *const clap_preset_discovery_indexer,
    location: *const clap_preset_discovery_location,
) -> bool {
    if indexer.is_null() || location.is_null() {
        return false;
    }
    // SAFETY: wie oben.
    unsafe {
        let d = &mut *((*indexer).indexer_data as *mut Declared);
        d.locations
            .push(((*location).kind, text((*location).location)));
    }
    true
}

unsafe extern "C" fn declare_soundpack(
    _: *const clap_preset_discovery_indexer,
    _: *const clap_preset_discovery_soundpack,
) -> bool {
    true
}

unsafe extern "C" fn indexer_extension(
    _: *const clap_preset_discovery_indexer,
    _: *const c_char,
) -> *const c_void {
    ptr::null()
}

/// Was der Empfänger für eine Fundstelle sammelt.
struct Collected {
    kind: u32,
    location: Option<String>,
    presets: Vec<Found>,
}

/// # Safety
/// `r` muss unser Empfänger sein (Daten zeigen auf `Collected`).
unsafe fn collected<'a>(r: *const clap_preset_discovery_metadata_receiver) -> &'a mut Collected {
    // SAFETY: laut Vertrag; lebt während get_metadata.
    unsafe { &mut *((*r).receiver_data as *mut Collected) }
}

unsafe extern "C" fn on_error(
    _: *const clap_preset_discovery_metadata_receiver,
    _: i32,
    _: *const c_char,
) {
}

unsafe extern "C" fn begin_preset(
    r: *const clap_preset_discovery_metadata_receiver,
    name: *const c_char,
    load_key: *const c_char,
) -> bool {
    if r.is_null() {
        return false;
    }
    // SAFETY: unser Empfänger.
    let c = unsafe { collected(r) };
    if c.presets.len() >= MAX_PRESETS {
        return false;
    }
    let name = text(name).unwrap_or_default();
    c.presets.push(Found {
        name,
        kind: c.kind,
        location: c.location.clone(),
        load_key: text(load_key),
        plugins: Vec::new(),
    });
    true
}

unsafe extern "C" fn add_plugin_id(
    r: *const clap_preset_discovery_metadata_receiver,
    id: *const clap_universal_plugin_id,
) {
    if r.is_null() || id.is_null() {
        return;
    }
    // SAFETY: unser Empfänger; `id` laut Vertrag gültig.
    unsafe {
        let c = collected(r);
        if text((*id).abi).as_deref() == Some("clap")
            && let (Some(p), Some(last)) = (text((*id).id), c.presets.last_mut())
        {
            last.plugins.push(p);
        }
    }
}

unsafe extern "C" fn ignore_str(
    _: *const clap_preset_discovery_metadata_receiver,
    _: *const c_char,
) {
}
unsafe extern "C" fn ignore_flags(_: *const clap_preset_discovery_metadata_receiver, _: u32) {}
unsafe extern "C" fn ignore_times(
    _: *const clap_preset_discovery_metadata_receiver,
    _: clap_timestamp,
    _: clap_timestamp,
) {
}
unsafe extern "C" fn ignore_kv(
    _: *const clap_preset_discovery_metadata_receiver,
    _: *const c_char,
    _: *const c_char,
) {
}

/// Dateien unter `dir` mit einer der Endungen (keine Endungen: alle), begrenzt.
fn walk(dir: &Path, exts: &[String], out: &mut Vec<PathBuf>, depth: u32) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if out.len() >= MAX_FILES {
            return;
        }
        if p.is_dir() {
            walk(&p, exts, out, depth + 1);
        } else if exts.is_empty()
            || p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
        {
            out.push(p);
        }
    }
}

/// Alle Presets der Bibliothek, die zu `plugin_id` gehören (oder keine Zuordnung haben).
/// Hauptthread.
pub(crate) fn discover(lib: &Library, plugin_id: &str) -> Vec<Found> {
    let Some(factory) = lib.preset_factory() else {
        return Vec::new();
    };
    let mut all = Vec::new();
    // SAFETY: Fabrik lebt mit der Bibliothek; Indexer und Empfänger leben über alle Aufrufe;
    // jeder Anbieter wird nach Gebrauch zerstört.
    unsafe {
        let f = &*factory;
        let (Some(count), Some(get), Some(create)) = (f.count, f.get_descriptor, f.create) else {
            return Vec::new();
        };
        for i in 0..count(factory).min(64) {
            let desc = get(factory, i);
            if desc.is_null() || (*desc).id.is_null() {
                continue;
            }
            let mut declared = Declared::default();
            let indexer = clap_preset_discovery_indexer {
                clap_version: CLAP_VERSION,
                name: c"StudioCraft".as_ptr(),
                vendor: c"StudioCraft".as_ptr(),
                url: c"".as_ptr(),
                version: c"1".as_ptr(),
                indexer_data: (&mut declared as *mut Declared).cast(),
                declare_filetype: Some(declare_filetype),
                declare_location: Some(declare_location),
                declare_soundpack: Some(declare_soundpack),
                get_extension: Some(indexer_extension),
            };
            let provider = create(factory, &indexer, (*desc).id);
            if provider.is_null() {
                continue;
            }
            let p = &*provider;
            if p.init.is_some_and(|init| init(provider)) {
                for (kind, location) in declared.locations.clone() {
                    let targets: Vec<(u32, Option<String>, Option<PathBuf>)> =
                        if kind == CLAP_PRESET_DISCOVERY_LOCATION_FILE {
                            let Some(loc) = location.clone() else {
                                continue;
                            };
                            let path = PathBuf::from(&loc);
                            if path.is_dir() {
                                let mut files = Vec::new();
                                walk(&path, &declared.extensions, &mut files, 0);
                                files
                                    .into_iter()
                                    .map(|f| {
                                        (
                                            kind,
                                            Some(f.to_string_lossy().into_owned()),
                                            Some(path.clone()),
                                        )
                                    })
                                    .collect()
                            } else {
                                vec![(kind, Some(loc), None)]
                            }
                        } else if kind == CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN {
                            vec![(kind, None, None)]
                        } else {
                            Vec::new()
                        };
                    for (kind, loc, root) in targets {
                        let Some(get_metadata) = p.get_metadata else {
                            break;
                        };
                        let cloc = match loc.as_deref().map(CString::new) {
                            Some(Ok(c)) => Some(c),
                            Some(Err(_)) => continue,
                            None => None,
                        };
                        let mut c = Collected {
                            kind,
                            location: loc.clone(),
                            presets: Vec::new(),
                        };
                        let receiver = clap_preset_discovery_metadata_receiver {
                            receiver_data: (&mut c as *mut Collected).cast(),
                            on_error: Some(on_error),
                            begin_preset: Some(begin_preset),
                            add_plugin_id: Some(add_plugin_id),
                            set_soundpack_id: Some(ignore_str),
                            set_flags: Some(ignore_flags),
                            add_creator: Some(ignore_str),
                            set_description: Some(ignore_str),
                            set_timestamps: Some(ignore_times),
                            add_feature: Some(ignore_str),
                            add_extra_info: Some(ignore_kv),
                        };
                        let lp = cloc.as_ref().map_or(ptr::null(), |c| c.as_ptr());
                        get_metadata(provider, kind, lp, &receiver);
                        // Unterordner als Gruppe vor den Namen (bei Dateien in Ordnern).
                        if let (Some(root), Some(file)) = (&root, &loc) {
                            let rel = Path::new(file)
                                .parent()
                                .and_then(|d| d.strip_prefix(root).ok())
                                .map(|d| {
                                    d.to_string_lossy()
                                        .replace(std::path::MAIN_SEPARATOR, " / ")
                                })
                                .unwrap_or_default();
                            if !rel.is_empty() {
                                for f in &mut c.presets {
                                    f.name = format!("{rel} / {}", f.name);
                                }
                            }
                        }
                        all.extend(c.presets);
                        if all.len() >= MAX_PRESETS {
                            break;
                        }
                    }
                }
            }
            if let Some(destroy) = p.destroy {
                destroy(provider);
            }
        }
    }
    all.retain(|f| f.plugins.is_empty() || f.plugins.iter().any(|p| p == plugin_id));
    all.sort_by_key(|f| f.name.to_lowercase());
    all.dedup_by(|a, b| a.kind == b.kind && a.location == b.location && a.load_key == b.load_key);
    all
}

/// Fabrik-Kennungen (aktuell und Entwurf), in dieser Reihenfolge versucht.
pub(crate) const FACTORY_IDS: [&CStr; 2] = [
    CLAP_PRESET_DISCOVERY_FACTORY_ID,
    CLAP_PRESET_DISCOVERY_FACTORY_ID_COMPAT,
];

/// Zeigertyp der Fabrik (für `Library::preset_factory`).
pub(crate) type Factory = clap_preset_discovery_factory;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_roundtrip_and_reject_garbage() {
        let f = Found {
            name: "Bass".into(),
            kind: CLAP_PRESET_DISCOVERY_LOCATION_FILE,
            location: Some("/x/Bass.fxp".into()),
            load_key: None,
            plugins: Vec::new(),
        };
        let p = f.preset("Bass".into());
        let (kind, loc, key) = parse_uri(&p.uri).unwrap_or((9, None, None));
        assert_eq!(kind, CLAP_PRESET_DISCOVERY_LOCATION_FILE);
        assert_eq!(loc.as_deref(), Some(c"/x/Bass.fxp"));
        assert_eq!(key, None);
        let inside = Found {
            kind: CLAP_PRESET_DISCOVERY_LOCATION_PLUGIN,
            location: None,
            load_key: Some("7".into()),
            ..f
        };
        let (kind, loc, key) = parse_uri(&inside.preset("x".into()).uri).unwrap_or((9, None, None));
        assert_eq!((kind, loc, key.as_deref()), (1, None, Some(c"7")));
        assert_eq!(parse_uri("urn:lv2:preset"), None);
        assert_eq!(parse_uri("clap-preset:x\u{1f}\u{1f}"), None);
    }
}
