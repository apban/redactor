mod watcher;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutState};

const WATCH_TIMEOUT: Duration = Duration::from_secs(120);
const TOGGLE_SHORTCUT: &str = "CmdOrCtrl+Alt+B";
const PANIC_SHORTCUT: &str = "CmdOrCtrl+Alt+Shift+B";

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
}

type SharedState = Mutex<RedactorState>;

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
    let index = window.label().strip_prefix("draw-").unwrap_or("0").to_string();
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
                    let nums: Vec<f64> =
                        parts.filter_map(|p| p.parse().ok()).collect();
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
    let toggle_shortcut: Shortcut = TOGGLE_SHORTCUT.parse().unwrap();
    let panic_shortcut: Shortcut = PANIC_SHORTCUT.parse().unwrap();

    let app = tauri::Builder::default()
        .manage(SharedState::default())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([TOGGLE_SHORTCUT, PANIC_SHORTCUT])
                .expect("invalid shortcut definition")
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut == &toggle_shortcut {
                        toggle(app);
                    } else if shortcut == &panic_shortcut {
                        clear_all(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            add_box,
            overlay_exit,
            overlay_cancel
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let toggle_item =
                MenuItemBuilder::with_id("toggle", "Toggle draw mode").build(app)?;
            let panic_item =
                MenuItemBuilder::with_id("panic", "Panic clear").build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Redactor").build(app)?;
            let menu = MenuBuilder::new(app)
                .items(&[&toggle_item, &panic_item, &quit_item])
                .build()?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "toggle" => toggle(app),
                    "panic" => clear_all(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            #[cfg(debug_assertions)]
            spawn_debug_trigger(app.handle().clone());

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
