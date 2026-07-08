use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arboard::Clipboard;
use tauri::AppHandle;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// File extensions the OS capture tools produce (screenshots and recordings).
const CAPTURE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "tiff", "heic", "pdf", "mov", "mp4", "m4v", "webm",
];

/// Watch for a completed capture: a new image on the clipboard OR a new
/// screenshot/recording file in the OS capture directories. When either
/// fires, or when the timeout expires, clear all boxes. Returns a stop flag;
/// setting it makes the watcher exit without touching anything.
///
/// Clipboard change detection is a sparse fingerprint of the clipboard image,
/// matching the spoon's changeCount semantics: a non-image clipboard change
/// (user copied text) re-baselines and keeps waiting.
/// TODO: platform-native change counters (NSPasteboard changeCount,
/// GetClipboardSequenceNumber) would avoid decoding the image on every poll.
pub fn start(app: AppHandle, timeout: Duration) -> (Arc<AtomicBool>, Arc<Mutex<Instant>>) {
    let stop = Arc::new(AtomicBool::new(false));
    let deadline = Arc::new(Mutex::new(Instant::now() + timeout));
    let flag = stop.clone();
    let shared_deadline = deadline.clone();
    std::thread::spawn(move || {
        let mut clipboard = Clipboard::new().ok();
        let mut clip_baseline = clipboard.as_mut().and_then(fingerprint);
        let dirs = capture_dirs();
        let file_baseline = snapshot_captures(&dirs);
        crate::log(&format!(
            "watcher: started, clip_baseline={}, dirs={:?}, baseline_files={}",
            clip_baseline.is_some(),
            dirs,
            file_baseline.len()
        ));
        loop {
            if Instant::now() >= *shared_deadline.lock().unwrap() {
                crate::log("watcher: timeout expired");
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
            if flag.load(Ordering::Relaxed) {
                crate::log("watcher: stopped externally");
                return;
            }
            if let Some(path) = find_new_capture(&dirs, &file_baseline) {
                crate::log(&format!("watcher: new capture file {path:?}"));
                break;
            }
            let current = clipboard.as_mut().and_then(fingerprint);
            if current != clip_baseline {
                if current.is_some() {
                    crate::log("watcher: new clipboard image");
                    break;
                }
                crate::log("watcher: clipboard became non-image, re-baselining");
                clip_baseline = current;
            }
        }
        if flag.load(Ordering::Relaxed) {
            return;
        }
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || crate::clear_all(&handle));
    });
    (stop, deadline)
}

fn fingerprint(clipboard: &mut Clipboard) -> Option<u64> {
    let img = clipboard.get_image().ok()?;
    let bytes = &img.bytes;
    let mut hash: u64 = ((img.width as u64) << 32)
        ^ (img.height as u64)
        ^ (bytes.len() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let step = (bytes.len() / 64).max(1);
    let mut i = 0;
    while i < bytes.len() {
        hash = hash.wrapping_mul(1_099_511_628_211).wrapping_add(bytes[i] as u64);
        i += step;
    }
    Some(hash)
}

/// Directories the OS capture tools save into.
fn capture_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let desktop = home_join("Desktop");
        dirs.push(screencapture_default("location").unwrap_or(desktop));
        if let Some(rec) = screencapture_default("location-for-screen-recordings") {
            dirs.push(rec);
        }
    }

    #[cfg(target_os = "windows")]
    {
        dirs.push(home_join("Pictures\\Screenshots"));
        dirs.push(home_join("Videos\\Captures"));
    }

    #[cfg(target_os = "linux")]
    {
        dirs.push(home_join("Pictures"));
        dirs.push(home_join("Pictures/Screenshots"));
        dirs.push(home_join("Videos/Screencasts"));
    }

    dirs.dedup();
    dirs
}

fn home_join(rel: &str) -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(rel)
}

#[cfg(target_os = "macos")]
fn screencapture_default(key: &str) -> Option<PathBuf> {
    let out = std::process::Command::new("defaults")
        .args(["read", "com.apple.screencapture", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        Some(home_join(rest))
    } else {
        Some(PathBuf::from(raw))
    }
}

fn is_capture_file(path: &PathBuf) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| CAPTURE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

fn snapshot_captures(dirs: &[PathBuf]) -> HashSet<PathBuf> {
    let mut seen = HashSet::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_capture_file(&path) {
                seen.insert(path);
            }
        }
    }
    seen
}

fn find_new_capture(dirs: &[PathBuf], baseline: &HashSet<PathBuf>) -> Option<PathBuf> {
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if is_capture_file(&path) && !baseline.contains(&path) {
                return Some(path);
            }
        }
    }
    None
}
