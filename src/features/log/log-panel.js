import { h } from "../../lib/dom.js";
import { formatDuration, formatTime } from "./format.js";

function row(entry) {
  return h(
    "div",
    { class: `log-row log-${entry.level}` },
    h("span", { class: "log-time" }, formatTime(entry.time)),
    h("span", { class: "log-message" }, entry.message),
    h("span", { class: "log-duration" }, formatDuration(entry.durationMs)),
  );
}

/** Bottom panel listing every backend call with its duration and any error. */
export function createLogPanel({ onClear }) {
  const body = h("div", { class: "log-body", role: "log" });
  const count = h("span", { class: "muted" });
  const element = h(
    "section",
    { class: "log-panel" },
    h(
      "header",
      { class: "panel-header" },
      h("strong", {}, "Log"),
      count,
      h("span", { class: "spacer" }),
      h("button", { type: "button", class: "ghost", onClick: onClear }, "Clear"),
    ),
    body,
  );

  return {
    element,
    render(entries) {
      const wasAtBottom = body.scrollHeight - body.scrollTop - body.clientHeight < 4;
      body.replaceChildren(...entries.map(row));
      count.textContent = entries.length === 0 ? "empty" : `${entries.length}`;
      if (wasAtBottom) body.scrollTop = body.scrollHeight;
    },
  };
}
