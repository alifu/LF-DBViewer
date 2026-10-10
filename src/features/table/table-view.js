import { confirmDialog, h } from "../../lib/dom.js";
import { createGrid } from "../grid/grid.js";
import { FILTER_OPERATORS, describeFilter, makeFilter, nextSort } from "./filters.js";
import { NO_CHANGES, addInsert, changeCount, editCell, editInsert, isSpecial, removeInsert, toChanges, toggleDeleted } from "./pending.js";

const PAGE_ROWS = 500;
/** Rows kept in memory per tab. Past this, narrow the result with a filter. */
const MAX_LOADED_ROWS = 200_000;
const numberFormat = new Intl.NumberFormat("en-US");

function describeSpecial(cell) {
  const size = `${numberFormat.format(cell.size)} bytes`;
  return cell.kind === "binary" ? `0x${cell.preview}… (${size})` : `${cell.preview}… (${size})`;
}

/**
 * One table or view: filter bar, virtualized grid, and pending edits.
 * `data.browse(page)` and `data.apply(changes)` talk to the backend.
 */
export function createTableView({ tab, data }) {
  const editable = tab.kind === "table";
  let columns = [];
  let primaryKey = [];
  let rows = [];
  let hasMore = false;
  let keyset = false;
  let loading = false;
  let sort = null;
  let filters = [];
  let pending = NO_CHANGES;
  let confirmedKeyless = false;

  const grid = createGrid({ onSort: changeSort, onEdit: edit, onSelect: () => drawToolbar(), onNearEnd: () => void loadMore() });
  const filterColumn = h("select", { "aria-label": "Filter column" });
  const filterOperator = h(
    "select",
    { "aria-label": "Filter condition" },
    FILTER_OPERATORS.map((operator) => h("option", { value: operator.value }, operator.label)),
  );
  const filterValue = h("input", { placeholder: "value", "aria-label": "Filter value", spellcheck: false });
  const chips = h("div", { class: "chips" });
  const status = h("span", { class: "muted table-status" });
  const message = h("span", { class: "error-text table-message" });
  const addButton = h("button", { type: "button", onClick: addRow }, "Add row");
  const deleteButton = h("button", { type: "button", onClick: deleteRow }, "Delete row");
  const nullButton = h("button", { type: "button", onClick: setNull }, "Set NULL");
  const applyButton = h("button", { type: "button", class: "primary", onClick: () => void apply() }, "Apply");
  const discardButton = h("button", { type: "button", onClick: discard }, "Discard");
  const filterForm = h(
    "form",
    { class: "filter-form", onSubmit: (event) => (event.preventDefault(), addFilter()) },
    filterColumn,
    filterOperator,
    filterValue,
    h("button", { type: "submit" }, "Filter"),
  );
  const element = h(
    "div",
    { class: "table-view" },
    h("div", { class: "table-toolbar" }, filterForm, chips, h("span", { class: "spacer" }), addButton, deleteButton, nullButton, discardButton, applyButton),
    grid.element,
    h("div", { class: "table-footer" }, status, message),
  );

  const columnIndex = (name) => columns.findIndex((column) => column.name === name);
  const isInsert = (row) => row >= rows.length;

  function cellAt(row, col) {
    const name = columns[col].name;
    if (isInsert(row)) {
      const values = pending.inserts[row - rows.length];
      if (!(name in values)) return { text: "(default)", className: "muted" };
      return values[name] === null ? { text: "NULL", className: "muted edited" } : { text: values[name], className: "edited" };
    }
    const edits = pending.edits[row];
    const isEdited = edits !== undefined && name in edits;
    const cell = isEdited ? edits[name] : rows[row][col];
    const className = isEdited ? "edited" : "";
    if (cell === null) return { text: "NULL", className: `muted ${className}` };
    if (isSpecial(cell)) return { text: describeSpecial(cell), className: "muted" };
    return { text: cell, className };
  }

  function canEdit(row, col) {
    if (!editable || columns[col].generated) return false;
    if (isInsert(row)) return true;
    return !pending.deleted[row] && !isSpecial(rows[row][col]);
  }

  function editText(row, col) {
    const { text, className = "" } = cellAt(row, col);
    return className.includes("muted") ? "" : text;
  }

  function rowClass(row) {
    if (isInsert(row)) return "inserted";
    return pending.deleted[row] ? "deleted" : "";
  }

  function draw({ keepScroll = true } = {}) {
    grid.setModel(
      {
        columns: columns.map((column) => ({
          name: column.name,
          detail: column.columnType,
          sort: sort?.column === column.name ? (sort.descending ? "desc" : "asc") : null,
        })),
        rowCount: rows.length + pending.inserts.length,
        cellAt,
        canEdit,
        editText,
        rowClass,
      },
      { keepScroll },
    );
    drawToolbar();
  }

  function drawToolbar() {
    const count = changeCount(pending);
    const selection = grid.selection();
    applyButton.textContent = count > 0 ? `Apply ${count} change${count === 1 ? "" : "s"}` : "Apply";
    applyButton.disabled = discardButton.disabled = count === 0 || loading;
    addButton.hidden = deleteButton.hidden = nullButton.hidden = applyButton.hidden = discardButton.hidden = !editable;
    deleteButton.disabled = selection === null;
    nullButton.disabled = selection === null || selection.col < 0 || !canEdit(selection.row, selection.col) || !(isInsert(selection.row) || columns[selection.col].nullable);
    chips.replaceChildren(
      ...filters.map((filter, index) =>
        h("button", { type: "button", class: "chip", title: "Remove this filter", onClick: () => removeFilter(index) }, `${describeFilter(filter)} ×`),
      ),
    );
    const more = rows.length >= MAX_LOADED_ROWS ? " · limit reached, add a filter to see other rows" : hasMore ? " · scroll for more" : "";
    const keyNote = editable && columns.length > 0 && primaryKey.length === 0 ? " · no primary key" : "";
    status.textContent = loading ? "Loading…" : `${numberFormat.format(rows.length)} row${rows.length === 1 ? "" : "s"} loaded${more}${keyNote}${editable ? "" : " · read-only"}`;
  }

  function nextPageCursor() {
    if (rows.length === 0) return {};
    const lastKey = primaryKey.map((name) => rows[rows.length - 1][columnIndex(name)]);
    // The key cursor needs plain values; otherwise fall back to counting rows.
    return keyset && lastKey.every((cell) => typeof cell === "string") ? { after: lastKey } : { offset: rows.length };
  }

  async function load({ reset }) {
    if (loading) return;
    loading = true;
    message.textContent = "";
    drawToolbar();
    try {
      const page = await data.browse({ limit: PAGE_ROWS, sort: sort ? [sort] : [], filters, ...(reset ? {} : nextPageCursor()) });
      rows = reset ? page.rows : [...rows, ...page.rows];
      ({ columns, primaryKey, hasMore, keyset } = page);
      if (reset) {
        pending = NO_CHANGES;
        const chosen = filterColumn.value;
        filterColumn.replaceChildren(...columns.map((column) => h("option", { value: column.name, selected: column.name === chosen }, column.name)));
      }
    } catch (error) {
      message.textContent = error.message;
    } finally {
      loading = false;
      draw({ keepScroll: !reset });
    }
  }

  async function loadMore() {
    if (hasMore && !loading && rows.length < MAX_LOADED_ROWS) await load({ reset: false });
  }

  /** Anything that refetches from the top would lose unsaved edits, so ask first. */
  async function reloadFromTop() {
    if (changeCount(pending) > 0) {
      const confirmed = await confirmDialog({
        title: "Discard unsaved changes?",
        body: `${tab.name} has changes that were not applied. Reloading will discard them.`,
        confirmLabel: "Discard and reload",
        danger: true,
      });
      if (!confirmed) return false;
    }
    await load({ reset: true });
    return true;
  }

  function changeSort(column) {
    const previous = sort;
    sort = nextSort(sort, column);
    void reloadFromTop().then((reloaded) => {
      if (!reloaded) sort = previous;
    });
  }

  function changeFilters(next) {
    const previous = filters;
    filters = next;
    void reloadFromTop().then((reloaded) => {
      if (!reloaded) filters = previous;
      drawToolbar();
    });
  }

  function addFilter() {
    const filter = makeFilter(filterColumn.value, filterOperator.value, filterValue.value);
    if (filter === null) {
      message.textContent = "Enter a value to filter by.";
      return;
    }
    filterValue.value = "";
    changeFilters([...filters, filter]);
  }

  function removeFilter(index) {
    changeFilters(filters.filter((_filter, position) => position !== index));
  }

  function setValue(row, col, value) {
    const name = columns[col].name;
    pending = isInsert(row) ? editInsert(pending, row - rows.length, name, value) : editCell(pending, row, name, value, rows[row][col]);
    draw();
  }

  function edit(row, col, text) {
    // Leaving the editor without typing must not turn NULL or "(default)" into an empty string.
    const isPlaceholder = cellAt(row, col).className?.includes("muted") ?? false;
    if (isPlaceholder && text === "") return;
    setValue(row, col, text);
  }

  function setNull() {
    const selection = grid.selection();
    if (selection !== null) setValue(selection.row, selection.col, null);
  }

  function addRow() {
    pending = addInsert(pending);
    draw();
    grid.scrollToRow(rows.length + pending.inserts.length - 1);
  }

  function deleteRow() {
    const selection = grid.selection();
    if (selection === null) return;
    pending = isInsert(selection.row) ? removeInsert(pending, selection.row - rows.length) : toggleDeleted(pending, selection.row);
    draw();
  }

  function discard() {
    pending = NO_CHANGES;
    draw();
  }

  async function confirmKeyless() {
    if (primaryKey.length > 0 || confirmedKeyless) return true;
    confirmedKeyless = await confirmDialog({
      title: "This table has no primary key",
      body: "Rows will be matched by all of their values. If two rows are identical, only one of them is changed. Continue?",
      confirmLabel: "Continue",
    });
    return confirmedKeyless;
  }

  async function apply() {
    message.textContent = "";
    let changes;
    try {
      changes = toChanges(pending, { rows, columns, primaryKey });
    } catch (error) {
      message.textContent = error.message;
      return;
    }
    const touchesExistingRows = changes.some((change) => change.kind !== "insert");
    if (touchesExistingRows && !(await confirmKeyless())) return;
    try {
      await data.apply(changes);
    } catch (error) {
      message.textContent = error.message;
      return;
    }
    await load({ reset: true });
  }

  void load({ reset: true });

  return { element, reload: reloadFromTop, hasUnsavedChanges: () => changeCount(pending) > 0 };
}
