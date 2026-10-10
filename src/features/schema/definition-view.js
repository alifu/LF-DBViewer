import { h } from "../../lib/dom.js";

/** Read-only `CREATE …` statement of a procedure, function, trigger or event. */
export function createDefinitionView({ load }) {
  const code = h("pre", { class: "definition" });
  const element = h("div", { class: "definition-view" }, code);

  async function reload() {
    code.className = "definition muted";
    code.textContent = "Loading…";
    try {
      code.textContent = await load();
      code.className = "definition";
    } catch (error) {
      code.textContent = error.message;
      code.className = "definition error-text";
    }
  }

  void reload();
  return { element, reload, hasUnsavedChanges: () => false };
}
