import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { contrastRatio } from "../lib/contrast.js";

const css = readFileSync(new URL("./themes.css", import.meta.url), "utf8");

function tokensOf(selector) {
  const start = css.indexOf(selector);
  assert.notEqual(start, -1, `${selector} not found in themes.css`);
  const body = css.slice(css.indexOf("{", start) + 1, css.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/(--[\w-]+):\s*([^;]+?);/g)].map(([, name, value]) => [name, value]));
}

const themes = {
  light: tokensOf(':root[data-theme="light"]'),
  dark: tokensOf(':root[data-theme="dark"]'),
};
const beforeScript = tokensOf(":root:not([data-theme])");

const TEXT = 4.5;
const GRAPHIC = 3;
// [foreground, background, minimum ratio]
const PAIRS = [
  ["--text", "--bg", TEXT],
  ["--text", "--surface", TEXT],
  ["--text", "--surface-2", TEXT],
  ["--muted", "--bg", TEXT],
  ["--muted", "--surface", TEXT],
  ["--muted", "--surface-2", TEXT],
  ["--on-accent", "--accent", TEXT],
  ["--on-accent", "--accent-hover", TEXT],
  ["--on-danger", "--danger-solid", TEXT],
  ["--ok", "--surface", TEXT],
  ["--danger", "--surface", TEXT],
  ["--danger", "--bg", TEXT],
  ["--text", "--accent-subtle", TEXT],
  ["--accent", "--surface", GRAPHIC],
  ["--focus", "--surface", GRAPHIC],
  ["--border", "--surface", 1.2],
  ["--syn-keyword", "--surface", TEXT],
  ["--syn-string", "--surface", TEXT],
  ["--syn-number", "--surface", TEXT],
  ["--syn-type", "--surface", TEXT],
  ["--syn-comment", "--surface", TEXT],
];

for (const [name, tokens] of Object.entries(themes)) {
  test(`${name} theme defines the same tokens as light`, () => {
    assert.deepEqual(Object.keys(tokens).sort(), Object.keys(themes.light).sort());
  });

  for (const [foreground, background, minimum] of PAIRS) {
    test(`${name}: ${foreground} on ${background} is at least ${minimum}:1`, () => {
      const ratio = contrastRatio(tokens[foreground], tokens[background]);

      assert.ok(ratio >= minimum, `${tokens[foreground]} on ${tokens[background]} is ${ratio.toFixed(2)}:1`);
    });
  }
}

test("the dark block used before scripts run matches the explicit dark theme", () => {
  assert.deepEqual(beforeScript, themes.dark);
});
