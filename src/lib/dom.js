/** Sets `element[key]`; false for read-only properties such as `input.list`, which are set as attributes instead. */
function setProperty(element, key, value) {
  try {
    element[key] = value;
    return true;
  } catch {
    return false;
  }
}

/**
 * Creates an element. `props` keys starting with "on" become listeners, `class`
 * and `dataset` are handled specially, and null/false children are skipped.
 */
export function h(tag, props = {}, ...children) {
  const element = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) element.addEventListener(key.slice(2).toLowerCase(), value);
    else if (key === "class") element.className = value;
    else if (key === "dataset") Object.assign(element.dataset, value);
    else if (!(key in element) || !setProperty(element, key, value)) element.setAttribute(key, value === true ? "" : value);
  }
  element.append(...children.flat().filter((child) => child !== null && child !== undefined && child !== false));
  return element;
}

/** Shows a modal question and resolves to true when the user confirms. */
export function confirmDialog({ title, body, confirmLabel = "OK", danger = false }) {
  return new Promise((resolve) => {
    const dialog = h(
      "dialog",
      { class: "dialog" },
      h("h2", {}, title),
      h("p", { class: "dialog-body" }, body),
      h(
        "div",
        { class: "dialog-actions" },
        h("button", { type: "button", onClick: () => dialog.close("cancel") }, "Cancel"),
        h(
          "button",
          { type: "button", class: danger ? "danger" : "primary", onClick: () => dialog.close("confirm") },
          confirmLabel,
        ),
      ),
    );
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(dialog.returnValue === "confirm");
    });
    document.body.append(dialog);
    dialog.showModal();
  });
}

/** Shows a message the user has to dismiss; resolves when it is closed. */
export function alertDialog({ title, body }) {
  return new Promise((resolve) => {
    const dialog = h(
      "dialog",
      { class: "dialog" },
      h("h2", {}, title),
      h("p", { class: "dialog-body" }, body),
      h("div", { class: "dialog-actions" }, h("button", { type: "button", class: "primary", onClick: () => dialog.close() }, "OK")),
    );
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve();
    });
    document.body.append(dialog);
    dialog.showModal();
  });
}
