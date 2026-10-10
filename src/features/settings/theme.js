export const THEME_PREFERENCES = Object.freeze(["system", "light", "dark"]);
export const DEFAULT_PREFERENCE = "system";
export const THEME_STORAGE_KEY = "lf-dbviewer.theme";
const DARK_QUERY = "(prefers-color-scheme: dark)";

export function parsePreference(value) {
  return THEME_PREFERENCES.includes(value) ? value : DEFAULT_PREFERENCE;
}

export function resolveTheme(preference, systemPrefersDark) {
  if (preference === "system") return systemPrefersDark ? "dark" : "light";
  return preference;
}

/** `window.localStorage` itself can throw when site data is blocked. */
export function browserStorage(win) {
  try {
    return win.localStorage;
  } catch {
    return { getItem: () => null, setItem: () => {} };
  }
}

/** Storage can be missing or throw (private windows, cleared data); the theme must still work. */
function readStored(storage) {
  try {
    return parsePreference(storage.getItem(THEME_STORAGE_KEY));
  } catch {
    return DEFAULT_PREFERENCE;
  }
}

function writeStored(storage, preference) {
  try {
    storage.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // The choice still applies for this run.
  }
}

/**
 * Keeps `data-theme` on the root element in step with the user's preference and,
 * for "system", with the operating system's appearance.
 */
export function createThemeController({ root, storage, matchMedia }) {
  const darkQuery = matchMedia(DARK_QUERY);
  let preference = readStored(storage);

  function apply() {
    root.dataset.theme = resolveTheme(preference, darkQuery.matches);
  }

  darkQuery.addEventListener("change", apply);
  apply();

  return {
    getPreference: () => preference,
    getResolved: () => root.dataset.theme,
    setPreference(next) {
      preference = parsePreference(next);
      writeStored(storage, preference);
      apply();
    },
  };
}
