//! Replacing the running exe. Windows lets a running exe be renamed but not overwritten, so:
//! rename `<exe>` to `<exe>.old`, copy the verified download to `<exe>`, and if that fails put
//! `<exe>.old` back. The file operations go through `Ops` so the order and the rollback are
//! tested with a fake (no real swap in tests). The start of the new process is the caller's.
use std::io;
use std::path::{Path, PathBuf};

use crate::update::{io_reason, old_path};

/// The few file operations a swap needs.
pub trait Ops {
    fn exists(&self, p: &Path) -> bool;
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove(&mut self, p: &Path) -> io::Result<()>;
}

/// The real file system.
pub struct Fs;

impl Ops for Fs {
    fn exists(&self, p: &Path) -> bool {
        p.exists()
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::copy(from, to).map(|_| ())
    }
    fn remove(&mut self, p: &Path) -> io::Result<()> {
        std::fs::remove_file(p)
    }
}

/// A swap that did not happen (or could not be undone); the text is safe to show.
#[derive(Debug, PartialEq, Eq)]
pub struct SwapError(pub String);

impl std::fmt::Display for SwapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SwapError {}

fn cannot(e: &io::Error) -> SwapError {
    SwapError(format!("Cannot update in this folder: {}", io_reason(e)))
}

/// Puts `new` in place of `exe`. Afterwards `exe` is the new program and `<exe>.old` the old
/// one (to be deleted by the next start). On any failure nothing is left changed.
pub fn swap_in(ops: &mut impl Ops, exe: &Path, new: &Path) -> Result<PathBuf, SwapError> {
    let old = old_path(exe);
    // A leftover from an earlier update that could not be deleted yet.
    if ops.exists(&old) {
        ops.remove(&old).map_err(|e| cannot(&e))?;
    }
    ops.rename(exe, &old).map_err(|e| cannot(&e))?;
    if let Err(e) = ops.copy(new, exe) {
        // A half-written copy must go before the old file can take its name back.
        let _ = ops.remove(exe);
        return Err(match ops.rename(&old, exe) {
            Ok(()) => cannot(&e),
            Err(_) => SwapError(format!(
                "Update failed ({}) and the old version could not be restored; \
                 it is still at {}",
                io_reason(&e),
                old.display()
            )),
        });
    }
    // The download is no longer needed.
    let _ = ops.remove(new);
    Ok(old)
}

/// Undoes a finished swap (the new program could not be started): the old file takes its name
/// back.
pub fn revert(ops: &mut impl Ops, exe: &Path) -> Result<(), SwapError> {
    let old = old_path(exe);
    let _ = ops.remove(exe);
    ops.rename(&old, exe).map_err(|e| cannot(&e))
}

/// Deletes the leftover `<exe>.old` (best effort; retried a few times in case the old process
/// is still closing).
pub fn remove_old(exe: &Path) {
    let old = old_path(exe);
    for _ in 0..5 {
        if !old.exists() || std::fs::remove_file(&old).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Records the calls and fails the ones it is told to.
    #[derive(Default)]
    struct Fake {
        files: HashSet<PathBuf>,
        log: Vec<String>,
        fail: Vec<&'static str>,
    }

    impl Fake {
        fn with(files: &[&str], fail: &[&'static str]) -> Self {
            Self {
                files: files.iter().map(PathBuf::from).collect(),
                log: vec![],
                fail: fail.to_vec(),
            }
        }
        fn step(&mut self, what: &str, op: &'static str) -> io::Result<()> {
            self.log.push(what.to_string());
            if self.fail.contains(&op) {
                // Only the first failure of that kind: a rollback of the same kind may work.
                self.fail.retain(|f| *f != op);
                return Err(io::Error::from(io::ErrorKind::PermissionDenied));
            }
            Ok(())
        }
    }

    impl Ops for Fake {
        fn exists(&self, p: &Path) -> bool {
            self.files.contains(p)
        }
        fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            self.step(&format!("rename {} {}", name(from), name(to)), "rename")?;
            self.files.remove(from);
            self.files.insert(to.to_path_buf());
            Ok(())
        }
        fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
            self.step(&format!("copy {} {}", name(from), name(to)), "copy")?;
            self.files.insert(to.to_path_buf());
            Ok(())
        }
        fn remove(&mut self, p: &Path) -> io::Result<()> {
            self.step(&format!("remove {}", name(p)), "remove")?;
            self.files.remove(p);
            Ok(())
        }
    }

    fn name(p: &Path) -> String {
        p.file_name().unwrap().to_string_lossy().into_owned()
    }

    const EXE: &str = r"C:\Apps\ai-subhub.exe";
    const NEW: &str = r"C:\Data\updates\ai-subhub-0.2.0.exe";
    const OLD: &str = r"C:\Apps\ai-subhub.exe.old";

    #[test]
    fn swaps_in_the_right_order() {
        let mut fs = Fake::with(&[EXE, NEW], &[]);
        let old = swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap();
        assert_eq!(old, PathBuf::from(OLD));
        assert_eq!(
            fs.log,
            [
                "rename ai-subhub.exe ai-subhub.exe.old",
                "copy ai-subhub-0.2.0.exe ai-subhub.exe",
                "remove ai-subhub-0.2.0.exe",
            ]
        );
        assert!(fs.files.contains(Path::new(EXE)) && fs.files.contains(Path::new(OLD)));
        assert!(!fs.files.contains(Path::new(NEW)));
    }

    #[test]
    fn a_leftover_old_file_is_removed_first() {
        let mut fs = Fake::with(&[EXE, NEW, OLD], &[]);
        swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap();
        assert_eq!(fs.log[0], "remove ai-subhub.exe.old");
        // ... and if it cannot be removed, nothing else is touched.
        let mut fs = Fake::with(&[EXE, NEW, OLD], &["remove"]);
        let e = swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap_err();
        assert!(
            e.0.starts_with("Cannot update in this folder: access is denied"),
            "{e}"
        );
        assert_eq!(fs.log.len(), 1);
    }

    #[test]
    fn an_unwritable_folder_changes_nothing() {
        let mut fs = Fake::with(&[EXE, NEW], &["rename"]);
        let e = swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap_err();
        assert_eq!(e.0, "Cannot update in this folder: access is denied");
        assert_eq!(fs.log.len(), 1, "stopped at the first rename");
        assert!(fs.files.contains(Path::new(EXE)) && fs.files.contains(Path::new(NEW)));
    }

    #[test]
    fn a_failed_copy_rolls_back_the_rename() {
        let mut fs = Fake::with(&[EXE, NEW], &["copy"]);
        let e = swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap_err();
        assert!(e.0.starts_with("Cannot update in this folder"), "{e}");
        assert_eq!(
            fs.log,
            [
                "rename ai-subhub.exe ai-subhub.exe.old",
                "copy ai-subhub-0.2.0.exe ai-subhub.exe",
                "remove ai-subhub.exe",
                "rename ai-subhub.exe.old ai-subhub.exe",
            ]
        );
        assert!(fs.files.contains(Path::new(EXE)), "the old exe is back");
        assert!(!fs.files.contains(Path::new(OLD)));
        assert!(
            fs.files.contains(Path::new(NEW)),
            "the download stays for a retry"
        );
    }

    #[test]
    fn a_failed_rollback_says_where_the_old_file_is() {
        // The first rename works, the copy fails, the rollback rename fails too.
        struct Stubborn(Fake);
        impl Ops for Stubborn {
            fn exists(&self, p: &Path) -> bool {
                self.0.exists(p)
            }
            fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
                if to == Path::new(EXE) {
                    return Err(io::Error::from(io::ErrorKind::PermissionDenied));
                }
                self.0.rename(from, to)
            }
            fn copy(&mut self, from: &Path, to: &Path) -> io::Result<()> {
                self.0.copy(from, to)
            }
            fn remove(&mut self, p: &Path) -> io::Result<()> {
                self.0.remove(p)
            }
        }
        let mut fs = Stubborn(Fake::with(&[EXE, NEW], &["copy"]));
        let e = swap_in(&mut fs, Path::new(EXE), Path::new(NEW)).unwrap_err();
        assert!(
            e.0.contains("could not be restored") && e.0.contains("ai-subhub.exe.old"),
            "{e}"
        );
    }

    #[test]
    fn revert_puts_the_old_exe_back() {
        let mut fs = Fake::with(&[EXE, OLD], &[]);
        revert(&mut fs, Path::new(EXE)).unwrap();
        assert_eq!(
            fs.log,
            [
                "remove ai-subhub.exe",
                "rename ai-subhub.exe.old ai-subhub.exe"
            ]
        );
        assert!(fs.files.contains(Path::new(EXE)) && !fs.files.contains(Path::new(OLD)));
    }
}
