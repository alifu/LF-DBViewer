import { h } from "../../lib/dom.js";

/**
 * Shows the exact SQL a change will run and asks for confirmation. Resolves to true if confirmed.
 * With `confirmText` the button stays disabled until that text is typed, for changes that destroy data.
 */
export function reviewSql({ title, statements, confirmLabel = "Run", danger = false, note = "", confirmText = null }) {
  return new Promise((resolve) => {
    const confirmButton = h("button", { type: "button", class: danger ? "danger" : "primary", onClick: () => dialog.close("confirm") }, confirmLabel);
    confirmButton.disabled = confirmText !== null;
    const typed =
      confirmText === null
        ? null
        : h("input", {
            placeholder: `Type ${confirmText} to confirm`,
            "aria-label": "Confirmation",
            spellcheck: false,
            onInput: () => (confirmButton.disabled = typed.value !== confirmText),
          });
    const dialog = h(
      "dialog",
      { class: "dialog review-dialog" },
      h("h2", {}, title),
      note && h("p", { class: "muted dialog-body" }, note),
      h("pre", { class: "review-sql" }, statements.map((statement) => `${statement};`).join("\n\n")),
      typed,
      h("div", { class: "dialog-actions" }, h("button", { type: "button", onClick: () => dialog.close("cancel") }, "Cancel"), confirmButton),
    );
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(dialog.returnValue === "confirm");
    });
    document.body.append(dialog);
    dialog.showModal();
    (typed ?? confirmButton).focus();
  });
}
