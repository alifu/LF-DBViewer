import { h } from "../../lib/dom.js";
import { formatBytes } from "../transfer/transfer-state.js";
import { hexDump } from "./hex-dump.js";

/**
 * Shows one cell in full: text in a box (editable when `onApply` is given), binary as a hex dump.
 * `load()` resolves to `{ binary, text, size, truncated }` from the backend.
 */
export function openCellDialog({ title, load, onApply }) {
  const note = h("p", { class: "muted" }, "Loading…");
  const area = h("textarea", { class: "cell-text", readOnly: true, spellcheck: false, "aria-label": "Value" });
  const apply = h("button", { type: "button", class: "primary", hidden: true }, "Use this value");
  const copy = h("button", { type: "button", onClick: () => void navigator.clipboard?.writeText(area.value) }, "Copy");
  const dialog = h("dialog", { class: "dialog cell-dialog" }, h("h2", {}, title), note, area, h("div", { class: "dialog-actions" }, copy, apply, h("button", { type: "button", onClick: () => dialog.close() }, "Close")));
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();

  load().then(
    (value) => {
      const shown = value.binary ? hexDump(value.text) : value.text;
      area.value = shown;
      area.classList.toggle("hex", value.binary);
      note.textContent = `${value.binary ? "Binary" : "Text"} · ${formatBytes(value.size)}${value.truncated ? " · only the first part is shown" : ""}`;
      // A cut-off value must not be written back: the rest would be lost.
      if (onApply && !value.binary && !value.truncated) {
        area.readOnly = false;
        apply.hidden = false;
        apply.addEventListener("click", () => (onApply(area.value), dialog.close()));
      }
    },
    (error) => (note.textContent = error.message),
  );
  return dialog;
}
