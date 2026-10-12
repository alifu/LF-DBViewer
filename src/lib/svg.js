const NAMESPACE = "http://www.w3.org/2000/svg";

/** Creates an SVG element. Attributes with null, undefined or false are skipped; `on…` keys become listeners. */
export function svg(tag, attributes = {}, ...children) {
  const element = document.createElementNS(NAMESPACE, tag);
  for (const [key, value] of Object.entries(attributes)) {
    if (value === null || value === undefined || value === false) continue;
    if (key.startsWith("on")) element.addEventListener(key.slice(2).toLowerCase(), value);
    else element.setAttribute(key, value === true ? "" : String(value));
  }
  element.append(...children.flat().filter((child) => child !== null && child !== undefined && child !== false));
  return element;
}
