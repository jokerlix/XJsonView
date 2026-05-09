mod commands;
mod state;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state::ViewerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::open_text,
            commands::close_file,
            commands::get_children,
            commands::get_value,
            commands::get_pointer,
            commands::child_for_segment,
            commands::search,
            commands::cancel_search,
            commands::export_subtree,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri app");
}
