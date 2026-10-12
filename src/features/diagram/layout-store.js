const PREFIX = "lf-dbviewer.diagram.";
const MIN_SCALE = 0.05;
const MAX_SCALE = 4;

const isNumber = (value) => typeof value === "number" && Number.isFinite(value);

/** What is kept for one database: where the boxes are, the pan and zoom, and whether only key columns show. */
export function emptyLayout() {
  return { positions: {}, view: null, keyColumnsOnly: false };
}

function cleaned(value) {
  const layout = emptyLayout();
  if (value === null || typeof value !== "object") return layout;
  for (const [id, position] of Object.entries(value.positions ?? {})) {
    if (position && isNumber(position.x) && isNumber(position.y)) layout.positions[id] = { x: position.x, y: position.y };
  }
  const view = value.view;
  if (view && isNumber(view.x) && isNumber(view.y) && isNumber(view.scale) && view.scale >= MIN_SCALE && view.scale <= MAX_SCALE) layout.view = { x: view.x, y: view.y, scale: view.scale };
  layout.keyColumnsOnly = value.keyColumnsOnly === true;
  return layout;
}

export const layoutKey = (serverKey, database) => `${PREFIX}${serverKey}/${database}`;

/** Storage can be missing, blocked or hold something odd: the diagram then simply lays itself out. */
export function loadLayout(storage, key) {
  try {
    return cleaned(JSON.parse(storage.getItem(key) ?? "null"));
  } catch {
    return emptyLayout();
  }
}

export function saveLayout(storage, key, layout) {
  try {
    storage.setItem(key, JSON.stringify(layout));
  } catch {
    // A layout is a convenience; losing it must not get in the way.
  }
}
