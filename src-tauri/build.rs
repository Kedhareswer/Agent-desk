fn main() {
    // Declaring the app's commands turns each into a permission
    // ("allow-get-settings", …). The notch window is granted them in
    // capabilities/default.json; the glow window gets none.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_settings",
            "save_settings",
            "status",
            "ask",
            "stop",
            "take_recovery",
            "resume_task",
            "dismiss_recovery",
            "set_hit_regions",
            "new_chat",
            "hide",
            "resize",
            "focus",
            "copy_text",
            "insert_text",
            "setup_info",
            "list_models",
            "download_speech",
            "answer_action",
            "list_schedules",
            "save_schedule",
            "delete_schedule",
            "run_schedule_now",
        ]),
    ))
    .expect("failed to run tauri-build");
}
