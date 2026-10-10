export const FILTER_OPERATORS = Object.freeze([
  { value: "contains", label: "contains", needsValue: true },
  { value: "equals", label: "=", needsValue: true },
  { value: "notEquals", label: "≠", needsValue: true },
  { value: "less", label: "<", needsValue: true },
  { value: "lessOrEqual", label: "≤", needsValue: true },
  { value: "greater", label: ">", needsValue: true },
  { value: "greaterOrEqual", label: "≥", needsValue: true },
  { value: "like", label: "LIKE", needsValue: true },
  { value: "notLike", label: "NOT LIKE", needsValue: true },
  { value: "isNull", label: "is NULL", needsValue: false },
  { value: "isNotNull", label: "is not NULL", needsValue: false },
]);

const operatorOf = (value) => FILTER_OPERATORS.find((operator) => operator.value === value);

/** A filter ready for the backend, or null when a value is required but missing. */
export function makeFilter(column, operatorValue, text) {
  const operator = operatorOf(operatorValue);
  if (!operator || !column) return null;
  if (!operator.needsValue) return { column, operator: operator.value, value: null };
  return text === "" ? null : { column, operator: operator.value, value: text };
}

export function describeFilter(filter) {
  const operator = operatorOf(filter.operator);
  return operator.needsValue ? `${filter.column} ${operator.label} ${filter.value}` : `${filter.column} ${operator.label}`;
}

/** Clicking a heading cycles: ascending, descending, back to the table's own order. */
export function nextSort(current, column) {
  if (current === null || current.column !== column) return { column, descending: false };
  return current.descending ? null : { column, descending: true };
}
