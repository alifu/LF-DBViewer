/** An error returned by a backend command: `code` is stable, `message` is for people. */
export class AppError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "AppError";
    this.code = code;
  }
}

export function toAppError(raw) {
  if (raw instanceof AppError) return raw;
  if (raw && typeof raw === "object" && typeof raw.message === "string") {
    return new AppError(raw.code ?? "unknown", raw.message);
  }
  return new AppError("unknown", String(raw));
}

/**
 * Wraps the backend so every call is timed and reported to the log panel.
 * `quiet` calls (such as the periodic ping) only log when they fail. `label` may be a
 * function of the result, for calls whose log line is the SQL the backend ran.
 */
export function createApi({ invoke, onLog, now = () => performance.now() }) {
  return async function call(command, args = {}, { label = command, quiet = false } = {}) {
    const started = now();
    try {
      const result = await invoke(command, args);
      const message = typeof label === "function" ? label(result) : label;
      if (!quiet) onLog({ level: "info", message, durationMs: now() - started });
      return result;
    } catch (raw) {
      const error = toAppError(raw);
      const prefix = typeof label === "function" ? command : label;
      onLog({ level: "error", message: `${prefix}: ${error.message}`, durationMs: now() - started });
      throw error;
    }
  };
}
