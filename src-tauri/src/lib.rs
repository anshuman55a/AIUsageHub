mod providers;

use std::time::Duration;

use reqwest::Url;
use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_updater::UpdaterExt;

use providers::{list_providers, probe_provider, ProviderResult};

const BLUR_HIDE_DEBOUNCE_MS: u64 = 180;
const UPDATER_PUBLIC_KEY: Option<&str> = option_env!("TAURI_UPDATER_PUBLIC_KEY");
const UPDATER_ENDPOINTS: Option<&str> = option_env!("TAURI_UPDATER_ENDPOINT");

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
    published_at: Option<String>,
}

fn updater_enabled() -> bool {
    matches!(UPDATER_PUBLIC_KEY, Some(value) if !value.trim().is_empty())
        && matches!(UPDATER_ENDPOINTS, Some(value) if !value.trim().is_empty())
}

fn updater_urls() -> Result<Vec<Url>, String> {
    let raw = UPDATER_ENDPOINTS.ok_or_else(|| "Updater endpoint is not configured.".to_string())?;
    let urls = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            Url::parse(value)
                .map_err(|error| format!("Invalid updater endpoint '{value}': {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if urls.is_empty() {
        return Err("Updater endpoint is not configured.".into());
    }

    Ok(urls)
}

fn build_updater(app: &tauri::AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    if !updater_enabled() {
        return Err("Updater is not configured for this build.".into());
    }

    let pubkey = UPDATER_PUBLIC_KEY
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Updater public key is missing.".to_string())?;

    let updater_builder = app
        .updater_builder()
        .pubkey(pubkey)
        .timeout(Duration::from_secs(20))
        .endpoints(updater_urls()?)
        .map_err(|error| format!("Failed to configure updater endpoints: {error}"))?;

    updater_builder
        .build()
        .map_err(|error| format!("Failed to build updater client: {error}"))
}

#[tauri::command]
async fn probe(id: String) -> ProviderResult {
    let id_clone = id.clone();
    // Run blocking provider probe in a thread
    tokio::task::spawn_blocking(move || probe_provider(&id_clone))
        .await
        .unwrap_or_else(|e| ProviderResult {
            id: id.clone(),
            name: id.clone(),
            icon: String::new(),
            brand_color: "#666".into(),
            plan: None,
            lines: vec![],
            error: Some(format!("Internal error: {}", e)),
        })
}

#[tauri::command]
async fn probe_all() -> Vec<ProviderResult> {
    let providers = list_providers();
    let mut results = Vec::new();

    for meta in providers {
        let id = meta.id.clone();
        let result = tokio::task::spawn_blocking(move || probe_provider(&id))
            .await
            .unwrap_or_else(|e| ProviderResult {
                id: meta.id.clone(),
                name: meta.name.clone(),
                icon: meta.icon.clone(),
                brand_color: meta.brand_color.clone(),
                plan: None,
                lines: vec![],
                error: Some(format!("Internal error: {}", e)),
            });
        results.push(result);
    }

    results
}

#[tauri::command]
fn updater_enabled_command() -> bool {
    updater_enabled()
}

#[tauri::command]
fn get_ollama_settings() -> providers::ollama::OllamaSettings {
    providers::ollama::load_settings()
}

#[tauri::command]
fn save_ollama_settings(url: String, api_key: String) -> Result<(), String> {
    let clean_url = if url.trim().is_empty() {
        "http://localhost:11434".into()
    } else {
        let trimmed = url.trim().trim_end_matches('/').to_string();
        // Only allow http:// and https:// schemes to prevent API key exfiltration
        let lower = trimmed.to_lowercase();
        if !lower.starts_with("http://") && !lower.starts_with("https://") {
            return Err("URL must start with http:// or https://".into());
        }
        // Block URLs with credentials embedded (http://user:pass@host)
        if let Some(after_scheme) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")) {
            if after_scheme.contains('@') {
                return Err("URL must not contain embedded credentials".into());
            }
        }
        trimmed
    };
    let settings = providers::ollama::OllamaSettings {
        url: clean_url,
        api_key: api_key.trim().to_string(),
    };
    providers::ollama::save_settings(&settings)
}

#[tauri::command]
async fn check_for_app_update(app: tauri::AppHandle) -> Result<Option<AppUpdateInfo>, String> {
    let update = build_updater(&app)?
        .check()
        .await
        .map_err(|error| format!("Failed to check for updates: {error}"))?;

    Ok(update.map(|update| AppUpdateInfo {
        version: update.version,
        current_version: update.current_version,
        notes: update.body,
        published_at: update.date.map(|value| value.to_string()),
    }))
}

#[tauri::command]
async fn install_app_update(app: tauri::AppHandle) -> Result<(), String> {
    let update = build_updater(&app)?
        .check()
        .await
        .map_err(|error| format!("Failed to check for updates: {error}"))?
        .ok_or_else(|| "No update is currently available.".to_string())?;

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| format!("Failed to install update: {error}"))?;

    Ok(())
}

fn toggle_panel(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
        } else {
            show_panel(app, &window);
        }
    }
}

fn show_panel(app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    // Position near tray (bottom-right for Windows)
    position_window_near_tray(app, window);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

fn position_window_near_tray(_app: &tauri::AppHandle, window: &tauri::WebviewWindow) {
    // Get primary monitor info
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let monitor_size = monitor.size();
        let monitor_pos = monitor.position();
        let scale = monitor.scale_factor();

        let win_width = (380.0 * scale) as i32;
        let win_height = (520.0 * scale) as i32;
        let margin = (12.0 * scale) as i32;

        // Position at bottom-right, above taskbar
        // Windows taskbar is typically ~40px
        let taskbar_height = (48.0 * scale) as i32;

        let x = monitor_pos.x + monitor_size.width as i32 - win_width - margin;
        let y = monitor_pos.y + monitor_size.height as i32 - win_height - taskbar_height;

        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }));
    }
}

fn hide_window_if_still_blurred(window: tauri::WebviewWindow) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(BLUR_HIDE_DEBOUNCE_MS)).await;

        let is_visible = window.is_visible().unwrap_or(false);
        let is_focused = window.is_focused().unwrap_or(false);

        if is_visible && !is_focused {
            let _ = window.hide();
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                show_panel(app, &window);
            }
        }))
        .setup(|app| {
            // Build context menu
            let show_i = MenuItem::with_id(app, "show", "Show UsageDock", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

            // Build tray icon
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .tooltip("UsageDock - AI Usage Tracker")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            show_panel(app.app_handle(), &window);
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|app, event| match event {
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } => {
                        toggle_panel(app.app_handle());
                    }
                    _ => {}
                })
                .build(app)?;

            // Set up focus loss handler to auto-hide
            let _app_handle = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Focused(false) = event {
                        hide_window_if_still_blurred(w.clone());
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe,
            probe_all,
            updater_enabled_command,
            check_for_app_update,
            install_app_update,
            get_ollama_settings,
            save_ollama_settings
        ])
        .run(tauri::generate_context!())
        .expect("Error while running UsageDock");
}
