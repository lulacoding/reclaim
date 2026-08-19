//! `reclaim-ui`'s Rust side (V1_DESIGN §3.2, ADR-001): launch-on-demand,
//! no daemon. Links `reclaim-core`/`reclaim-scan`/`reclaim-detect`/
//! `reclaim-action`/`reclaim-store` directly and exposes their
//! functionality as Tauri IPC commands -- no localhost server, no HTTP,
//! no second process. See `commands` module doc for the safety-boundary
//! rule this crate follows.

mod commands;
mod paths;
mod tier;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::scan::scan,
            commands::list::list,
            commands::clean::clean_dry_run,
            commands::clean::clean,
            commands::restore::restore,
            commands::restore::restore_all,
            commands::status::status,
            commands::settings::get_settings,
            commands::settings::save_settings,
            commands::settings::list_protected_paths,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
