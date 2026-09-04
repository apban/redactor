mod watcher;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
#[cfg(desktop)]
use tauri_plugin_updater::UpdaterExt;

const WATCH_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_TOGGLE_SHORTCUT: &str = "CmdOrCtrl+Alt+B";

/// Timestamped stderr log for diagnosing state transitions.
pub(crate) fn log(msg: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() % 86400;
    eprintln!(
        "[redactor {:02}:{:02}:{:02}.{:03}] {msg}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60,
        now.subsec_millis()
    );
}

#[derive(Default)]
struct RedactorState {
    drawing: bool,
    box_count: u32,
    watcher_stop: Option<Arc<AtomicBool>>,
    watcher_deadline: Option<Arc<Mutex<Instant>>>,
    /// The currently registered toggle shortcut, as a Tauri accelerator string.
    toggle_shortcut: String,
}

type SharedState = Mutex<RedactorState>;

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|d| d.join("config.json"))
}

fn load_toggle_shortcut(app: &AppHandle) -> String {
    config_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("toggle").and_then(|t| t.as_str().map(String::from)))
        .unwrap_or_else(|| DEFAULT_TOGGLE_SHORTCUT.to_string())
}

fn save_toggle_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let path = config_path(app).ok_or("no config dir")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::json!({ "toggle": shortcut });
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).map_err(|e| e.to_string())
}

fn windows_with_prefix(app: &AppHandle, prefix: &str) -> Vec<WebviewWindow> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| label.starts_with(prefix))
        .map(|(_, w)| w)
        .collect()
}

fn stop_watcher(app: &AppHandle) {
    let state = app.state::<SharedState>();
    let mut s = state.lock().unwrap();
    if let Some(flag) = s.watcher_stop.take() {
        flag.store(true, Ordering::Relaxed);
    }
    s.watcher_deadline = None;
}

/// Exclude a window from screen captures so the draw-mode tint never bakes
/// into the screenshot. Supported on macOS (NSWindowSharingNone) and Windows
/// (WDA_EXCLUDEFROMCAPTURE). On Linux the tint is disabled instead (frontend
/// sniffs the platform).
#[allow(unused_variables)]
fn exclude_from_capture(window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;
        if let Ok(ns_window) = window.ns_window() {
            let ns_window = ns_window as *mut AnyObject;
            unsafe {
                let _: () = msg_send![&*ns_window, setSharingType: 0u64];
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowDisplayAffinity;
        const WDA_EXCLUDEFROMCAPTURE: u32 = 0x0000_0011;
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                SetWindowDisplayAffinity(hwnd.0 as _, WDA_EXCLUDEFROMCAPTURE);
            }
        }
    }
}

fn overlay_builder<'a>(
    app: &'a AppHandle,
    label: String,
    monitor: &tauri::Monitor,
) -> WebviewWindowBuilder<'a, tauri::Wry, AppHandle> {
    let scale = monitor.scale_factor();
    let pos = monitor.position();
    let size = monitor.size();
    WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .position(pos.x as f64 / scale, pos.y as f64 / scale)
        .inner_size(size.width as f64 / scale, size.height as f64 / scale)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
}

fn enter_draw_mode(app: &AppHandle) {
    log("enter_draw_mode");
    {
        let state = app.state::<SharedState>();
        let mut s = state.lock().unwrap();
        if s.drawing {
            return;
        }
        s.drawing = true;
    }

    let monitors = app.available_monitors().unwrap_or_default();
    let boxes_exist = !windows_with_prefix(app, "boxes-").is_empty();
    for (i, monitor) in monitors.iter().enumerate() {
        // Boxes layer first (below), capturable and always click-through.
        if !boxes_exist {
            match overlay_builder(app, format!("boxes-{i}"), monitor)
                .focused(false)
                .build()
            {
                Ok(w) => {
                    let _ = w.set_ignore_cursor_events(true);
                }
                Err(e) => log(&format!("failed to create boxes-{i}: {e}")),
            }
        }
        // Draw layer on top: interactive, excluded from captures.
        match overlay_builder(app, format!("draw-{i}"), monitor)
            .focused(true)
            .build()
        {
            Ok(w) => exclude_from_capture(&w),
            Err(e) => log(&format!("failed to create draw-{i}: {e}")),
        }
    }

    // The watcher runs from draw-mode entry so a capture taken mid-draw-mode
    // (no explicit commit) still clears everything.
    let state = app.state::<SharedState>();
    let mut s = state.lock().unwrap();
    if s.watcher_stop.is_none() {
        let (stop, deadline) = watcher::start(app.clone(), WATCH_TIMEOUT);
        s.watcher_stop = Some(stop);
        s.watcher_deadline = Some(deadline);
    }
}

/// Leave draw mode, keeping committed boxes on screen (they stay until a
/// capture completes, the timeout fires, or a clear is requested).
fn exit_draw_mode(app: &AppHandle) {
    let box_count;
    {
        let state = app.state::<SharedState>();
        let mut s = state.lock().unwrap();
        if !s.drawing {
            return;
        }
        s.drawing = false;
        box_count = s.box_count;
    }
    log(&format!("exit_draw_mode: box_count={box_count}"));
    for w in windows_with_prefix(app, "draw-") {
        let _ = w.close();
    }
    if box_count == 0 {
        clear_all(app);
    }
}

pub(crate) fn clear_all(app: &AppHandle) {
    log("clear_all");
    stop_watcher(app);
    {
        let state = app.state::<SharedState>();
        let mut s = state.lock().unwrap();
        s.drawing = false;
        s.box_count = 0;
    }
    for w in windows_with_prefix(app, "draw-") {
        let _ = w.close();
    }
    for w in windows_with_prefix(app, "boxes-") {
        let _ = w.close();
    }
}

fn toggle(app: &AppHandle) {
    let drawing = app.state::<SharedState>().lock().unwrap().drawing;
    if drawing {
        exit_draw_mode(app);
    } else {
        enter_draw_mode(app);
    }
}

fn add_box_internal(app: &AppHandle, index: &str, x: f64, y: f64, w: f64, h: f64) {
    {
        let state = app.state::<SharedState>();
        let mut s = state.lock().unwrap();
        s.box_count += 1;
        log(&format!("add_box: boxes-{index} count={}", s.box_count));
        // Drawing activity restarts the auto-clear countdown.
        if let Some(deadline) = &s.watcher_deadline {
            *deadline.lock().unwrap() = Instant::now() + WATCH_TIMEOUT;
        }
    }
    let _ = app.emit_to(
        format!("boxes-{index}"),
        "add-box",
        serde_json::json!({ "x": x, "y": y, "w": w, "h": h }),
    );
}

#[tauri::command]
fn add_box(window: WebviewWindow, app: AppHandle, x: f64, y: f64, w: f64, h: f64) {
    let index = window
        .label()
        .strip_prefix("draw-")
        .unwrap_or("0")
        .to_string();
    add_box_internal(&app, &index, x, y, w, h);
}

#[tauri::command]
fn overlay_exit(app: AppHandle) {
    exit_draw_mode(&app);
}

#[tauri::command]
fn overlay_cancel(app: AppHandle) {
    clear_all(&app);
}

/// Register the toggle shortcut currently held in state. Unregisters first so
/// this is idempotent (double-register is an error otherwise).
fn register_toggle(app: &AppHandle) {
    let cur = app
        .state::<SharedState>()
        .lock()
        .unwrap()
        .toggle_shortcut
        .clone();
    match cur.parse::<Shortcut>() {
        Ok(sc) => {
            let gs = app.global_shortcut();
            let _ = gs.unregister(sc.clone());
            if let Err(e) = gs.register(sc) {
                log(&format!("failed to register toggle shortcut: {e}"));
            }
        }
        Err(_) => log(&format!("invalid stored shortcut: {cur}")),
    }
}

#[tauri::command]
fn get_toggle_shortcut(app: AppHandle) -> String {
    app.state::<SharedState>()
        .lock()
        .unwrap()
        .toggle_shortcut
        .clone()
}

/// Suspend the global shortcut while the settings window captures keys, so the
/// combo reaches the focused webview instead of toggling redactor. State keeps
/// the string, so resume/close re-registers it.
#[tauri::command]
fn pause_shortcut(app: AppHandle) {
    let cur = app
        .state::<SharedState>()
        .lock()
        .unwrap()
        .toggle_shortcut
        .clone();
    if let Ok(sc) = cur.parse::<Shortcut>() {
        let _ = app.global_shortcut().unregister(sc);
    }
}

#[tauri::command]
fn resume_shortcut(app: AppHandle) {
    register_toggle(&app);
}

/// Validate, re-register, and persist a new toggle shortcut. Returns an error
/// string the settings window shows inline; on error the old shortcut stays.
#[tauri::command]
fn set_toggle_shortcut(app: AppHandle, shortcut: String) -> Result<(), String> {
    let new: Shortcut = shortcut
        .parse()
        .map_err(|_| format!("invalid shortcut: {shortcut}"))?;
    let gs = app.global_shortcut();
    let old = app
        .state::<SharedState>()
        .lock()
        .unwrap()
        .toggle_shortcut
        .clone();
    if let Ok(old_sc) = old.parse::<Shortcut>() {
        let _ = gs.unregister(old_sc);
    }
    gs.register(new).map_err(|e| e.to_string())?;
    save_toggle_shortcut(&app, &shortcut)?;
    app.state::<SharedState>().lock().unwrap().toggle_shortcut = shortcut;
    log("toggle shortcut updated");
    Ok(())
}

/// Open (or focus) the settings window. On macOS the app runs as an Accessory,
/// so bump the activation policy to Regular while settings is open, otherwise
/// the window cannot take keyboard focus to capture a shortcut.
fn open_settings(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.set_focus();
        return;
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let built = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Redactor Settings")
        .inner_size(360.0, 220.0)
        .resizable(false)
        .build();
    match built {
        Ok(w) => {
            let handle = app.clone();
            w.on_window_event(move |event| {
                if let tauri::WindowEvent::Destroyed = event {
                    // Re-register in case the window closed mid-capture (paused).
                    register_toggle(&handle);
                    #[cfg(target_os = "macos")]
                    let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
                }
            });
        }
        Err(e) => log(&format!("failed to open settings: {e}")),
    }
}

/// Debug-build remote control: poll a trigger file for commands so tests can
/// drive the app without synthetic input events. Enabled only when the
/// REDACTOR_DEBUG_TRIGGER env var points at a file.
#[cfg(debug_assertions)]
fn spawn_debug_trigger(app: AppHandle) {
    let Some(path) = std::env::var_os("REDACTOR_DEBUG_TRIGGER") else {
        return;
    };
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(300));
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        let cmd = contents.trim().to_string();
        if cmd.is_empty() {
            continue;
        }
        let _ = std::fs::write(&path, "");
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || {
            let mut parts = cmd.split_whitespace();
            match parts.next() {
                Some("toggle") => toggle(&handle),
                Some("exit") => exit_draw_mode(&handle),
                Some("panic") => clear_all(&handle),
                Some("simbox") => {
                    let nums: Vec<f64> = parts.filter_map(|p| p.parse().ok()).collect();
                    if let [x, y, w, h] = nums[..] {
                        add_box_internal(&handle, "0", x, y, w, h);
                    }
                }
                _ => log(&format!("unknown debug command: {cmd}")),
            }
        });
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(SharedState::default())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                // Only the toggle shortcut is ever registered, so any pressed
                // event is the toggle. Shortcuts are registered in setup once
                // the persisted value is loaded, and re-registered on change.
                .with_handler(move |app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            add_box,
            overlay_exit,
            overlay_cancel,
            get_toggle_shortcut,
            set_toggle_shortcut,
            pause_shortcut,
            resume_shortcut
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            let toggle_shortcut = load_toggle_shortcut(handle);
            app.state::<SharedState>().lock().unwrap().toggle_shortcut = toggle_shortcut;
            register_toggle(handle);

            let toggle_item = MenuItemBuilder::with_id("toggle", "Toggle draw mode").build(app)?;
            let settings_item = MenuItemBuilder::with_id("settings", "Settings…").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Redactor").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&toggle_item, &settings_item, &quit_item])
                .build()?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => toggle(app),
                    "settings" => open_settings(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            #[cfg(debug_assertions)]
            spawn_debug_trigger(app.handle().clone());

            // Check for updates in the background. A found update downloads and
            // installs silently and applies on the next launch; failures (no
            // network, no release yet) are logged and ignored.
            #[cfg(desktop)]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    match handle.updater() {
                        Ok(updater) => match updater.check().await {
                            Ok(Some(update)) => {
                                log(&format!("update available: {}", update.version));
                                if let Err(e) =
                                    update.download_and_install(|_, _| {}, || {}).await
                                {
                                    log(&format!("update install failed: {e}"));
                                } else {
                                    log("update installed; applies on next launch");
                                }
                            }
                            Ok(None) => log("no update available"),
                            Err(e) => log(&format!("update check failed: {e}")),
                        },
                        Err(e) => log(&format!("updater unavailable: {e}")),
                    }
                });
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app, event| {
        // Keep running as a tray app when all overlay windows close.
        if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
