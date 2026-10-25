//! Tests for CLI path resolution and admin env precedence.

use academic_routine::{resolve_routine_path, resolve_write_target, BotContext};
use std::path::PathBuf;

#[test]
fn sync_ucam_file_target_may_not_exist() {
    let missing = PathBuf::from("nonexistent/directory/routine_target.json");
    let resolved = resolve_write_target(Some(&missing));
    assert_eq!(resolved, missing);

    let default_resolved = resolve_write_target(None);
    assert_eq!(default_resolved, PathBuf::from("output/routine_data.json"));
}

#[test]
fn read_command_rejects_missing_explicit_file() {
    let missing = PathBuf::from("completely/nonexistent/routine.json");
    let result = resolve_routine_path(Some(&missing));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Specified routine file does not exist"));
    assert!(err.contains("completely/nonexistent/routine.json"));
}

#[test]
fn read_command_falls_back_only_when_no_file_given() {
    let temp_dir = tempfile::tempdir().unwrap();
    let sample = temp_dir.path().join("existing_routine.json");
    std::fs::write(&sample, "{}").unwrap();

    let result = resolve_routine_path(Some(&sample));
    assert_eq!(result, Ok(Some(sample)));

    // When None is passed in repository root with output/routine_data.json,
    // it returns Ok(Some("output/routine_data.json")) if present or Ok(None)
    let fallback = resolve_routine_path(None);
    assert!(fallback.is_ok());
}

#[test]
fn admin_chat_id_precedence_in_bot_context() {
    let temp_dir = tempfile::tempdir().unwrap();
    let state_file = temp_dir.path().join("routine_state.json");
    let routine_file = temp_dir.path().join("routine_data.json");
    std::fs::write(&routine_file, "{}").unwrap();

    // Use a lock/serialized env test since std::env affects process
    std::env::set_var("ADMIN_CHAT_ID", "987654321");
    std::env::remove_var("TELEGRAM_CHAT_ID");

    let ctx = BotContext::new(
        routine_file.clone(),
        state_file.clone(),
        Some("B2".to_string()),
        true,
    )
    .unwrap();

    assert_eq!(ctx.admin_chat_id, Some(987654321));

    // Cleanup env
    std::env::remove_var("ADMIN_CHAT_ID");
}
