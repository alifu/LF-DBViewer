import { h } from "../../lib/dom.js";
import { charsetsOf, collationsFor, defaultCollationFor } from "./collation-options.js";

const SERVER_DEFAULT = "";

function selectOf(name, value, options, placeholder) {
  return h(
    "select",
    { name },
    h("option", { value: SERVER_DEFAULT, selected: value === SERVER_DEFAULT }, placeholder),
    ...options.map((option) => h("option", { value: option, selected: option === value }, option)),
  );
}

/**
 * Create or alter a database's character set and collation.
 * `submit(change)` previews, confirms and applies; it resolves true when the change was made.
 */
export function openDatabaseDialog({ mode, name = "", charset = null, collation = null, options, submit }) {
  const creating = mode === "create";
  const nameInput = h("input", { name: "name", value: name, placeholder: "database_name", spellcheck: false, readOnly: !creating, required: true });
  const charsetSelect = selectOf("charset", charset ?? SERVER_DEFAULT, charsetsOf(options.collations), "(server default)");
  const collationSelect = h("select", { name: "collation" });
  const message = h("p", { class: "form-message error", role: "alert" });

  function drawCollations(selected) {
    const available = collationsFor(options.collations, charsetSelect.value).map((entry) => entry.name);
    const fallback = charsetSelect.value === SERVER_DEFAULT ? "(server default)" : `(default: ${defaultCollationFor(options.collations, charsetSelect.value)})`;
    collationSelect.replaceChildren(
      h("option", { value: SERVER_DEFAULT, selected: selected === SERVER_DEFAULT }, fallback),
      ...available.map((entry) => h("option", { value: entry, selected: entry === selected }, entry)),
    );
    collationSelect.disabled = charsetSelect.value === SERVER_DEFAULT;
  }
  drawCollations(collation ?? SERVER_DEFAULT);
  charsetSelect.addEventListener("change", () => drawCollations(SERVER_DEFAULT));

  const form = h(
    "form",
    {
      class: "database-form",
      onSubmit: async (event) => {
        event.preventDefault();
        message.textContent = "";
        const change = {
          kind: creating ? "createDatabase" : "alterDatabase",
          name: nameInput.value.trim(),
          charset: charsetSelect.value || null,
          collation: collationSelect.value || null,
        };
        try {
          if (await submit(change)) dialog.close();
        } catch (error) {
          message.textContent = error.message;
        }
      },
    },
    h("h2", {}, creating ? "New database" : `Alter database ${name}`),
    h("label", { class: "field" }, h("span", {}, "Name"), nameInput),
    h("label", { class: "field" }, h("span", {}, "Character set"), charsetSelect),
    h("label", { class: "field" }, h("span", {}, "Collation"), collationSelect),
    message,
    h(
      "div",
      { class: "dialog-actions" },
      h("button", { type: "button", onClick: () => dialog.close() }, "Cancel"),
      h("button", { type: "submit", class: "primary" }, "Review SQL…"),
    ),
  );
  const dialog = h("dialog", { class: "dialog database-dialog" }, form);
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  (creating ? nameInput : charsetSelect).focus();
}
