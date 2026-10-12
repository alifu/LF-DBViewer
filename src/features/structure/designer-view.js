import { h } from "../../lib/dom.js";
import { charsetOfCollation } from "./collation-options.js";
import {
  COMMON_TYPES, INDEX_KINDS, REFERENTIAL_ACTIONS, ROW_FORMATS, addCheck, addColumn, addForeignKey, addIndex, emptyTable, formatIndexColumns,
  formatNameList, hasChanges, isTextType, moveColumn, parseIndexColumns, parseNameList, primaryKeyColumns, removeCheck, removeColumn, removeForeignKey,
  removeIndex, renameColumn, setDefault, setPrimaryKey, uniqueName, updateCheck, updateColumn, updateForeignKey, updateIndex, updateOptions,
} from "./designer-state.js";

const DEFAULT_KINDS = Object.freeze([
  { value: "none", label: "No default" },
  { value: "null", label: "NULL" },
  { value: "value", label: "Value" },
  { value: "expression", label: "Expression" },
]);
const TYPES_LIST_ID = "lf-column-types";

const text = (value, handlers = {}) => h("input", { value, spellcheck: false, autocomplete: "off", ...handlers });
const check = (checked, onChange, label) => h("input", { type: "checkbox", checked, onChange: (event) => onChange(event.target.checked), "aria-label": label });
const option = (value, label, selected) => h("option", { value, selected }, label);
const iconButton = (label, title, onClick) => h("button", { type: "button", class: "ghost icon", title, "aria-label": title, onClick }, label);

/**
 * Designer for one table: columns, indexes, foreign keys and options. It edits a copy of the
 * table; "Review & apply" sends the original and the copy to the backend, which works out the SQL.
 * `data`: `load()` (null for a new table), `serverOptions()`, `tableNames()`, `review(change)`
 * (preview, confirm, apply; resolves true when applied), `onCreated(name)` and `onRenamed(name)`.
 */
export function createDesignerView({ tab, data }) {
  const isNew = tab.kind === "newtable";
  let original = null;
  let table = emptyTable("new_table");
  let initialNew = table;
  let serverOptions = { collations: [], engines: [] };
  let tableNames = [];
  let convertColumns = false;
  const expanded = new Set();

  const message = h("span", { class: "designer-message", role: "status" });
  const nameInput = text(table.name, { "aria-label": "Table name", onInput: () => (table = { ...table, name: nameInput.value }) });
  const columnsBox = h("div", { class: "designer-box" });
  const indexesBox = h("div", { class: "designer-box" });
  const keysBox = h("div", { class: "designer-box" });
  const checksBox = h("div", { class: "designer-box" });
  const optionsBox = h("div", { class: "designer-box" });
  const applyButton = h("button", { type: "button", class: "primary", onClick: () => void submit() }, "Review & apply");
  const revertButton = h("button", { type: "button", onClick: () => void load() }, "Revert");
  const element = h(
    "div",
    { class: "designer" },
    h("datalist", { id: TYPES_LIST_ID }, ...COMMON_TYPES.map((type) => h("option", { value: type }))),
    h("div", { class: "designer-header" }, h("label", { class: "field inline" }, h("span", {}, "Table"), nameInput), h("span", { class: "spacer" }), message, revertButton, applyButton),
    h("div", { class: "designer-body" }, section("Columns", columnsBox), section("Indexes", indexesBox), section("Foreign keys", keysBox), section("Checks", checksBox), section("Options", optionsBox)),
  );

  function section(title, box) {
    return h("section", { class: "designer-section" }, h("h3", {}, title), box);
  }

  function say(textContent, kind = "") {
    message.textContent = textContent;
    message.className = `designer-message ${kind}`;
  }

  // ---- columns ------------------------------------------------------------

  const setColumns = (next) => {
    table = next;
    renderColumns();
    renderIndexes();
    renderKeys();
  };

  function columnRow(column, position) {
    const isPrimary = primaryKeyColumns(table).includes(column.name);
    const isOpen = expanded.has(column.name);
    const kind = column.default.kind;
    const defaultInput =
      kind === "value" || kind === "expression"
        ? text(column.default.value ?? "", { "aria-label": `Default of ${column.name}`, onInput: (event) => (table = updateColumn(table, position, setDefault(column, kind, event.target.value))) })
        : null;
    const main = h(
      "tr",
      { class: column.originalName === null ? "new-row" : "" },
      h("td", { class: "cell-actions" }, iconButton("▲", "Move up", () => setColumns(moveColumn(table, position, -1))), iconButton("▼", "Move down", () => setColumns(moveColumn(table, position, 1)))),
      h(
        "td",
        {},
        text(column.name, {
          "aria-label": "Column name",
          onInput: (event) => {
            const previous = table.columns[position].name;
            table = renameColumn(table, position, event.target.value);
            if (expanded.delete(previous)) expanded.add(event.target.value);
            renderIndexes();
            renderKeys();
          },
        }),
      ),
      h(
        "td",
        {},
        text(column.columnType, {
          list: TYPES_LIST_ID,
          "aria-label": `Type of ${column.name}`,
          onInput: (event) => (table = updateColumn(table, position, { columnType: event.target.value })),
          onChange: (event) => {
            if (isTextType(event.target.value) !== isTextType(column.columnType)) renderColumns();
          },
        }),
      ),
      h("td", { class: "cell-check" }, check(column.nullable, (value) => ((table = updateColumn(table, position, { nullable: value })), renderColumns()), `${column.name} allows NULL`)),
      h(
        "td",
        { class: "cell-default" },
        h(
          "select",
          { "aria-label": `Default kind of ${column.name}`, onChange: (event) => ((table = updateColumn(table, position, setDefault(column, event.target.value))), renderColumns()) },
          ...DEFAULT_KINDS.map((entry) => option(entry.value, entry.label, entry.value === kind)),
        ),
        defaultInput,
      ),
      h("td", { class: "cell-check" }, check(column.autoIncrement, (value) => ((table = updateColumn(table, position, { autoIncrement: value })), renderColumns()), `${column.name} is auto increment`)),
      h("td", { class: "cell-check" }, check(isPrimary, (value) => setColumns(setPrimaryKey(table, column.name, value)), `${column.name} is in the primary key`)),
      h(
        "td",
        { class: "cell-actions" },
        iconButton(isOpen ? "▾" : "▸", "More options", () => ((isOpen ? expanded.delete(column.name) : expanded.add(column.name)), renderColumns())),
        iconButton("✕", "Remove column", () => setColumns(removeColumn(table, position))),
      ),
    );
    return isOpen ? [main, detailRow(column, position)] : [main];
  }

  function detailRow(column, position) {
    const collations = serverOptions.collations;
    const collationSelect = isTextType(column.columnType)
      ? h(
          "select",
          { "aria-label": `Collation of ${column.name}`, onChange: (event) => (table = updateColumn(table, position, { collation: event.target.value || null })) },
          option("", "(table default)", column.collation === null),
          ...collations.map((entry) => option(entry.name, entry.name, entry.name === column.collation)),
        )
      : null;
    const generated = column.generated;
    return h(
      "tr",
      { class: "detail-row" },
      h(
        "td",
        { colSpan: 8 },
        h(
          "div",
          { class: "detail-grid" },
          collationSelect && h("label", { class: "field" }, h("span", {}, "Collation"), collationSelect),
          h("label", { class: "field" }, h("span", {}, "On update"), text(column.onUpdate ?? "", { placeholder: "CURRENT_TIMESTAMP", onInput: (event) => (table = updateColumn(table, position, { onUpdate: event.target.value || null })) })),
          h("label", { class: "field wide" }, h("span", {}, "Generated as (expression, leave empty for a normal column)"), text(generated?.expression ?? "", {
            placeholder: "price * quantity",
            onInput: (event) => (table = updateColumn(table, position, { generated: event.target.value === "" ? null : { expression: event.target.value, stored: table.columns[position].generated?.stored ?? false } })),
          })),
          h("label", { class: "check" }, check(generated?.stored ?? false, (value) => (table = updateColumn(table, position, { generated: { expression: table.columns[position].generated?.expression ?? "", stored: value } })), "Stored"), "Stored"),
          h("label", { class: "check" }, check(column.invisible, (value) => (table = updateColumn(table, position, { invisible: value })), "Invisible"), "Invisible (left out of SELECT * and plain INSERTs)"),
          h("label", { class: "field wide" }, h("span", {}, "Comment"), text(column.comment, { onInput: (event) => (table = updateColumn(table, position, { comment: event.target.value })) })),
        ),
      ),
    );
  }

  function renderColumns() {
    const head = h("tr", {}, ...["", "Name", "Type", "Null", "Default", "A_I", "PK", ""].map((label) => h("th", {}, label)));
    columnsBox.replaceChildren(
      h("table", { class: "designer-table" }, h("thead", {}, head), h("tbody", {}, ...table.columns.flatMap(columnRow))),
      h("button", { type: "button", onClick: () => setColumns(addColumn(table)) }, "Add column"),
    );
  }

  // ---- indexes and foreign keys ----------------------------------------------

  function indexRow(index, position) {
    if (index.kind === "primary") {
      return h("tr", {}, h("td", {}, "PRIMARY"), h("td", { class: "muted" }, "Primary key"), h("td", { class: "muted", colSpan: 2 }, `${formatIndexColumns(index.columns)} (set with the PK boxes above)`));
    }
    return h(
      "tr",
      { class: index.originalName === null ? "new-row" : "" },
      h("td", {}, text(index.name, { "aria-label": "Index name", onInput: (event) => (table = updateIndex(table, position, { name: event.target.value })) })),
      h("td", {}, h("select", { "aria-label": "Index type", onChange: (event) => (table = updateIndex(table, position, { kind: event.target.value })) }, ...INDEX_KINDS.map((entry) => option(entry.value, entry.label, entry.value === index.kind)))),
      h("td", {}, text(formatIndexColumns(index.columns), { "aria-label": "Index columns", placeholder: "column, other(20), third DESC", onInput: (event) => (table = updateIndex(table, position, { columns: parseIndexColumns(event.target.value) })) })),
      h("td", { class: "cell-actions" }, iconButton("✕", "Remove index", () => ((table = removeIndex(table, position)), renderIndexes()))),
    );
  }

  function renderIndexes() {
    indexesBox.replaceChildren(
      h("table", { class: "designer-table" }, h("thead", {}, h("tr", {}, ...["Name", "Type", "Columns", ""].map((label) => h("th", {}, label)))), h("tbody", {}, ...table.indexes.map(indexRow))),
      h("button", { type: "button", onClick: () => ((table = addIndex(table)), renderIndexes()) }, "Add index"),
    );
  }

  function keyRow(key, position) {
    const actions = (field, value) => h("select", { "aria-label": field, onChange: (event) => (table = updateForeignKey(table, position, { [field === "On delete" ? "onDelete" : "onUpdate"]: event.target.value })) }, ...REFERENTIAL_ACTIONS.map((entry) => option(entry.value, entry.label, entry.value === value)));
    return h(
      "tr",
      { class: key.originalName === null ? "new-row" : "" },
      h("td", {}, text(key.name, { "aria-label": "Foreign key name", onInput: (event) => (table = updateForeignKey(table, position, { name: event.target.value })) })),
      h("td", {}, text(formatNameList(key.columns), { "aria-label": "Foreign key columns", onInput: (event) => (table = updateForeignKey(table, position, { columns: parseNameList(event.target.value) })) })),
      h("td", {}, text(key.refDatabase ?? "", { "aria-label": "Referenced database", placeholder: "(this database)", onInput: (event) => (table = updateForeignKey(table, position, { refDatabase: event.target.value.trim() || null })) })),
      h("td", {}, text(key.refTable, { list: "lf-table-names", "aria-label": "Referenced table", onInput: (event) => (table = updateForeignKey(table, position, { refTable: event.target.value })) })),
      h("td", {}, text(formatNameList(key.refColumns), { "aria-label": "Referenced columns", onInput: (event) => (table = updateForeignKey(table, position, { refColumns: parseNameList(event.target.value) })) })),
      h("td", {}, actions("On delete", key.onDelete)),
      h("td", {}, actions("On update", key.onUpdate)),
      h("td", { class: "cell-actions" }, iconButton("✕", "Remove foreign key", () => ((table = removeForeignKey(table, position)), renderKeys()))),
    );
  }

  function renderKeys() {
    keysBox.replaceChildren(
      h("datalist", { id: "lf-table-names" }, ...tableNames.map((name) => h("option", { value: name }))),
      h("table", { class: "designer-table" }, h("thead", {}, h("tr", {}, ...["Name", "Columns", "Database", "Table", "Referenced", "On delete", "On update", ""].map((label) => h("th", {}, label)))), h("tbody", {}, ...table.foreignKeys.map(keyRow))),
      h("button", { type: "button", onClick: () => ((table = addForeignKey(table)), renderKeys()) }, "Add foreign key"),
    );
  }

  function checkRow(constraint, position) {
    return h(
      "tr",
      { class: constraint.originalName === null ? "new-row" : "" },
      h("td", {}, text(constraint.name, { "aria-label": "Check name", placeholder: "(automatic)", onInput: (event) => (table = updateCheck(table, position, { name: event.target.value })) })),
      h("td", { class: "wide" }, text(constraint.expression, { "aria-label": "Check condition", placeholder: "price >= 0", onInput: (event) => (table = updateCheck(table, position, { expression: event.target.value })) })),
      h("td", { class: "cell-actions" }, iconButton("✕", "Remove check", () => ((table = removeCheck(table, position)), renderChecks()))),
    );
  }

  function renderChecks() {
    checksBox.replaceChildren(
      h("table", { class: "designer-table" }, h("thead", {}, h("tr", {}, ...["Name", "Condition", ""].map((label) => h("th", {}, label)))), h("tbody", {}, ...(table.checks ?? []).map(checkRow))),
      h("button", { type: "button", onClick: () => ((table = addCheck(table)), renderChecks()) }, "Add check"),
    );
  }

  // ---- options --------------------------------------------------------------------

  function collationSelect() {
    const byCharset = Map.groupBy(serverOptions.collations, (entry) => entry.charset);
    return h(
      "select",
      { "aria-label": "Table collation", onChange: (event) => ((table = updateOptions(table, { collation: event.target.value || null })), renderOptions()) },
      option("", "(server default)", table.options.collation === null),
      ...[...byCharset].map(([charset, entries]) => h("optgroup", { label: charset }, ...entries.map((entry) => option(entry.name, entry.name, entry.name === table.options.collation)))),
    );
  }

  function renderOptions() {
    const { options } = table;
    const collationChanged = original !== null && options.collation !== original.options.collation && options.collation !== null;
    optionsBox.replaceChildren(
      h(
        "div",
        { class: "detail-grid" },
        h("label", { class: "field" }, h("span", {}, "Engine"), h("select", { "aria-label": "Engine", onChange: (event) => (table = updateOptions(table, { engine: event.target.value || null })) }, option("", "(server default)", options.engine === null), ...serverOptions.engines.map((engine) => option(engine, engine, engine.toLowerCase() === (options.engine ?? "").toLowerCase())))),
        h("label", { class: "field" }, h("span", {}, "Row format"), h("select", { "aria-label": "Row format", onChange: (event) => (table = updateOptions(table, { rowFormat: event.target.value || null })) }, option("", "(default)", options.rowFormat === null), ...ROW_FORMATS.map((format) => option(format, format, format.toLowerCase() === (options.rowFormat ?? "").toLowerCase())))),
        h("label", { class: "field" }, h("span", {}, "Collation"), collationSelect()),
        !isNew && h("label", { class: "field" }, h("span", {}, "Next AUTO_INCREMENT"), text(options.autoIncrement ?? "", { inputMode: "numeric", onInput: (event) => (table = updateOptions(table, { autoIncrement: /^\d+$/.test(event.target.value) ? Number(event.target.value) : null })) })),
        h("label", { class: "field wide" }, h("span", {}, "Comment"), text(options.comment, { onInput: (event) => (table = updateOptions(table, { comment: event.target.value })) })),
        collationChanged &&
          h("label", { class: "check wide" }, check(convertColumns, (value) => (convertColumns = value), "Convert columns"), `Also convert the existing text columns to ${charsetOfCollation(options.collation)} (rewrites the table)`),
      ),
    );
  }

  function renderAll() {
    nameInput.value = table.name;
    renderColumns();
    renderIndexes();
    renderKeys();
    renderChecks();
    renderOptions();
  }

  // ---- loading and applying ---------------------------------------------------------

  async function load() {
    say("Loading…");
    try {
      [serverOptions, tableNames] = await Promise.all([data.serverOptions(), data.tableNames()]);
      original = isNew ? null : await data.load();
      table = isNew ? emptyTable(uniqueName(tableNames, "new_table")) : structuredClone(original);
      initialNew = table;
      convertColumns = false;
      expanded.clear();
      say("");
      renderAll();
    } catch (error) {
      say(error.message, "error");
    }
  }

  async function submit() {
    say("");
    const change = isNew
      ? { kind: "createTable", table }
      : { kind: "alterTable", original, edited: table, convertColumns };
    applyButton.disabled = true;
    try {
      if (!(await data.review(change))) return;
      if (isNew) data.onCreated(table.name);
      else if (table.name !== original.name) data.onRenamed(table.name);
      else await load();
    } catch (error) {
      say(error.message, "error");
    } finally {
      applyButton.disabled = false;
    }
  }

  void load();

  return {
    element,
    reload: () => (hasChanges(isNew ? initialNew : original, table) ? Promise.resolve(false) : load().then(() => true)),
    hasUnsavedChanges: () => hasChanges(isNew ? initialNew : original, table),
  };
}
