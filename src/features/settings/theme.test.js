import assert from "node:assert/strict";
import { test } from "node:test";

import { THEME_STORAGE_KEY, browserStorage, createThemeController, parsePreference, resolveTheme } from "./theme.js";

function setup({ stored = null, systemDark = false, storageFails = false } = {}) {
  const values = new Map(stored === null ? [] : [[THEME_STORAGE_KEY, stored]]);
  const storage = {
    getItem: (key) => {
      if (storageFails) throw new Error("blocked");
      return values.get(key) ?? null;
    },
    setItem: (key, value) => {
      if (storageFails) throw new Error("blocked");
      values.set(key, value);
    },
  };
  const listeners = [];
  const query = { matches: systemDark, addEventListener: (_type, listener) => listeners.push(listener) };
  const root = { dataset: {} };
  const controller = createThemeController({ root, storage, matchMedia: () => query });
  const changeSystem = (dark) => {
    query.matches = dark;
    listeners.forEach((listener) => listener());
  };
  return { controller, root, values, changeSystem };
}

test("unknown stored values fall back to system", () => {
  assert.equal(parsePreference("purple"), "system");
  assert.equal(parsePreference(null), "system");
  assert.equal(parsePreference("dark"), "dark");
});

test("system follows the operating system, explicit choices ignore it", () => {
  assert.equal(resolveTheme("system", true), "dark");
  assert.equal(resolveTheme("system", false), "light");
  assert.equal(resolveTheme("light", true), "light");
  assert.equal(resolveTheme("dark", false), "dark");
});

test("a stored preference is applied at start", () => {
  const { root, controller } = setup({ stored: "dark", systemDark: false });

  assert.equal(root.dataset.theme, "dark");
  assert.equal(controller.getPreference(), "dark");
});

test("choosing a theme applies it and remembers it", () => {
  const { root, controller, values } = setup({ systemDark: true });

  controller.setPreference("light");

  assert.equal(root.dataset.theme, "light");
  assert.equal(values.get(THEME_STORAGE_KEY), "light");
});

test("with system selected, an operating system change updates the theme live", () => {
  const { root, changeSystem } = setup({ systemDark: false });

  changeSystem(true);

  assert.equal(root.dataset.theme, "dark");
});

test("an explicit choice is not changed by the operating system", () => {
  const { root, controller, changeSystem } = setup({ systemDark: false });
  controller.setPreference("light");

  changeSystem(true);

  assert.equal(root.dataset.theme, "light");
});

test("unusable storage still lets the theme work for this run", () => {
  const { root, controller } = setup({ storageFails: true, systemDark: true });

  controller.setPreference("light");

  assert.equal(root.dataset.theme, "light");
  assert.equal(controller.getPreference(), "light");
});

test("blocked browser storage is replaced by an inert one", () => {
  const blocked = {
    get localStorage() {
      throw new Error("SecurityError");
    },
  };

  const storage = browserStorage(blocked);

  assert.equal(storage.getItem("anything"), null);
  assert.doesNotThrow(() => storage.setItem("a", "b"));
});
