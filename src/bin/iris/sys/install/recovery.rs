//! Recovery primitives for an update that moves the previous payload
//! aside before installing its replacement. Backups are data until a
//! restore succeeds: cleanup and later updates preserve any that remain.

use std::path::{Path, PathBuf};

/// Move `new` to `target`, returning the previous target's backup.
pub(super) fn replace_with_backup(
    new: &Path,
    target: &Path,
    backup: &Path,
) -> Result<Option<PathBuf>, String> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("update: create {}: {e}", parent.display()))?;
    }
    if backup.exists() {
        std::fs::remove_file(backup)
            .map_err(|e| format!("update: remove old backup {}: {e}", backup.display()))?;
    }
    let old = if target.exists() {
        if let Some(parent) = backup.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("update: create {}: {e}", parent.display()))?;
        }
        rename(target, backup)
            .map_err(|e| format!("update: move {} aside: {e}", target.display()))?;
        Some(backup.to_path_buf())
    } else {
        None
    };
    if let Err(error) = rename(new, target) {
        let mut message = format!("update: replace {}: {error}", target.display());
        if let Some(old) = &old {
            if let Err(restore) = rename(old, target) {
                message = format!(
                    "{message}; restore {} from {}: {restore}; recovery file remains at {}; \
                     restore it before running iris --update again",
                    target.display(),
                    old.display(),
                    old.display(),
                );
            }
        }
        return Err(message);
    }
    Ok(old)
}

/// Restore files replaced before an update error. Every restore is
/// attempted; an unrestored backup stays at its path and is reported.
pub(super) fn rollback(replaced: &[(PathBuf, Option<PathBuf>)]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (target, old) in replaced.iter().rev() {
        let removed = match std::fs::remove_file(target) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
            Err(e) => {
                let mut message = format!(
                    "update: remove replaced {} during rollback: {e}",
                    target.display()
                );
                if let Some(old) = old {
                    message = format!("{message}; recovery file remains at {}", old.display());
                }
                errors.push(message);
                false
            }
        };
        if !removed {
            continue;
        }
        if let Some(old) = old {
            if let Err(e) = rename(old, target) {
                errors.push(format!(
                    "update: restore {} from {}: {e}; recovery file remains at {}",
                    target.display(),
                    old.display(),
                    old.display(),
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// List regular files below `dir`, relative to `root`. Symlinks and
/// other special entries are rejected instead of followed.
pub(super) fn files_below(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("update: read payload {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("update: read payload {}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("update: inspect payload {}: {e}", path.display()))?;
        if kind.is_dir() {
            files_below(root, &path, out)?;
        } else if kind.is_file() {
            out.push(
                path.strip_prefix(root)
                    .expect("payload child")
                    .to_path_buf(),
            );
        } else {
            return Err(format!("update: payload {} is not a file", path.display()));
        }
    }
    Ok(())
}

/// True while staging holds any previous-payload file, or the previous
/// executable remains aside without a restored target. An unreadable
/// backup path counts as recovery data: cleanup must fail closed.
pub(super) fn has_recovery(staging: &Path, aside: &Path, target: &Path) -> bool {
    payload_recovery(staging) || executable_recovery(aside, target)
}

fn payload_recovery(staging: &Path) -> bool {
    let backup = staging.join("old");
    match backup.try_exists() {
        Ok(false) => return false,
        Err(_) => return true,
        Ok(true) => {}
    }
    let mut files = Vec::new();
    files_below(&backup, &backup, &mut files)
        .map(|()| !files.is_empty())
        .unwrap_or(true)
}

fn executable_recovery(aside: &Path, target: &Path) -> bool {
    match aside.try_exists() {
        Ok(false) => false,
        Err(_) => true,
        Ok(true) => !matches!(target.try_exists(), Ok(true)),
    }
}

/// Prevent a new update or its readiness check from deleting recovery
/// files left by an earlier failed rollback.
pub(super) fn refuse_recovery(staging: &Path, aside: &Path, target: &Path) -> Result<(), String> {
    if has_recovery(staging, aside, target) {
        Err(format!(
            "update: recovery files from a previous update remain at {} or in {}; restore them \
             before running iris --update again",
            aside.display(),
            staging.join("old").display(),
        ))
    } else {
        Ok(())
    }
}

/// Finish a staged update. Successful updates and failures with every
/// old file restored remove staging. A failed rollback keeps it and
/// names the recovery directory in the returned error.
pub(super) fn finish_swap(
    staging: &Path,
    aside: &Path,
    target: &Path,
    swapped: Result<(), String>,
) -> Result<(), String> {
    match swapped {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(staging);
            Ok(())
        }
        Err(error) if has_recovery(staging, aside, target) => Err(format!(
            "{error}; recovery files remain at {} or in {}; restore them before running iris \
             --update again",
            aside.display(),
            staging.join("old").display(),
        )),
        Err(error) => {
            let _ = std::fs::remove_dir_all(staging);
            Err(error)
        }
    }
}

/// Delete update leftovers only when no previous-payload files remain.
/// The executable moved aside is retained too when recovery is pending.
pub(super) fn tidy(staging: &Path, aside: &Path, target: &Path) {
    if has_recovery(staging, aside, target) {
        return;
    }
    let _ = std::fs::remove_file(aside);
    let _ = std::fs::remove_dir_all(staging);
}

#[cfg(test)]
thread_local! {
    static RENAME_FAULTS: std::cell::RefCell<std::collections::VecDeque<bool>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// Queue rename outcomes for a deterministic recovery test. Each `true`
/// fails one rename on the current thread with `PermissionDenied`.
#[cfg(test)]
pub(super) fn inject_renames(failures: &[bool]) {
    RENAME_FAULTS.with(|faults| *faults.borrow_mut() = failures.iter().copied().collect());
}

fn rename(source: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if RENAME_FAULTS
        .with(|faults| faults.borrow_mut().pop_front())
        .unwrap_or(false)
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected rename failure",
        ));
    }
    std::fs::rename(source, target)
}

// WHY: the class closed here is "a failed update destroys its recovery
// data". Tests cover failed replacement, failed rollback, later cleanup,
// and a later update attempted before recovery. Not covered: a process or
// power failure between individual filesystem operations.
#[cfg(test)]
mod tests;
