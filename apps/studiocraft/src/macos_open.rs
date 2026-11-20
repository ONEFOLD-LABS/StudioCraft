//! macOS: Projekte und Audiodateien aus dem Finder öffnen (Issue #117).
//!
//! Der Finder übergibt Dokumente nicht als Startparameter, sondern als Ereignis an den
//! App-Delegate (`application:openURLs:`), beim Start wie im laufenden Betrieb. winit 0.30 meldet
//! dieses Ereignis nicht weiter; deshalb bekommt winits Delegate-Klasse hier zur Laufzeit genau
//! diese eine Methode dazu. Sie legt die Pfade in eine Warteschlange, die die App im nächsten
//! Frame abarbeitet ([`take`]). Einer der wenigen dokumentierten `unsafe`-Orte außerhalb von
//! `crates/plugin-host` (AGENTS.md).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use eframe::egui;
use objc2::MainThreadMarker;
use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::sel;
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSURL};

/// Vom Finder geöffnete, noch nicht verarbeitete Dateien.
static PENDING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Zum Aufwecken der Oberfläche, wenn eine Datei kommt.
static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

/// Merkt sich den egui-Kontext (einmal, aus dem ersten Frame).
pub fn set_context(ctx: &egui::Context) {
    let _ = CONTEXT.set(ctx.clone());
}

/// Die seit dem letzten Aufruf geöffneten Dateien.
pub fn take() -> Vec<PathBuf> {
    PENDING
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

/// Legt die Dateipfade aus `urls` in die Warteschlange.
fn queue(urls: &NSArray<NSURL>) {
    let paths: Vec<PathBuf> = urls
        .iter()
        .filter_map(|url| url.path())
        .map(|p| PathBuf::from(p.to_string()))
        .collect();
    if let Ok(mut pending) = PENDING.lock() {
        pending.extend(paths);
    }
    if let Some(ctx) = CONTEXT.get() {
        ctx.request_repaint();
    }
}

/// `-[delegate application:openURLs:]`.
unsafe extern "C-unwind" fn open_urls(
    _this: *mut AnyObject,
    _cmd: Sel,
    _app: *mut AnyObject,
    urls: *mut NSArray<NSURL>,
) {
    // SAFETY: AppKit ruft die Methode mit einem gültigen NSArray<NSURL> auf, das für die
    // Dauer des Aufrufs lebt; ein Nullzeiger wird übergangen.
    if let Some(urls) = unsafe { urls.as_ref() } {
        queue(urls);
    }
}

/// Hängt `application:openURLs:` an die Klasse des App-Delegates. Aufzurufen, nachdem winit
/// seine Ereignisschleife (samt Delegate) angelegt hat und bevor sie läuft. Ohne Delegate oder
/// außerhalb des Hauptthreads tut es nichts.
pub fn install() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate) = app.delegate() else {
        return;
    };
    let object: &AnyObject = (*delegate).as_ref();
    let class: &AnyClass = object.class();
    let open: unsafe extern "C-unwind" fn(
        *mut AnyObject,
        Sel,
        *mut AnyObject,
        *mut NSArray<NSURL>,
    ) = open_urls;
    // SAFETY: Ein Funktionszeiger lässt sich in den allgemeinen `Imp` umwandeln; die Laufzeit
    // ruft ihn mit genau der Signatur auf, die der Typ-String `v@:@@` beschreibt (void; self,
    // _cmd, NSApplication, NSArray), und die stimmt mit `open_urls` überein.
    let imp: Imp = unsafe { std::mem::transmute::<_, Imp>(open) };
    // SAFETY: `class` ist die von winit registrierte Delegate-Klasse. `class_addMethod` fügt die
    // Methode nur hinzu, wenn die Klasse sie selbst noch nicht hat (sonst gibt es NO zurück
    // und ändert nichts); der Typ-String ist eine statische C-Zeichenkette.
    unsafe {
        objc2::ffi::class_addMethod(
            (class as *const AnyClass).cast_mut(),
            sel!(application:openURLs:),
            imp,
            c"v@:@@".as_ptr(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_foundation::NSString;

    #[test]
    fn opened_urls_land_in_the_queue_as_paths() {
        let urls: Vec<_> = ["/tmp/Lied/Lied.studiocraft", "/tmp/Take 1.wav"]
            .into_iter()
            .map(|p| NSURL::fileURLWithPath(&NSString::from_str(p)))
            .collect();
        let array = NSArray::from_retained_slice(&urls);
        let ptr = (&*array as *const NSArray<NSURL>).cast_mut();
        // SAFETY: wie beim Aufruf durch AppKit: gültiges Array für die Dauer des Aufrufs.
        unsafe {
            open_urls(
                std::ptr::null_mut(),
                sel!(application:openURLs:),
                std::ptr::null_mut(),
                ptr,
            )
        };
        assert_eq!(
            take(),
            vec![
                PathBuf::from("/tmp/Lied/Lied.studiocraft"),
                PathBuf::from("/tmp/Take 1.wav")
            ]
        );
        assert!(take().is_empty());
    }
}
