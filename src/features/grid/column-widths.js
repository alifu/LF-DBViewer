export const DEFAULT_COLUMN_WIDTH = 180;
export const MIN_COLUMN_WIDTH = 60;
export const MAX_COLUMN_WIDTH = 900;

export const clampWidth = (width) => Math.min(MAX_COLUMN_WIDTH, Math.max(MIN_COLUMN_WIDTH, Math.round(width)));

/** `widths` is `{ [columnName]: px }`; columns the user never resized have the default. */
export const widthOf = (widths, name) => widths[name] ?? DEFAULT_COLUMN_WIDTH;

/** The CSS `grid-template-columns` value for the row-number column followed by the data columns. */
export function gridTemplate(rowNumberWidth, names, widths) {
  return [rowNumberWidth, ...names.map((name) => widthOf(widths, name))].map((width) => `${width}px`).join(" ");
}

export function totalWidth(rowNumberWidth, names, widths) {
  return names.reduce((sum, name) => sum + widthOf(widths, name), rowNumberWidth);
}

/** A new widths object with one column resized (the old one is left alone). */
export function withWidth(widths, name, width) {
  return { ...widths, [name]: clampWidth(width) };
}
