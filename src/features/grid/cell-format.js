const numberFormat = new Intl.NumberFormat("en-US");

/** Binary and oversized values arrive as `{ kind, preview, size }`; they are shown, not edited. */
export function isSpecial(cell) {
  return cell !== null && typeof cell === "object";
}

export function describeSpecial(cell) {
  const size = `${numberFormat.format(cell.size)} bytes`;
  return cell.kind === "binary" ? `0x${cell.preview}… (${size})` : `${cell.preview}… (${size})`;
}

/** What the grid shows for a cell, with the class that styles it. */
export function displayCell(cell) {
  if (cell === null) return { text: "NULL", className: "muted" };
  if (isSpecial(cell)) return { text: describeSpecial(cell), className: "muted" };
  return { text: cell, className: "" };
}
