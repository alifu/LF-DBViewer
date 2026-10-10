import { h } from "../../lib/dom.js";
import { visibleRange } from "./window.js";

export const ROW_HEIGHT = 24;
const OVERSCAN_ROWS = 12;
const ROW_NUMBER_WIDTH = 60;
const COLUMN_WIDTH = 180;
/** Ask for more rows when this close to the end of what is loaded. */
const NEAR_END_ROWS = 60;

/**
 * A table that only creates DOM for the rows on screen, so it stays fast with
 * hundreds of thousands of rows. It owns no data: `setModel` supplies
 * `{ columns: [{ name, detail, sort }], rowCount, cellAt(row, col) -> { text, className },
 *    rowClass(row), canEdit(row, col), editText(row, col) }`.
 */
export function createGrid({ onSort, onEdit, onSelect, onNearEnd }) {
  const header = h("div", { class: "grid-header" });
  const body = h("div", { class: "grid-body" });
  const element = h("div", { class: "grid", tabIndex: 0 }, header, body);
  let model = { columns: [], rowCount: 0 };
  let selected = null;
  let editor = null;
  let frame = 0;

  function drawHeader() {
    const template = [ROW_NUMBER_WIDTH, ...model.columns.map(() => COLUMN_WIDTH)].map((width) => `${width}px`).join(" ");
    const width = `${ROW_NUMBER_WIDTH + model.columns.length * COLUMN_WIDTH}px`;
    element.style.setProperty("--grid-columns", template);
    header.style.width = width;
    body.style.width = width;
    header.replaceChildren(
      h("div", { class: "grid-cell row-number" }, "#"),
      ...model.columns.map((column) =>
        h(
          "div",
          { class: "grid-cell grid-heading", title: `${column.name} ${column.detail}`, onClick: () => onSort(column.name) },
          h("strong", {}, column.name),
          h("span", { class: "muted" }, column.detail),
          column.sort && h("span", { class: "sort-mark" }, column.sort === "desc" ? "▼" : "▲"),
        ),
      ),
    );
  }

  function drawRow(row) {
    const cells = model.columns.map((_column, col) => {
      const { text, className = "" } = model.cellAt(row, col);
      const isSelected = selected !== null && selected.row === row && selected.col === col;
      return h("div", { class: `grid-cell ${className}${isSelected ? " selected" : ""}`, dataset: { row, col } }, text);
    });
    return h(
      "div",
      { class: `grid-row ${model.rowClass(row)}`, style: `top:${row * ROW_HEIGHT}px` },
      h("div", { class: "grid-cell row-number", dataset: { row, col: -1 } }, String(row + 1)),
      ...cells,
    );
  }

  function drawRows() {
    frame = 0;
    if (editor !== null) return;
    const { start, end } = visibleRange({
      scrollTop: element.scrollTop,
      viewportHeight: element.clientHeight,
      rowHeight: ROW_HEIGHT,
      rowCount: model.rowCount,
      overscan: OVERSCAN_ROWS,
    });
    const rows = [];
    for (let row = start; row < end; row += 1) rows.push(drawRow(row));
    body.replaceChildren(...rows);
    if (model.rowCount > 0 && end >= model.rowCount - NEAR_END_ROWS) onNearEnd();
  }

  function scheduleDraw() {
    if (frame === 0) frame = requestAnimationFrame(drawRows);
  }

  function cellPosition(target) {
    const cell = target.closest?.(".grid-cell[data-row]");
    return cell ? { cell, row: Number(cell.dataset.row), col: Number(cell.dataset.col) } : null;
  }

  function finishEdit(commit) {
    if (editor === null) return;
    const { row, col, input } = editor;
    editor = null;
    if (commit) onEdit(row, col, input.value);
    drawRows();
  }

  function startEdit({ cell, row, col }) {
    if (col < 0 || !model.canEdit(row, col)) return;
    const input = h("input", { class: "grid-editor", value: model.editText(row, col), spellcheck: false });
    editor = { row, col, input };
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") finishEdit(true);
      if (event.key === "Escape") finishEdit(false);
      event.stopPropagation();
    });
    input.addEventListener("blur", () => finishEdit(true));
    cell.replaceChildren(input);
    input.focus();
    input.select();
  }

  element.addEventListener("scroll", () => {
    finishEdit(true);
    scheduleDraw();
  });
  body.addEventListener("click", (event) => {
    const position = cellPosition(event.target);
    if (position === null || editor !== null) return;
    selected = { row: position.row, col: position.col };
    onSelect(selected);
    drawRows();
  });
  body.addEventListener("dblclick", (event) => {
    const position = cellPosition(event.target);
    if (position !== null && editor === null) startEdit(position);
  });
  new ResizeObserver(scheduleDraw).observe(element);

  return {
    element,
    /** `keepScroll: false` is for a fresh result (new sort or filter); appended pages keep their place. */
    setModel(next, { keepScroll = true } = {}) {
      editor = null;
      model = next;
      if (!keepScroll) {
        selected = null;
        element.scrollTop = 0;
      }
      if (selected !== null && selected.row >= model.rowCount) selected = null;
      body.style.height = `${model.rowCount * ROW_HEIGHT}px`;
      drawHeader();
      drawRows();
    },
    selection: () => selected,
    scrollToRow(row) {
      element.scrollTop = Math.max(0, row * ROW_HEIGHT - element.clientHeight / 2);
    },
  };
}
