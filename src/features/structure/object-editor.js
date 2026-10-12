import { h } from "../../lib/dom.js";
import { createSqlEditor } from "../query/editor.js";
import { objectNameFromSql } from "./object-name.js";
import { templateFor } from "./object-templates.js";

/**
 * Edits the `CREATE` statement of a view, procedure, function, trigger or event.
 * Saving shows the SQL first. `data`: `load()` (the current definition), `review(sql)` (preview, confirm,
 * apply; resolves true when saved), `run()` for routines, and `onSaved(name)` with the object's name.
 */
export function createObjectEditor({ tab, data, engine }) {
  const kind = tab.kind === "viewdef" ? "view" : tab.kind;
  const isRoutine = kind === "procedure" || kind === "function";
  let savedText = tab.isNew ? templateFor(kind) : "";
  let currentName = tab.isNew ? null : tab.name;

  const message = h("span", { class: "designer-message", role: "status" });
  const editorHost = h("div", { class: "object-editor-host" });
  const saveButton = h("button", { type: "button", class: "primary", title: "Review the SQL, then save (⌘↵)", onClick: () => void save() }, "Review & save");
  const revertButton = h("button", { type: "button", onClick: () => void load() }, "Revert");
  const runButton = h("button", { type: "button", title: "Run with parameter values", onClick: () => void data.run() }, "Run…");
  runButton.hidden = !isRoutine || tab.isNew;
  const element = h(
    "div",
    { class: "object-editor" },
    h("div", { class: "query-toolbar" }, saveButton, revertButton, runButton, h("span", { class: "spacer" }), message),
    editorHost,
  );
  const editor = createSqlEditor({ parent: editorHost, engine, onRun: () => void save(), onCancel: () => {}, onChange: () => {} });
  editor.setText(savedText);

  function say(text, kind = "") {
    message.textContent = text;
    message.className = `designer-message ${kind}`;
  }

  async function load() {
    if (tab.isNew && currentName === null) {
      editor.setText(templateFor(kind));
      savedText = editor.getText();
      return;
    }
    say("Loading…");
    try {
      savedText = await data.load(currentName);
      editor.setText(savedText);
      say("");
    } catch (error) {
      say(error.message, "error");
    }
  }

  async function save() {
    const sql = editor.getText();
    say("");
    saveButton.disabled = true;
    try {
      if (!(await data.review({ sql, existingName: currentName }))) return;
      savedText = sql;
      const name = objectNameFromSql(sql) ?? currentName;
      say("Saved.", "ok");
      data.onSaved(name, currentName);
      currentName = name;
    } catch (error) {
      say(error.message, "error");
    } finally {
      saveButton.disabled = false;
    }
  }

  if (!tab.isNew) void load();

  return {
    element,
    reload: () => (editor.getText() === savedText ? load().then(() => true) : Promise.resolve(false)),
    focus: () => editor.focus(),
    hasUnsavedChanges: () => editor.getText() !== savedText,
    destroy: () => editor.destroy(),
  };
}
