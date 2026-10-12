mod cache;
mod commands;
mod config;
mod db;
mod extract;
mod llm;
mod mathfmt;
mod ocr;
mod pdf;
mod structure;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            db::init(app.handle()).map_err(|e| e.into())
        })
        .invoke_handler(tauri::generate_handler![
            commands::db_status,
            commands::app_info,
            commands::get_config,
            commands::save_config,
            commands::ocr_image,
            commands::grade_answer,
            commands::test_llm,
            commands::import_answer_files,
            commands::cache_stats,
            commands::clear_cache,
            commands::save_answers,
            commands::import_sheets,
            commands::test_providers
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
