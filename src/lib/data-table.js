import { h } from "./dom.js";

/**
 * A plain table. `columns` are `{ label, cell(row) -> string | Node, className? }`.
 * It is for lists of tens or hundreds of rows; the data grid is for big result sets.
 */
export function dataTable({ columns, rows, empty = "Nothing to show." }) {
  const head = h("tr", {}, ...columns.map((column) => h("th", { class: column.className ?? "" }, column.label)));
  const body = rows.length === 0 ? [h("tr", {}, h("td", { colSpan: columns.length, class: "muted empty" }, empty))] : rows.map((row) => h("tr", { class: row.rowClass ?? "" }, ...columns.map((column) => h("td", { class: column.className ?? "" }, column.cell(row)))));
  return h("table", { class: "data-table" }, h("thead", {}, head), h("tbody", {}, ...body));
}

export function searchBox(placeholder, onInput) {
  return h("input", { type: "search", placeholder, "aria-label": placeholder, spellcheck: false, onInput: (event) => onInput(event.target.value) });
}
