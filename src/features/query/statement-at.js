/**
 * Which statements a Run button should execute. `spans` come from the backend splitter:
 * `{ text, start, end }` with UTF-16 offsets, the same units the editor reports.
 */

/**
 * The statement under the cursor. A cursor just after a statement's `;` still belongs to
 * it; in the gap between two statements the previous one is used, and before the first
 * statement the first one.
 */
export function statementAt(spans, cursor) {
  if (spans.length === 0) return null;
  const containing = spans.find((span) => cursor >= span.start && cursor <= span.end + 1);
  if (containing) return containing;
  const before = spans.filter((span) => span.end < cursor);
  return before.length > 0 ? before[before.length - 1] : spans[0];
}

/** `current` runs one statement, `all` runs every statement in order. */
export function chooseToRun(mode, spans, cursor) {
  if (mode === "all") return spans;
  const found = statementAt(spans, cursor);
  return found === null ? [] : [found];
}

const EXPLAINABLE = /^\s*(select|insert|update|delete|replace|with|table)\b/i;

/** `EXPLAIN` in front of each statement that can be explained; others are left as they are. */
export function explainTexts(spans) {
  return spans.map((span) => (EXPLAINABLE.test(span.text) ? `EXPLAIN ${span.text}` : span.text));
}
