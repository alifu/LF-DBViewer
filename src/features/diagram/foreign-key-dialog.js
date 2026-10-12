import { field, formDialog, input } from "../admin/admin-dialogs.js";
import { h } from "../../lib/dom.js";
import { REFERENTIAL_ACTIONS } from "../structure/designer-state.js";

/**
 * Name the key and choose its actions; `submit(values)` resolves true once the change was applied. The dragged pair is
 * the first column pair; "Add column pair" extends it for a composite key. `childColumns` and `parentColumns` are the
 * names offered for the extra pairs.
 */
export function openForeignKeyDialog({ child, parent, defaultName, childColumns = [], parentColumns = [], submit }) {
  const name = input({ value: defaultName, "aria-label": "Constraint name" });
  const action = (label, value) => h("select", { "aria-label": label }, ...REFERENTIAL_ACTIONS.map((entry) => h("option", { value: entry.value, selected: entry.value === value }, entry.label)));
  const onDelete = action("On delete", "restrict");
  const onUpdate = action("On update", "restrict");
  const where = (side) => `${side.database ? `${side.database}.` : ""}${side.table}.${side.column}`;
  const extraPairs = h("div", { class: "fk-pairs" });
  const picker = (label, names) => h("select", { "aria-label": label }, ...names.map((column) => h("option", { value: column }, column)));
  const addPair = () => {
    const pair = h("div", { class: "fk-pair" }, picker("Column of the child table", childColumns), h("span", { class: "muted" }, "refers to"), picker("Column of the parent table", parentColumns), h("button", { type: "button", class: "ghost", onClick: () => pair.remove() }, "Remove"));
    extraPairs.append(pair);
  };
  const pairValues = () => [...extraPairs.children].map((pair) => pair.querySelectorAll("select"));
  const dialog = formDialog({
    title: "New foreign key",
    fields: [
      h("p", {}, h("strong", {}, where(child)), " refers to ", h("strong", {}, where(parent))),
      h("p", { class: "muted" }, "The referenced column must be indexed (a primary or unique key) and have the same type."),
      extraPairs,
      childColumns.length > 0 && parentColumns.length > 0 && h("button", { type: "button", class: "ghost", onClick: addPair }, "Add column pair"),
      field("Constraint name", name),
      field("When the parent row is deleted", onDelete),
      field("When the parent key changes", onUpdate),
    ],
    submitLabel: "Review SQL…",
    submit: () => {
      const extra = pairValues();
      return submit({
        name: name.value.trim(),
        columns: [child.column, ...extra.map(([own]) => own.value)],
        refColumns: [parent.column, ...extra.map(([, other]) => other.value)],
        onDelete: onDelete.value,
        onUpdate: onUpdate.value,
      });
    },
  });
  name.focus();
  name.select();
  return dialog;
}
