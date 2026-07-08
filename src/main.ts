import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

type BoxRect = { x: number; y: number; w: number; h: number };

const MIN_BOX_SIZE = 4;

function makeBox(rect: BoxRect): HTMLDivElement {
  const el = document.createElement("div");
  el.className = "box";
  placeBox(el, rect);
  document.body.append(el);
  return el;
}

function placeBox(el: HTMLDivElement, { x, y, w, h }: BoxRect) {
  el.style.left = `${x}px`;
  el.style.top = `${y}px`;
  el.style.width = `${w}px`;
  el.style.height = `${h}px`;
}

const label = getCurrentWebviewWindow().label;

if (label.startsWith("boxes-")) {
  // Boxes layer: click-through, capturable. Renders boxes it is told about.
  document.body.className = "boxes";
  void listen<BoxRect>("add-box", (event) => {
    makeBox(event.payload);
  });
} else {
  // Draw layer: interactive, excluded from screen captures. The tint is
  // capture-excluded on macOS/Windows; on Linux there is no exclusion API,
  // so skip the tint to keep captures clean.
  const linux = navigator.userAgent.includes("Linux");
  document.body.className = linux ? "draw-untinted" : "draw";

  let startPt: { x: number; y: number } | null = null;
  let preview: HTMLDivElement | null = null;

  document.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    startPt = { x: e.clientX, y: e.clientY };
    preview = makeBox({ x: e.clientX, y: e.clientY, w: 1, h: 1 });
  });

  document.addEventListener("pointermove", (e) => {
    if (!startPt || !preview) return;
    placeBox(preview, {
      x: Math.min(startPt.x, e.clientX),
      y: Math.min(startPt.y, e.clientY),
      w: Math.max(1, Math.abs(e.clientX - startPt.x)),
      h: Math.max(1, Math.abs(e.clientY - startPt.y)),
    });
  });

  document.addEventListener("pointerup", () => {
    if (!preview) return;
    const rect: BoxRect = {
      x: preview.offsetLeft,
      y: preview.offsetTop,
      w: preview.offsetWidth,
      h: preview.offsetHeight,
    };
    preview.remove();
    preview = null;
    startPt = null;
    if (rect.w >= MIN_BOX_SIZE && rect.h >= MIN_BOX_SIZE) {
      void invoke("add_box", rect);
    }
  });

  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") {
      void invoke("overlay_cancel");
    } else if (e.key === "Enter") {
      void invoke("overlay_exit");
    }
  });
}
