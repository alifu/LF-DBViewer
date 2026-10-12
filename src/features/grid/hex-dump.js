const BYTES_PER_LINE = 16;
const PRINTABLE_FROM = 0x20;
const PRINTABLE_TO = 0x7e;

/** Classic hex dump of a hex string: offset, 16 bytes in hex, and the printable characters. */
export function hexDump(hex) {
  const bytes = hex.match(/../g) ?? [];
  const lines = [];
  for (let offset = 0; offset < bytes.length; offset += BYTES_PER_LINE) {
    const chunk = bytes.slice(offset, offset + BYTES_PER_LINE);
    const cells = chunk.join(" ").padEnd(BYTES_PER_LINE * 3 - 1, " ");
    const text = chunk
      .map((byte) => {
        const code = Number.parseInt(byte, 16);
        return code >= PRINTABLE_FROM && code <= PRINTABLE_TO ? String.fromCharCode(code) : ".";
      })
      .join("");
    lines.push(`${offset.toString(16).padStart(8, "0")}  ${cells}  ${text}`);
  }
  return lines.join("\n");
}
