//! Confined file access for agents (craft-contrib standard `app-agents`, rule 18): every path an
//! agent passes must lie inside a read root (to read) or a write root (to write).
//!
//! A path is made absolute against the working directory and its links are followed. For a path
//! that does not exist yet, the nearest existing folder is resolved and the rest must be plain
//! names, so `..` below a missing folder and dangling links are refused. Roots are compared by
//! whole path components: `/work2/x` is not inside `/work`.

use std::path::{Component, Path, PathBuf};

/// The folders an agent may read from and write to.
#[derive(Clone, Debug)]
pub struct Roots {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
}

/// Kind of access a path is checked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

impl Roots {
    /// Roots from the folders given on the command line. Each must be an existing folder; at
    /// least one of each kind is required, so an agent entry point never starts unconfined.
    pub fn new(read: &[PathBuf], write: &[PathBuf]) -> Result<Self, String> {
        if read.is_empty() || write.is_empty() {
            return Err(
                "the MCP server needs --automation-read-root and --automation-write-root".into(),
            );
        }
        let resolve = |dirs: &[PathBuf]| -> Result<Vec<PathBuf>, String> {
            let mut out: Vec<PathBuf> = Vec::new();
            for d in dirs {
                let c = std::fs::canonicalize(d)
                    .map_err(|e| format!("automation root {}: {e}", d.display()))?;
                if !c.is_dir() {
                    return Err(format!("automation root is not a folder: {}", d.display()));
                }
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            Ok(out)
        };
        Ok(Self {
            read: resolve(read)?,
            write: resolve(write)?,
        })
    }

    /// `path` resolved, if it may be read.
    pub fn readable(&self, path: &Path) -> Result<PathBuf, String> {
        self.check(path, Access::Read)
    }

    /// `path` resolved, if it may be written (created, replaced).
    pub fn writable(&self, path: &Path) -> Result<PathBuf, String> {
        self.check(path, Access::Write)
    }

    fn check(&self, path: &Path, access: Access) -> Result<PathBuf, String> {
        let resolved = resolve(path)?;
        let (roots, what) = match access {
            Access::Read => (&self.read, "read"),
            Access::Write => (&self.write, "write"),
        };
        if roots.iter().any(|r| inside(&resolved, r)) {
            return Ok(resolved);
        }
        let list: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
        let plural = if list.len() == 1 { "" } else { "s" };
        Err(format!(
            "automation path rejected: outside the {what} root{plural} {}: {}",
            list.join(", "),
            path.display()
        ))
    }
}

/// Whether `path` is `root` or lies below it (whole components; case-insensitive on Windows).
fn inside(path: &Path, root: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| p.to_string_lossy().to_lowercase();
        let (p, r) = (lower(path), lower(root));
        Path::new(&p).starts_with(Path::new(&r))
    } else {
        path.starts_with(root)
    }
}

/// `path` absolute, its links followed; for a missing path the nearest existing ancestor is
/// resolved and the remaining components must be plain names.
fn resolve(path: &Path) -> Result<PathBuf, String> {
    let refuse = |why: &str| format!("automation path rejected: {why}: {}", path.display());
    if path.as_os_str().is_empty() {
        return Err(refuse("empty path"));
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| refuse(&e.to_string()))?
            .join(path)
    };
    if let Ok(c) = std::fs::canonicalize(&abs) {
        return Ok(c);
    }
    // A link that leads nowhere: writing through it would create its target elsewhere.
    if std::fs::symlink_metadata(&abs).is_ok() {
        return Err(refuse("link leads nowhere"));
    }
    let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
    let mut base = abs.as_path();
    loop {
        let Some(parent) = base.parent() else {
            return Err(refuse("no existing folder"));
        };
        match base.components().next_back() {
            Some(Component::Normal(name)) => rest.push(name),
            _ => return Err(refuse("`..` or `.` below a folder that does not exist")),
        }
        base = parent;
        if std::fs::symlink_metadata(base).is_ok() {
            break;
        }
    }
    let mut out = std::fs::canonicalize(base).map_err(|e| refuse(&e.to_string()))?;
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("studiocraft-roots-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn refuses_to_start_without_both_kinds_of_root() {
        let d = temp("both");
        assert!(Roots::new(&[], &[]).is_err());
        assert!(Roots::new(std::slice::from_ref(&d), &[]).is_err());
        assert!(Roots::new(&[], std::slice::from_ref(&d)).is_err());
        assert!(Roots::new(std::slice::from_ref(&d), std::slice::from_ref(&d)).is_ok());
        assert!(Roots::new(&[d.join("missing")], std::slice::from_ref(&d)).is_err());
    }

    #[test]
    fn paths_outside_a_root_are_refused() {
        let base = temp("outside");
        let read = base.join("in");
        let write = base.join("out");
        let sibling = base.join("out2");
        for d in [&read, &write, &sibling] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(read.join("a.wav"), b"x").unwrap();
        let roots = Roots::new(std::slice::from_ref(&read), std::slice::from_ref(&write)).unwrap();

        assert!(roots.readable(&read.join("a.wav")).is_ok());
        // Read and write are separate grants.
        assert!(roots.writable(&read.join("a.wav")).is_err());
        assert!(roots.writable(&write.join("new/deeper/b.wav")).is_ok());
        // Whole components: `out2` is not inside `out`.
        assert!(roots.writable(&sibling.join("b.wav")).is_err());
        // `..` cannot climb out, whether the folder exists or not.
        assert!(roots.writable(&write.join("../in/b.wav")).is_err());
        assert!(
            roots
                .writable(&write.join("missing/../../in/b.wav"))
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_out_of_a_root_are_refused() {
        let base = temp("links");
        let root = base.join("root");
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("escape")).unwrap();
        std::os::unix::fs::symlink(base.join("nowhere"), root.join("dangling")).unwrap();
        let roots = Roots::new(std::slice::from_ref(&root), std::slice::from_ref(&root)).unwrap();

        assert!(roots.writable(&root.join("escape/x.wav")).is_err());
        assert!(roots.writable(&root.join("dangling")).is_err());
        assert!(roots.writable(&root.join("fine.wav")).is_ok());
    }
}
