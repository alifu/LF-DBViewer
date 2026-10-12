/** Sizes, anchor points, curves and pan/zoom maths of the diagram. All numbers are in diagram units unless a name says screen. */

export const HEADER_HEIGHT = 30;
export const ROW_HEIGHT = 22;
export const BOX_PADDING = 10;
export const BOX_FOOTER = 6;
export const CHAR_WIDTH = 7.2;
export const MIN_BOX_WIDTH = 170;
export const MAX_BOX_WIDTH = 400;
export const MAX_NAME_CHARS = 46;
const TAG_WIDTH = 30;
const MIN_CURVE = 40;
const LOOP_REACH = 46;

export function ellipsize(text, maxChars) {
  return text.length <= maxChars ? text : `${text.slice(0, maxChars - 1)}…`;
}

/** Width that fits the longest column name and type, or the table name. */
export function boxWidth(title, rows) {
  const titleWidth = ellipsize(title, MAX_NAME_CHARS).length * CHAR_WIDTH * 1.15 + 2 * BOX_PADDING;
  const rowWidth = rows.reduce((widest, row) => Math.max(widest, (ellipsize(row.name, MAX_NAME_CHARS).length + ellipsize(row.type, 24).length + 3) * CHAR_WIDTH + 2 * BOX_PADDING + TAG_WIDTH), 0);
  return Math.min(MAX_BOX_WIDTH, Math.max(MIN_BOX_WIDTH, Math.ceil(Math.max(titleWidth, rowWidth))));
}

export function boxHeight(rowCount) {
  return HEADER_HEIGHT + rowCount * ROW_HEIGHT + BOX_FOOTER;
}

/** Vertical middle of a row, from the top of the box. */
export function rowCenter(rowIndex) {
  return HEADER_HEIGHT + rowIndex * ROW_HEIGHT + ROW_HEIGHT / 2;
}

/** Where a line meets a box: the left or right edge, at the height of a row (or of the header for `null`). */
export function anchor(box, position, rowIndex, side) {
  return { x: side === "right" ? position.x + box.width : position.x, y: position.y + (rowIndex === null ? HEADER_HEIGHT / 2 : rowCenter(rowIndex)), side };
}

/** Lines leave the side that faces the other box; boxes that overlap horizontally, and a table to itself, loop out on the right. */
export function chooseSides(from, fromPosition, to, toPosition) {
  if (from === to) return ["right", "right"];
  if (fromPosition.x + from.width < toPosition.x) return ["right", "left"];
  if (toPosition.x + to.width < fromPosition.x) return ["left", "right"];
  return ["right", "right"];
}

/** A smooth curve between two anchors, leaving and entering horizontally. */
export function edgePath(start, end) {
  const sameSide = start.side === end.side;
  const reach = sameSide ? LOOP_REACH + Math.abs(start.x - end.x) / 4 : Math.max(MIN_CURVE, Math.abs(end.x - start.x) / 2);
  const out = (point) => (point.side === "right" ? reach : -reach);
  return `M ${start.x} ${start.y} C ${start.x + out(start)} ${start.y}, ${end.x + out(end)} ${end.y}, ${end.x} ${end.y}`;
}

// ---- pan and zoom -------------------------------------------------------------------------------

export const MIN_SCALE = 0.08;
export const MAX_SCALE = 2.5;

/** Screen point to diagram point, for a view `{ x, y, scale }` (screen = diagram * scale + offset). */
export function toDiagram(view, point) {
  return { x: (point.x - view.x) / view.scale, y: (point.y - view.y) / view.scale };
}

/** Zooms by `factor` keeping the diagram point under `point` (screen) where it is. */
export function zoomAt(view, factor, point) {
  const scale = Math.min(MAX_SCALE, Math.max(MIN_SCALE, view.scale * factor));
  const ratio = scale / view.scale;
  return { scale, x: point.x - (point.x - view.x) * ratio, y: point.y - (point.y - view.y) * ratio };
}

export function panBy(view, dx, dy) {
  return { ...view, x: view.x + dx, y: view.y + dy };
}

export function boundsOf(rectangles) {
  if (rectangles.length === 0) return { x: 0, y: 0, width: 0, height: 0 };
  const left = Math.min(...rectangles.map((rectangle) => rectangle.x));
  const top = Math.min(...rectangles.map((rectangle) => rectangle.y));
  const right = Math.max(...rectangles.map((rectangle) => rectangle.x + rectangle.width));
  const bottom = Math.max(...rectangles.map((rectangle) => rectangle.y + rectangle.height));
  return { x: left, y: top, width: right - left, height: bottom - top };
}

/** The view that shows all of `bounds` centred in the viewport, never zoomed in past `maxScale`. */
export function fitView(bounds, viewport, { padding = 40, maxScale = 1 } = {}) {
  if (bounds.width <= 0 || bounds.height <= 0) return { x: 0, y: 0, scale: 1 };
  const available = { width: Math.max(1, viewport.width - 2 * padding), height: Math.max(1, viewport.height - 2 * padding) };
  const scale = Math.min(maxScale, MAX_SCALE, Math.max(MIN_SCALE, Math.min(available.width / bounds.width, available.height / bounds.height)));
  return { scale, x: (viewport.width - bounds.width * scale) / 2 - bounds.x * scale, y: (viewport.height - bounds.height * scale) / 2 - bounds.y * scale };
}
