mod backend;

use backend::{AutomationStatus, DesktopBackend, DesktopSnapshot};
use sorter_core::PlannedMove;
use sorter_runtime::RunReport;
use sorter_storage::{Config, HistoryEntry};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

struct AppState(Arc<DesktopBackend>);

#[tauri::command]
async fn load_snapshot(state: tauri::State<'_, AppState>) -> Result<DesktopSnapshot, String> {
    run_blocking(Arc::clone(&state.0), DesktopBackend::snapshot).await
}

#[tauri::command]
async fn save_configuration(
    state: tauri::State<'_, AppState>,
    config: Config,
) -> Result<Config, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.save_configuration(&config)).await
}

#[tauri::command]
async fn test_filename(config: Config, filename: String) -> Result<Option<PlannedMove>, String> {
    blocking(move || DesktopBackend::test_filename(&config, &filename)).await
}

#[tauri::command]
async fn preview_directory(
    state: tauri::State<'_, AppState>,
    directory_id: String,
) -> Result<RunReport, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.preview_directory(&directory_id)).await
}

#[tauri::command]
async fn execute_directory(
    state: tauri::State<'_, AppState>,
    directory_id: String,
) -> Result<RunReport, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.execute_directory(&directory_id)).await
}

#[tauri::command]
async fn load_history(
    state: tauri::State<'_, AppState>,
    limit: u32,
) -> Result<Vec<HistoryEntry>, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.history(limit)).await
}

#[tauri::command]
async fn start_automation(
    state: tauri::State<'_, AppState>,
    directory_id: String,
) -> Result<Vec<AutomationStatus>, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.start_automation(&directory_id)).await
}

#[tauri::command]
async fn stop_automation(
    state: tauri::State<'_, AppState>,
    directory_id: String,
) -> Result<Vec<AutomationStatus>, String> {
    let backend = Arc::clone(&state.0);
    blocking(move || backend.stop_automation(&directory_id)).await
}

#[tauri::command]
async fn automation_status(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AutomationStatus>, String> {
    run_blocking(Arc::clone(&state.0), DesktopBackend::automation_status).await
}

async fn run_blocking<T, F>(backend: Arc<DesktopBackend>, operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&DesktopBackend) -> Result<T, String> + Send + 'static,
{
    blocking(move || operation(&backend)).await
}

async fn blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("desktop worker failed: {error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Starts the native desktop application.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or run its event loop.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_path = configured_path("SORTER_CONFIG")
                .unwrap_or(app.path().app_config_dir()?.join("sorter.toml"));
            let database_path = configured_path("SORTER_DATABASE")
                .unwrap_or(app.path().app_data_dir()?.join("sorter.db"));
            let backend =
                DesktopBackend::new(config_path, database_path).map_err(std::io::Error::other)?;
            app.manage(AppState(Arc::new(backend)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            load_snapshot,
            save_configuration,
            test_filename,
            preview_directory,
            execute_directory,
            load_history,
            start_automation,
            stop_automation,
            automation_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running desktop application");
}

fn configured_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable).map(PathBuf::from)
}
