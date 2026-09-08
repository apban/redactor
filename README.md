# Redactor

Draw black redaction boxes on your screen, take a screenshot or recording with the boxes baked into the image, and the boxes disappear the moment the capture completes.

## Why

You're about to screenshot something with an API key, customer name, or other sensitive content visible. Editing the screenshot after the fact is annoying. Redactor lets you cover the sensitive parts first, then take your normal screenshot over the area.

## Usage

1. Press **Cmd+Alt+B** (macOS) or **Ctrl+Alt+B** (Windows/Linux) to enter draw mode. The screen tints slightly.
2. Click-drag black boxes over anything sensitive. Repeat as needed.
3. Take your screenshot or screen recording with your OS capture tool, to clipboard or to file. Custom keybinds work too. No need to leave draw mode first: the tint and crosshair are excluded from captures, only the boxes are baked in.
4. The moment the capture completes (image on the clipboard, or a new file in your screenshot/recording folder), the boxes disappear. If macOS shows the floating corner thumbnail, the capture completes when the thumbnail goes away.

Optional: **Return** or the toggle hotkey exits draw mode early, keeping the boxes up (click-through) while you do something else before capturing.

### Escape hatches

- **Esc** during draw mode: cancel and clear everything.
- **Cmd/Ctrl+Alt+Shift+B** any time: panic clear.
- If you never take a screenshot, boxes clear themselves after 120 seconds.

## Install

Grab the build for your OS from [Releases](../../releases). The builds are unsigned:

- **macOS**: right-click the app, choose Open, confirm. Only needed the first time.
- **Windows**: if SmartScreen appears, click More info, then Run anyway.
- **Linux**: `chmod +x` the AppImage and run it. X11 works; Wayland is best effort (global hotkeys depend on your compositor; use the tray menu if they don't fire).

No permissions are required on macOS. Redactor does not use Accessibility or Screen Recording APIs; the screenshot is taken by the OS tool you already use.

## Build from source

Requires [Rust](https://rustup.rs/) and Node 20+.

```bash
npm install
npm run tauri build
```

Dev mode: `npm run tauri dev`.

## How it works

Two transparent, always-on-top windows per monitor. The boxes layer is click-through and holds the black rectangles; the OS screenshot tool composites it into the capture, so the boxes are baked into the pixels, not metadata. The draw layer (tint, crosshair, drag preview) sits above it and is excluded from screen captures (`NSWindowSharingNone` on macOS, `WDA_EXCLUDEFROMCAPTURE` on Windows; Linux has no exclusion API, so the tint is skipped there). A background thread watches for a completed capture: a new image on the clipboard, or a new image/video file in the OS capture folders (on macOS it reads your configured `com.apple.screencapture` locations). Either signal clears everything.

## License

MIT. See [LICENSE](LICENSE).
