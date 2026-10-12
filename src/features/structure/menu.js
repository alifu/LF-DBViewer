import { h } from "../../lib/dom.js";

const EDGE_GAP = 6;

/** Keeps a popup of the given size inside the window. */
export function placeMenu({ x, y, width, height, viewportWidth, viewportHeight }) {
  return {
    left: Math.max(EDGE_GAP, Math.min(x, viewportWidth - width - EDGE_GAP)),
    top: Math.max(EDGE_GAP, Math.min(y, viewportHeight - height - EDGE_GAP)),
  };
}

let closeOpenMenu = null;

/**
 * A context menu at the pointer. `items` are `{ label, onSelect, danger?, disabled? }`,
 * or `"-"` for a separator. It closes on a click elsewhere, Escape, or choosing an item.
 */
export function showMenu(event, items) {
  event.preventDefault();
  closeOpenMenu?.();
  const entries = items.map((item) =>
    item === "-"
      ? h("div", { class: "menu-separator", role: "separator" })
      : h(
          "button",
          { type: "button", class: item.danger ? "menu-item danger" : "menu-item", disabled: Boolean(item.disabled), role: "menuitem", onClick: () => (close(), item.onSelect()) },
          item.label,
        ),
  );
  const menu = h("div", { class: "menu", role: "menu" }, ...entries);
  document.body.append(menu);
  const { left, top } = placeMenu({
    x: event.clientX,
    y: event.clientY,
    width: menu.offsetWidth,
    height: menu.offsetHeight,
    viewportWidth: window.innerWidth,
    viewportHeight: window.innerHeight,
  });
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;

  const onKey = (keyEvent) => keyEvent.key === "Escape" && close();
  const onPointer = (pointerEvent) => !menu.contains(pointerEvent.target) && close();
  function close() {
    menu.remove();
    window.removeEventListener("keydown", onKey);
    window.removeEventListener("pointerdown", onPointer, true);
    closeOpenMenu = null;
  }
  window.addEventListener("keydown", onKey);
  window.addEventListener("pointerdown", onPointer, true);
  closeOpenMenu = close;
}
