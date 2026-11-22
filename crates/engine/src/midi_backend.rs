//! MIDI-Anbindung je System. midir hat Backends für Linux (ALSA), Windows, macOS und iOS; auf
//! den übrigen Systemen (z. B. FreeBSD) gibt es hier gleichnamige Platzhalter, deren `new`
//! einen Fehler meldet. MIDI fehlt dort dann, alles andere läuft.

/// Was die Platzhalter melden; [`crate::midi`] macht daraus [`crate::EngineError::MidiUnsupported`].
pub(crate) const UNSUPPORTED: &str = "MIDI wird auf diesem System nicht unterstützt";

#[cfg(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios"
))]
pub use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios"
)))]
pub use unsupported::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

#[cfg(not(any(
    target_os = "linux",
    target_os = "windows",
    target_os = "macos",
    target_os = "ios"
)))]
mod unsupported {
    use std::convert::Infallible;
    use std::marker::PhantomData;

    const MESSAGE: &str = super::UNSUPPORTED;

    /// Port eines Platzhalters; es gibt nie einen.
    pub struct Port(Infallible);

    pub struct MidiInput(Infallible);
    pub struct MidiOutput(Infallible);
    pub struct MidiInputConnection<T>(Infallible, PhantomData<T>);
    pub struct MidiOutputConnection(Infallible);

    impl MidiInput {
        pub fn new(_name: &str) -> Result<Self, &'static str> {
            Err(MESSAGE)
        }
        pub fn ports(&self) -> Vec<Port> {
            match self.0 {}
        }
        pub fn port_name(&self, _port: &Port) -> Result<String, &'static str> {
            match self.0 {}
        }
        pub fn connect<F, T>(
            self,
            _port: &Port,
            _name: &str,
            _callback: F,
            _data: T,
        ) -> Result<MidiInputConnection<T>, &'static str>
        where
            F: FnMut(u64, &[u8], &mut T) + Send + 'static,
            T: Send,
        {
            match self.0 {}
        }
    }

    impl MidiOutput {
        pub fn new(_name: &str) -> Result<Self, &'static str> {
            Err(MESSAGE)
        }
        pub fn ports(&self) -> Vec<Port> {
            match self.0 {}
        }
        pub fn port_name(&self, _port: &Port) -> Result<String, &'static str> {
            match self.0 {}
        }
        pub fn connect(
            self,
            _port: &Port,
            _name: &str,
        ) -> Result<MidiOutputConnection, &'static str> {
            match self.0 {}
        }
    }

    impl MidiOutputConnection {
        pub fn send(&mut self, _data: &[u8]) -> Result<(), &'static str> {
            match self.0 {}
        }
    }
}
