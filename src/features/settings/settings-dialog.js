import { h } from "../../lib/dom.js";

const APPEARANCE_OPTIONS = Object.freeze([
  { value: "system", label: "System", hint: "Follow the macOS appearance." },
  { value: "light", label: "Light", hint: "" },
  { value: "dark", label: "Dark", hint: "" },
]);

function appearanceSection(theme) {
  const options = APPEARANCE_OPTIONS.map((option) =>
    h(
      "div",
      {},
      h(
        "label",
        { class: "settings-option" },
        h("input", {
          type: "radio",
          name: "appearance",
          value: option.value,
          checked: theme.getPreference() === option.value,
          onChange: () => theme.setPreference(option.value),
        }),
        option.label,
      ),
      option.hint && h("p", { class: "settings-hint muted" }, option.hint),
    ),
  );
  return h("fieldset", { class: "settings-section" }, h("legend", {}, "Appearance"), ...options);
}

let openDialog = null;

/** Modal settings window. A second request while it is open just keeps the first. */
export function openSettings({ theme }) {
  if (openDialog !== null) return;
  const dialog = h(
    "dialog",
    { class: "dialog settings" },
    h("h2", {}, "Settings"),
    appearanceSection(theme),
    h("div", { class: "dialog-actions" }, h("button", { type: "button", class: "primary", onClick: () => dialog.close() }, "Done")),
  );
  dialog.addEventListener("close", () => {
    dialog.remove();
    openDialog = null;
  });
  openDialog = dialog;
  document.body.append(dialog);
  dialog.showModal();
}
