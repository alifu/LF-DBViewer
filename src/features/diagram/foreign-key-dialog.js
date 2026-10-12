import { field, formDialog, input } from "../admin/admin-dialogs.js";
import { h } from "../../lib/dom.js";
import { REFERENTIAL_ACTIONS } from "../structure/designer-state.js";

/** Name the key and choose its actions; `submit(values)` resolves true once the change was applied. */
export function openForeignKeyDialog({ child, parent, defaultName, submit }) {
  const name = input({ value: defaultName, "aria-label": "Constraint name" });
  const action = (label, value) => h("select", { "aria-label": label }, ...REFERENTIAL_ACTIONS.map((entry) => h("option", { value: entry.value, selected: entry.value === value }, entry.label)));
  const onDelete = action("On delete", "restrict");
  const onUpdate = action("On update", "restrict");
  const where = (side) => `${side.database ? `${side.database}.` : ""}${side.table}.${side.column}`;
  const dialog = formDialog({
    title: "New foreign key",
    fields: [
      h("p", {}, h("strong", {}, where(child)), " refers to ", h("strong", {}, where(parent))),
      h("p", { class: "muted" }, "The referenced column must be indexed (a primary or unique key) and have the same type."),
      field("Constraint name", name),
      field("When the parent row is deleted", onDelete),
      field("When the parent key changes", onUpdate),
    ],
    submitLabel: "Review SQL…",
    submit: () => submit({ name: name.value.trim(), onDelete: onDelete.value, onUpdate: onUpdate.value }),
  });
  name.focus();
  name.select();
  return dialog;
}
