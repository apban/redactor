# Redactor

A [Hammerspoon](https://www.hammerspoon.org/) Spoon for macOS that lets you draw black redaction boxes on screen, then take a clipboard screenshot with the boxes baked into the image. Boxes disappear automatically once the screenshot lands on the clipboard.

## Why

You're about to screenshot something with an API key, customer name, or other sensitive content visible. Editing the screenshot after the fact is annoying. Redactor lets you cover the sensitive parts first, then drag your normal screenshot region over the area.

## Requirements

- macOS
- [Hammerspoon](https://www.hammerspoon.org/) (free, MIT-licensed). Install with `brew install --cask hammerspoon`.
- Accessibility permission for Hammerspoon (System Settings → Privacy & Security → Accessibility).

## Install

```bash
git clone https://github.com/apban/redactor.git ~/redactor
ln -s ~/redactor/Redactor.spoon ~/.hammerspoon/Spoons/Redactor.spoon
```

The symlink lets you `git pull` to update without reinstalling. If you'd rather just copy:

```bash
git clone https://github.com/apban/redactor.git
cp -R redactor/Redactor.spoon ~/.hammerspoon/Spoons/
```

## Configure

Add to your `~/.hammerspoon/init.lua`:

```lua
hs.loadSpoon("Redactor")

spoon.Redactor:bindHotkeys({
  toggle = { {"cmd", "alt"}, "b" },          -- enter/exit draw mode
  panic  = { {"cmd", "alt", "shift"}, "b" }, -- clear all boxes
})
```

Reload Hammerspoon (menu bar icon → Reload Config).

## Usage

1. **⌘⌥B** to enter draw mode. The screen tints slightly so you know you're in.
2. Click-drag to add black boxes over anything sensitive. Repeat for as many as you need.
3. Press **Return** to commit (or ⌘⌥B again). Boxes stay on screen.
4. Take your screenshot. macOS default is **⌘⌃⇧4** (drag region → clipboard). If you've remapped the screenshot shortcut to ⌘⇧S in System Settings, that also works (see [Screenshot hotkey passthrough](#screenshot-hotkey-passthrough)).
5. The moment the image lands on your clipboard, the boxes disappear.

### Escape hatches

- **Esc** during draw mode: cancel and clear what you just drew.
- **⌘⌥⇧B** any time: panic clear — kills all boxes and stops the watcher.
- The clipboard watcher times out after 120 seconds if you walk away without screenshotting.

## Screenshot hotkey passthrough

While in draw mode, an active event tap captures all mouse drags so they draw boxes instead of triggering whatever app is below. That means if you trigger your screenshot tool without exiting draw mode first, your screenshot drag will get eaten and turn into another box.

Redactor solves this by watching for a specific keystroke that should exit draw mode AND pass through to your screenshot tool. Default: **⌘⇧S** (matches users who've remapped the macOS screenshot shortcut to ⌘⇧S).

To customize for a different screenshot key, set the properties before `bindHotkeys`:

```lua
spoon.Redactor.screenshotPassthroughKey  = "4"
spoon.Redactor.screenshotPassthroughMods = { cmd = true, ctrl = true, shift = true }
spoon.Redactor:bindHotkeys({ ... })
```

To disable passthrough entirely (and always require an explicit Return before screenshotting), set the key to `nil`.

## Configuration reference

All properties are set on `spoon.Redactor` before calling `bindHotkeys`.

| Property | Default | What it does |
|---|---|---|
| `boxLevel` | `hs.canvas.windowLevels.screenSaver` | Window level for committed boxes. If your screenshot tool dims them during capture, bump to `assistiveTechHigh`. |
| `overlayLevel` | `hs.canvas.windowLevels.overlay` | Window level for the draw-mode tint. |
| `watchTimeout` | `120` | Seconds the clipboard watcher waits before giving up. |
| `screenshotPassthroughKey` | `"s"` | Key (string name) that exits draw mode and passes through. `nil` disables. |
| `screenshotPassthroughMods` | `{cmd=true, shift=true}` | Modifier flags required for the passthrough key. |

## Troubleshooting

**My screenshot captures a dimmed version of the boxes instead of solid black.**
Your screenshot tool is drawing above the boxes. Bump the window level:
```lua
spoon.Redactor.boxLevel = hs.canvas.windowLevels.assistiveTechHigh
```

**Nothing happens when I press the hotkey.**
Check that Hammerspoon has Accessibility permission. Then reload config and check the Hammerspoon console for errors (menu bar icon → Console).

**The boxes won't go away.**
Press ⌘⌥⇧B (panic clear). If that fails, reload Hammerspoon config.

## License

MIT. See [LICENSE](LICENSE).
