//! Datei-Dialoge: nativ über rfd. Im Browser gibt es diese Dialoge nicht; dort steht ein
//! Platzhalter mit derselben Schnittstelle, der wie „Abbrechen“ wirkt. Öffnen und Speichern
//! über die Datei-APIs des Browsers ist ein eigener Schritt (Issue #30).

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use rfd::FileDialog;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::FileDialog;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::path::{Path, PathBuf};

    /// Platzhalter: jede Auswahl endet ohne Datei.
    #[derive(Default)]
    pub(crate) struct FileDialog;

    impl FileDialog {
        pub(crate) fn new() -> Self {
            Self
        }
        pub(crate) fn set_directory(self, _dir: impl AsRef<Path>) -> Self {
            self
        }
        pub(crate) fn add_filter(self, _name: impl Into<String>, _ext: &[impl ToString]) -> Self {
            self
        }
        pub(crate) fn set_file_name(self, _name: impl Into<String>) -> Self {
            self
        }
        pub(crate) fn save_file(self) -> Option<PathBuf> {
            None
        }
        pub(crate) fn pick_file(self) -> Option<PathBuf> {
            None
        }
        pub(crate) fn pick_files(self) -> Option<Vec<PathBuf>> {
            None
        }
    }
}
