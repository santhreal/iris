use super::*;
use std::prelude::v1::test;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp(label: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(".internal/qa/recovery-tests")
        .join(format!(
            "{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

fn file(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create file parent");
    }
    std::fs::write(path, text).expect("write test file");
}

#[test]
fn a_failed_new_move_restores_the_previous_target() {
    let root = temp("restore-new-move");
    let new = root.join("new.exe");
    let target = root.join("iris.exe");
    let backup = root.join("iris.exe.old");
    file(&new, "new");
    file(&target, "old");
    inject_renames(&[false, true]);

    let error = replace_with_backup(&new, &target, &backup).expect_err("new move must fail");

    assert!(error.contains("replace"), "{error}");
    assert_eq!(std::fs::read_to_string(target).expect("old target"), "old");
    assert!(!backup.exists(), "restored backup must be consumed");
    assert_eq!(std::fs::read_to_string(new).expect("new file"), "new");
}

#[test]
fn a_failed_restore_keeps_and_names_the_only_backup() {
    let root = temp("restore-also-fails");
    let new = root.join("new.exe");
    let target = root.join("iris.exe");
    let backup = root.join("iris.exe.old");
    file(&new, "new");
    file(&target, "old");
    inject_renames(&[false, true, true]);

    let error = replace_with_backup(&new, &target, &backup).expect_err("restore must fail");

    assert!(error.contains("recovery file remains at"), "{error}");
    assert!(error.contains(&backup.display().to_string()), "{error}");
    assert!(
        error.contains("restore it before running iris --update again"),
        "{error}"
    );
    assert!(!target.exists(), "test never creates the missing target");
    assert_eq!(std::fs::read_to_string(backup).expect("backup"), "old");
}

#[test]
fn rollback_restores_other_files_and_keeps_an_unrestored_backup() {
    let root = temp("partial-rollback");
    let good_target = root.join("ffmpeg.exe");
    let good_backup = root.join("old/ffmpeg.exe");
    let bad_target = root.join("blocked");
    let bad_backup = root.join("old/blocked");
    file(&good_target, "new");
    file(&good_backup, "old ffmpeg");
    std::fs::create_dir_all(&bad_target).expect("target directory blocks removal");
    file(&bad_backup, "old blocked");

    let error = rollback(&[
        (good_target.clone(), Some(good_backup.clone())),
        (bad_target.clone(), Some(bad_backup.clone())),
    ])
    .expect_err("one restore must fail");

    assert!(error.contains("remove replaced"), "{error}");
    assert!(error.contains(&bad_backup.display().to_string()), "{error}");
    assert_eq!(
        std::fs::read_to_string(good_target).expect("restored"),
        "old ffmpeg"
    );
    assert!(!good_backup.exists());
    assert!(bad_target.is_dir());
    assert_eq!(
        std::fs::read_to_string(bad_backup).expect("kept backup"),
        "old blocked"
    );
}

#[test]
fn recovery_survives_finish_readiness_and_startup_cleanup() {
    let root = temp("preserve-recovery");
    let staging = root.join(".iris-update");
    let backup = staging.join("old/ffmpeg.exe");
    let aside = root.join("iris.exe.old");
    let target = root.join("iris.exe");
    file(&backup, "old ffmpeg");
    file(&aside, "old iris");
    file(&target, "current iris");

    let error = finish_swap(
        &staging,
        &aside,
        &target,
        Err("injected update failure".to_owned()),
    )
    .expect_err("update failure must be returned");
    assert_eq!(
        std::fs::read_to_string(&backup).expect("backup must survive failed update cleanup"),
        "old ffmpeg"
    );
    assert!(error.contains("injected update failure"), "{error}");
    assert!(error.contains("recovery files remain"), "{error}");
    assert!(
        error.contains("restore them before running iris --update again"),
        "{error}"
    );

    let error =
        refuse_recovery(&staging, &aside, &target).expect_err("readiness must refuse cleanup");
    assert!(
        error.contains("recovery files from a previous update remain"),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(&backup).expect("backup"),
        "old ffmpeg"
    );

    tidy(&staging, &aside, &target);
    assert_eq!(
        std::fs::read_to_string(&backup).expect("backup"),
        "old ffmpeg"
    );
    assert_eq!(std::fs::read_to_string(&aside).expect("aside"), "old iris");

    std::fs::remove_file(&backup).expect("simulate completed manual recovery");
    tidy(&staging, &aside, &target);
    assert!(!staging.exists(), "empty recovery directory is a leftover");
    assert!(!aside.exists(), "aside is no longer needed after recovery");
}

#[test]
fn restored_and_successful_updates_still_remove_staging() {
    let root = temp("ordinary-cleanup");
    let staging = root.join(".iris-update");
    let aside = root.join("iris.exe.old");
    let target = root.join("iris.exe");
    file(&target, "current iris");
    std::fs::create_dir_all(staging.join("old/licenses")).expect("empty backup tree");

    let error = finish_swap(&staging, &aside, &target, Err("restored".to_owned()))
        .expect_err("original update error must remain");
    assert_eq!(error, "restored");
    assert!(!staging.exists());

    file(&staging.join("payload"), "new");
    finish_swap(&staging, &aside, &target, Ok(())).expect("successful update");
    assert!(!staging.exists());
    assert!(refuse_recovery(&staging, &aside, &target).is_ok());
}

#[test]
fn an_unrestored_executable_backup_survives_until_it_is_restored() {
    let root = temp("executable-recovery");
    let staging = root.join(".iris-update");
    let aside = root.join("iris.exe.old");
    let target = root.join("iris.exe");
    std::fs::create_dir_all(staging.join("old")).expect("empty payload backup");
    file(&aside, "old iris");

    let error = finish_swap(
        &staging,
        &aside,
        &target,
        Err("exe restore failed".to_owned()),
    )
    .expect_err("update failure must be returned");
    assert!(error.contains("recovery files remain"), "{error}");
    assert!(error.contains(&aside.display().to_string()), "{error}");
    assert!(refuse_recovery(&staging, &aside, &target).is_err());
    tidy(&staging, &aside, &target);
    assert_eq!(
        std::fs::read_to_string(&aside).expect("old executable"),
        "old iris"
    );

    std::fs::rename(&aside, &target).expect("restore executable");
    tidy(&staging, &aside, &target);
    assert!(!staging.exists());
    assert_eq!(
        std::fs::read_to_string(&target).expect("restored executable"),
        "old iris"
    );

    // An aside left by a successful update is not recovery data while
    // its replacement target exists, so normal startup cleanup removes it.
    file(&aside, "previous iris");
    file(&staging.join("leftover"), "stale");
    tidy(&staging, &aside, &target);
    assert!(!aside.exists());
    assert!(!staging.exists());
}

#[cfg(unix)]
#[test]
fn a_special_recovery_entry_fails_closed() {
    let root = temp("special-recovery");
    let staging = root.join(".iris-update");
    let backup = staging.join("old");
    let aside = root.join("iris.exe.old");
    let target = root.join("iris.exe");
    std::fs::create_dir_all(&backup).expect("backup directory");
    file(&aside, "old iris");
    file(&target, "current iris");
    std::os::unix::fs::symlink(root.join("missing"), backup.join("link"))
        .expect("create test symlink");

    assert!(has_recovery(&staging, &aside, &target));
    assert!(refuse_recovery(&staging, &aside, &target).is_err());
    tidy(&staging, &aside, &target);
    assert!(backup.join("link").symlink_metadata().is_ok());
    assert!(aside.exists());
}
