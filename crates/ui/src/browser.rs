//! Dateien im Browser: Auswahl über das Datei-Feld des Browsers (rfd, asynchron) und
//! Downloads über einen Blob-Link. Die gewählten Dateien landen in einer Warteschlange, die
//! [`ProjectWindow::poll_browser_files`] in jedem Frame abarbeitet. Projekte werden als
//! zip-Bündel gespeichert und geöffnet (siehe `bundle.rs`).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;

use crate::ProjectWindow;
use crate::menu::Action;
use studiocraft_project::file::EXTENSION;

/// Wofür eine Datei gewählt wurde.
#[derive(Clone, Copy)]
enum Kind {
    Open,
    Import,
}

struct Picked {
    kind: Kind,
    name: String,
    bytes: Vec<u8>,
}

/// Gewählte, noch nicht verarbeitete Dateien.
#[derive(Default, Clone)]
pub(crate) struct Pending(Rc<RefCell<Vec<Picked>>>);

impl Pending {
    fn pick(&self, ctx: &egui::Context, kind: Kind, filter: &'static str, exts: Vec<String>) {
        let pending = self.clone();
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let dialog = rfd::AsyncFileDialog::new().add_filter(filter, &exts);
            let files = match kind {
                Kind::Open => dialog.pick_file().await.into_iter().collect(),
                Kind::Import => dialog.pick_files().await.unwrap_or_default(),
            };
            for f in files {
                let bytes = f.read().await;
                pending.0.borrow_mut().push(Picked {
                    kind,
                    name: f.file_name(),
                    bytes,
                });
            }
            ctx.request_repaint();
        });
    }
}

/// Bietet `bytes` als Download `name` an.
fn download(name: &str, bytes: &[u8], mime: &str) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("Download: {e:?}");
    let parts = js_sys::Array::new();
    parts.push(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "Download: kein Dokument".to_owned())?;
    let link = document
        .create_element("a")
        .map_err(js)?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "Download: kein Link".to_owned())?;
    link.set_href(&url);
    link.set_download(name);
    link.click();
    web_sys::Url::revoke_object_url(&url).map_err(js)
}

impl ProjectWindow {
    /// Datei-Befehle im Browser. `false`: nicht hier behandelt (der normale Weg gilt).
    pub(crate) fn run_in_browser(&mut self, ctx: &egui::Context, action: Action) -> bool {
        let result = match action {
            Action::Open => {
                let exts = vec![EXTENSION.to_owned(), "zip".to_owned()];
                self.browser.pick(
                    ctx,
                    Kind::Open,
                    crate::i18n::tr("StudioCraft-Projekt"),
                    exts,
                );
                Ok(())
            }
            Action::Save | Action::SaveAs | Action::SaveVersion => self
                .project_bundle()
                .and_then(|(name, bytes)| download(&name, &bytes, "application/zip")),
            Action::ImportAudio => {
                let exts = studiocraft_engine::AudioClip::extensions()
                    .iter()
                    .map(|e| (*e).to_owned())
                    .collect();
                self.browser
                    .pick(ctx, Kind::Import, crate::i18n::tr("Audiodateien"), exts);
                Ok(())
            }
            Action::ExportMixdown => self.mixdown_bytes("wav").and_then(|(name, bytes, peak)| {
                if peak > 1.0 {
                    self.message = Some(crate::i18n::trf(
                        "Export übersteuert (Spitze {db} dBFS) – Ausgang leiser stellen",
                        &[("db", &format!("{:+.1}", 20.0 * peak.log10()))],
                    ));
                }
                download(&name, &bytes, "audio/wav")
            }),
            // Ein Browser-Tab schließt der Nutzer selbst.
            Action::Quit => Ok(()),
            Action::New => return false,
        };
        if let Err(e) = result {
            self.message = Some(e);
        }
        true
    }

    /// Verarbeitet Dateien, die der Browser inzwischen geliefert hat.
    pub(crate) fn poll_browser_files(&mut self) {
        let picked: Vec<Picked> = std::mem::take(&mut *self.browser.0.borrow_mut());
        if picked.is_empty() {
            return;
        }
        let before = self.project.clone();
        let mut errors = Vec::new();
        let mut imported = false;
        for p in picked {
            let r = match p.kind {
                Kind::Open => self.open_bundle(&p.name, &p.bytes),
                Kind::Import => {
                    imported = true;
                    self.import_audio_bytes(&p.name, &p.bytes)
                }
            };
            if let Err(e) = r {
                errors.push(e);
            }
        }
        if imported {
            self.checkpoint(before);
        }
        if !errors.is_empty() {
            self.message = Some(errors.join("; "));
        }
    }
}
