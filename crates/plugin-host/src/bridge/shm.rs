//! The memory both processes share: one block of audio in each direction, the events of the
//! block and a few counters. It is a file in the runtime folder mapped by both processes
//! (`MAP_SHARED`; a file mapping on Windows), removed again once the helper has mapped it.
//!
//! Who may touch what (the protocol that makes the raw access sound): the app writes `input`,
//! `events`, `frames` and `count`, then publishes the block with `req` (release). The helper
//! sees `req` (acquire), reads those fields, writes `output` and publishes with `done`
//! (release). The app reads `output` only after seeing `done == req` (acquire), and writes the
//! next block only then. The helper is treated as hostile: everything read back is bounded and
//! sanitised by the reader.

use std::{
    fs::{File, OpenOptions},
    path::Path,
    ptr,
    sync::atomic::{AtomicU32, Ordering},
};

/// Largest block the bridge carries (the host's `MAX_BLOCK`).
pub(crate) const MAX_FRAMES: usize = 8192;
/// Most events per block.
pub(crate) const MAX_EVENTS: usize = 1024;
/// Marks a region written by this version.
const MAGIC: u32 = 0x5343_4252; // "SCBR"
const VERSION: u32 = 1;

/// One timed event of a block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Event {
    pub kind: u32,
    pub time: u32,
    pub a: u32,
    pub b: u32,
    pub value: f64,
}

pub(crate) const EVENT_PARAM: u32 = 1;
pub(crate) const EVENT_MIDI: u32 = 2;
pub(crate) const EVENT_NOTE: u32 = 3;

#[repr(C)]
pub(crate) struct Region {
    magic: AtomicU32,
    version: AtomicU32,
    /// Number of the last block the app handed over.
    pub req: AtomicU32,
    /// Number of the last block the helper finished.
    pub done: AtomicU32,
    /// Frames of the block `req`.
    pub frames: AtomicU32,
    /// Events of the block `req`.
    pub count: AtomicU32,
    /// The plugin's latency, written by the helper.
    pub latency: AtomicU32,
    /// The helper is to call `stop_processing` before the next block.
    pub stop: AtomicU32,
    events: [Event; MAX_EVENTS],
    input: [[f32; MAX_FRAMES]; 2],
    output: [[f32; MAX_FRAMES]; 2],
}

/// The mapping in this process.
pub(crate) struct Shared {
    ptr: *mut Region,
    _file: File,
}

// SAFETY: the mapping lives as long as `Shared`; the counters are atomics and the arrays are
// accessed only in the turns the protocol above allows, so moving the owner between threads is
// sound.
unsafe impl Send for Shared {}
// SAFETY: as above; concurrent access from the app's main and audio threads touches only the
// atomics (the arrays belong to the audio thread).
unsafe impl Sync for Shared {}

impl Shared {
    /// A new region in a new file at `path` (must not exist).
    pub fn create(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        file.set_len(size() as u64)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let s = Self::map(file)?;
        s.region().magic.store(MAGIC, Ordering::Relaxed);
        s.region().version.store(VERSION, Ordering::Release);
        Ok(s)
    }

    /// The region the app created at `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let len = file.metadata().map_err(|e| e.to_string())?.len();
        if len != size() as u64 {
            return Err("shared memory has the wrong size".into());
        }
        let s = Self::map(file)?;
        let r = s.region();
        if r.magic.load(Ordering::Acquire) != MAGIC || r.version.load(Ordering::Acquire) != VERSION
        {
            return Err("shared memory of another version".into());
        }
        Ok(s)
    }

    #[cfg(unix)]
    fn map(file: File) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        // SAFETY: a fresh shared mapping of the whole file, which has exactly `size()` bytes;
        // the result is checked before use.
        let p = unsafe {
            libc::mmap(
                ptr::null_mut(),
                size(),
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if p == libc::MAP_FAILED || p.is_null() {
            return Err(format!(
                "shared memory: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self {
            ptr: p.cast(),
            _file: file,
        })
    }

    #[cfg(windows)]
    fn map(file: File) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::CloseHandle,
            System::Memory::{
                CreateFileMappingW, FILE_MAP_ALL_ACCESS, MapViewOfFile, PAGE_READWRITE,
            },
        };
        let err = || format!("shared memory: {}", std::io::Error::last_os_error());
        // SAFETY: the handle of the open file; a mapping of its full size (0, 0), unnamed and
        // with default security. The result is checked.
        let mapping = unsafe {
            CreateFileMappingW(
                file.as_raw_handle(),
                ptr::null(),
                PAGE_READWRITE,
                0,
                0,
                ptr::null(),
            )
        };
        if mapping.is_null() {
            return Err(err());
        }
        // SAFETY: a view of the whole mapping (the file has exactly `size()` bytes).
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, size()) };
        // SAFETY: the mapping handle is no longer needed; the view keeps the mapping alive.
        unsafe { CloseHandle(mapping) };
        if view.Value.is_null() {
            return Err(err());
        }
        Ok(Self {
            ptr: view.Value.cast(),
            _file: file,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn map(_file: File) -> Result<Self, String> {
        Err("the plugin bridge is not available on this system".into())
    }

    pub fn region(&self) -> &Region {
        // SAFETY: `ptr` points to a live mapping of `size()` bytes, page aligned, for as long
        // as `self`; `Region` consists of atomics and plain numbers valid for any bit pattern.
        unsafe { &*self.ptr }
    }

    /// Copy `left`/`right` (up to [`MAX_FRAMES`]) into the input of the next block.
    /// App side, only while the helper is idle (`done == req`).
    pub fn write_input(&self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        // SAFETY: the protocol gives the arrays to the app while `done == req`; `n` is within
        // the arrays and the source slices.
        unsafe {
            let input = ptr::addr_of_mut!((*self.ptr).input);
            ptr::copy_nonoverlapping(left.as_ptr(), (*input)[0].as_mut_ptr(), n);
            ptr::copy_nonoverlapping(right.as_ptr(), (*input)[1].as_mut_ptr(), n);
        }
    }

    /// Copy the output of the last finished block into `left`/`right`, sanitised: values that
    /// are not finite become silence, the rest is limited to ±64 (+36 dBFS). App side, only
    /// while the helper is idle.
    pub fn read_output(&self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        // SAFETY: as in `write_input`; read element by element (volatile: the other process
        // wrote them) so nothing is assumed about their values.
        unsafe {
            let output = ptr::addr_of!((*self.ptr).output);
            for (ch, dst) in [&mut *left, &mut *right].into_iter().enumerate() {
                let src = (*output)[ch].as_ptr();
                for (i, d) in dst.iter_mut().take(n).enumerate() {
                    let v = ptr::read_volatile(src.add(i));
                    *d = if v.is_finite() {
                        v.clamp(-64.0, 64.0)
                    } else {
                        0.0
                    };
                }
            }
        }
    }

    /// Store the events of the next block (at most [`MAX_EVENTS`]). App side, helper idle.
    pub fn write_events(&self, events: &[Event]) {
        let n = events.len().min(MAX_EVENTS);
        // SAFETY: as in `write_input`.
        unsafe {
            let dst = ptr::addr_of_mut!((*self.ptr).events);
            ptr::copy_nonoverlapping(events.as_ptr(), (*dst).as_mut_ptr(), n);
        }
        self.region().count.store(n as u32, Ordering::Relaxed);
    }

    /// Helper side: the block `req` (after seeing it with acquire): its frames (bounded), the
    /// input copied into `left`/`right` and the events passed to `each`.
    pub fn read_block(
        &self,
        left: &mut [f32],
        right: &mut [f32],
        mut each: impl FnMut(Event),
    ) -> usize {
        let r = self.region();
        let n = (r.frames.load(Ordering::Relaxed) as usize)
            .min(MAX_FRAMES)
            .min(left.len())
            .min(right.len());
        let count = (r.count.load(Ordering::Relaxed) as usize).min(MAX_EVENTS);
        // SAFETY: the protocol gives the arrays to the helper between `req` and `done`; all
        // indices are bounded above.
        unsafe {
            let input = ptr::addr_of!((*self.ptr).input);
            ptr::copy_nonoverlapping((*input)[0].as_ptr(), left.as_mut_ptr(), n);
            ptr::copy_nonoverlapping((*input)[1].as_ptr(), right.as_mut_ptr(), n);
            let events = ptr::addr_of!((*self.ptr).events);
            for i in 0..count {
                each(ptr::read_volatile((*events).as_ptr().add(i)));
            }
        }
        n
    }

    /// Helper side: write the processed block.
    pub fn write_output(&self, left: &[f32], right: &[f32]) {
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        // SAFETY: as in `read_block`.
        unsafe {
            let output = ptr::addr_of_mut!((*self.ptr).output);
            ptr::copy_nonoverlapping(left.as_ptr(), (*output)[0].as_mut_ptr(), n);
            ptr::copy_nonoverlapping(right.as_ptr(), (*output)[1].as_mut_ptr(), n);
        }
    }
}

impl Drop for Shared {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: the mapping made in `map`, unmapped exactly once.
        unsafe {
            libc::munmap(self.ptr.cast(), size());
        }
        #[cfg(windows)]
        // SAFETY: the view made in `map`, unmapped exactly once.
        unsafe {
            windows_sys::Win32::System::Memory::UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.ptr.cast(),
                },
            );
        }
    }
}

fn size() -> usize {
    std::mem::size_of::<Region>()
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;

    #[test]
    fn two_mappings_see_the_same_block() {
        let path =
            std::env::temp_dir().join(format!("studiocraft-shm-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let app = Shared::create(&path).unwrap();
        let helper = Shared::open(&path).unwrap();
        // Windows may refuse while the file is mapped; it goes at the end then.
        let _ = std::fs::remove_file(&path);

        app.write_input(&[0.5; 4], &[-0.5; 4]);
        app.write_events(&[Event {
            kind: EVENT_PARAM,
            time: 2,
            a: 7,
            b: 0,
            value: 0.25,
        }]);
        app.region().frames.store(4, Ordering::Relaxed);
        app.region().req.store(1, Ordering::Release);

        let (mut l, mut r) = (vec![0.0; 8], vec![0.0; 8]);
        let mut seen = Vec::new();
        let n = helper.read_block(&mut l, &mut r, |e| seen.push(e));
        assert_eq!(n, 4);
        assert_eq!(&l[..4], &[0.5; 4]);
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].a, 7);
        helper.write_output(&[f32::NAN, 1000.0, 0.1, 0.2], &[0.3; 4]);
        helper.region().done.store(1, Ordering::Release);

        let (mut ol, mut or) = (vec![9.0; 4], vec![9.0; 4]);
        app.read_output(&mut ol, &mut or);
        // Hostile output: NaN becomes silence, huge values are limited.
        assert_eq!(ol, [0.0, 64.0, 0.1, 0.2]);
        assert_eq!(or, [0.3; 4]);
    }

    #[test]
    fn a_region_of_another_size_or_kind_is_refused() {
        let path = std::env::temp_dir().join(format!("studiocraft-shm-bad-{}", std::process::id()));
        std::fs::write(&path, b"short").unwrap();
        assert!(Shared::open(&path).is_err());
        let f = OpenOptions::new().write(true).open(&path).unwrap();
        f.set_len(size() as u64).unwrap();
        assert!(Shared::open(&path).is_err(), "zero magic");
        drop(f);
        std::fs::remove_file(&path).unwrap();
    }
}
