mod commands;
mod state;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state::ViewerState::new())
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::close_file,
            commands::get_children,
            commands::get_value,
            commands::get_pointer,
            commands::search,
            commands::cancel_search,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri app");
}
