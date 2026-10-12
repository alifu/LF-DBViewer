import { h } from "../../lib/dom.js";
import { inputParameters } from "./routine-call.js";

/** Asks for the parameter values of a procedure or function. `run(values)` gets text by position, or null for NULL. */
export function openRunDialog({ kind, name, parameters, run }) {
  const inputs = inputParameters(parameters);
  const rows = inputs.map((parameter) => {
    const text = h("input", { spellcheck: false, placeholder: parameter.dataType, "aria-label": parameter.name ?? `Parameter ${parameter.position}` });
    const isNull = h("input", { type: "checkbox", "aria-label": "NULL", onChange: () => (text.disabled = isNull.checked) });
    return { parameter, text, isNull, element: h("div", { class: "run-row" }, h("label", { class: "field" }, h("span", {}, `${parameter.name ?? `#${parameter.position}`} · ${parameter.mode ?? ""} ${parameter.dataType}`), text), h("label", { class: "check" }, isNull, "NULL")) };
  });
  const dialog = h(
    "dialog",
    { class: "dialog run-dialog" },
    h("h2", {}, `Run ${kind} ${name}`),
    rows.length === 0 ? h("p", { class: "muted" }, "No input parameters.") : h("div", {}, ...rows.map((row) => row.element)),
    h(
      "div",
      { class: "dialog-actions" },
      h("button", { type: "button", onClick: () => dialog.close() }, "Cancel"),
      h(
        "button",
        {
          type: "button",
          class: "primary",
          onClick: () => {
            const values = Object.fromEntries(rows.map((row) => [row.parameter.position, row.isNull.checked ? null : row.text.value]));
            dialog.close();
            run(values);
          },
        },
        "Run",
      ),
    ),
  );
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  rows[0]?.text.focus();
}
