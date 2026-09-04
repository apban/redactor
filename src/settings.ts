import { invoke } from "@tauri-apps/api/core";

const captureBtn = document.getElementById("capture") as HTMLButtonElement;
const saveBtn = document.getElementById("save") as HTMLButtonElement;
const statusEl = document.getElementById("status") as HTMLParagraphElement;

let pending: string | null = null;
let capturing = false;
let current = "";

// Map a KeyboardEvent.code to the main key token Tauri's accelerator parser
// accepts. Returns null for keys we do not support or for modifier-only codes.
function mainKey(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  const map: Record<string, string> = {
    Space: "Space",
    Minus: "-",
    Equal: "=",
    BracketLeft: "[",
    BracketRight: "]",
    Backslash: "\\",
    Semicolon: ";",
    Quote: "'",
    Comma: ",",
    Period: ".",
    Slash: "/",
    Backquote: "`",
  };
  return map[code] ?? null;
}

// Render a Tauri accelerator string as macOS glyphs, e.g. "CmdOrCtrl+Alt+B"
// becomes "⌥⌘B". Modifiers sort into the mac order ⌃⌥⇧⌘ before the key.
function toSymbols(accel: string): string {
  const glyph: Record<string, string> = {
    Control: "⌃",
    Ctrl: "⌃",
    Alt: "⌥",
    Option: "⌥",
    Shift: "⇧",
    Cmd: "⌘",
    Command: "⌘",
    CmdOrCtrl: "⌘",
    Super: "⌘",
  };
  const rank: Record<string, number> = { "⌃": 0, "⌥": 1, "⇧": 2, "⌘": 3 };
  const mods: string[] = [];
  let key = "";
  for (const part of accel.split("+")) {
    if (glyph[part]) mods.push(glyph[part]);
    else key = part;
  }
  const uniq = [...new Set(mods)].sort((a, b) => rank[a] - rank[b]);
  return uniq.join("") + key;
}

// Build a Tauri accelerator string from a KeyboardEvent, or null if the combo
// is not valid (no non-modifier key, or no modifier).
function accelerator(e: KeyboardEvent): string | null {
  const mods: string[] = [];
  if (e.metaKey) mods.push("Cmd");
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  const key = mainKey(e.code);
  if (!key || mods.length === 0) return null;
  return [...mods, key].join("+");
}

function startCapture() {
  capturing = true;
  captureBtn.classList.add("capturing");
  captureBtn.textContent = "Press keys…";
  // Suspend the global shortcut so the combo reaches this window instead of
  // toggling redactor while we capture it.
  void invoke("pause_shortcut");
}

function stopCapture() {
  capturing = false;
  captureBtn.classList.remove("capturing");
}

captureBtn.addEventListener("click", startCapture);

document.addEventListener("keydown", (e) => {
  if (!capturing) return;
  e.preventDefault();
  if (e.key === "Escape") {
    stopCapture();
    captureBtn.textContent = toSymbols(pending ?? current);
    // Nothing saved, so restore the live shortcut.
    void invoke("resume_shortcut");
    return;
  }
  const combo = accelerator(e);
  if (!combo) return; // wait for a non-modifier key with a modifier held
  pending = combo;
  captureBtn.textContent = toSymbols(combo);
  saveBtn.disabled = combo === current;
  statusEl.textContent = "";
  stopCapture();
});

saveBtn.addEventListener("click", async () => {
  if (!pending) return;
  saveBtn.disabled = true;
  try {
    await invoke("set_toggle_shortcut", { shortcut: pending });
    current = pending;
    statusEl.textContent = "Saved.";
    statusEl.className = "status ok";
  } catch (err) {
    statusEl.textContent = String(err);
    statusEl.className = "status err";
    saveBtn.disabled = false;
  }
});

(async () => {
  current = await invoke<string>("get_toggle_shortcut");
  captureBtn.textContent = current ? toSymbols(current) : "…";
})();
