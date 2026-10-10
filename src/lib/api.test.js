import assert from "node:assert/strict";
import { test } from "node:test";

import { AppError, createApi, toAppError } from "./api.js";

function setup(invoke) {
  const logged = [];
  const ticks = [0, 12];
  const call = createApi({ invoke, onLog: (entry) => logged.push(entry), now: () => ticks.shift() });
  return { call, logged };
}

test("returns the result and logs the label with its duration", async () => {
  const { call, logged } = setup(async () => "8.0.36");

  const result = await call("open_connection", { id: 1 }, { label: "Connect to Local" });

  assert.equal(result, "8.0.36");
  assert.deepEqual(logged, [{ level: "info", message: "Connect to Local", durationMs: 12 }]);
});

test("quiet calls log nothing on success", async () => {
  const { call, logged } = setup(async () => ({ connected: true }));

  await call("ping_connection", {}, { quiet: true });

  assert.deepEqual(logged, []);
});

test("failures are logged and rethrown as AppError, even when quiet", async () => {
  const { call, logged } = setup(async () => {
    throw { code: "database", message: "Access denied" };
  });

  await assert.rejects(call("open_connection", {}, { quiet: true }), (error) => {
    assert.ok(error instanceof AppError);
    assert.equal(error.code, "database");
    return true;
  });
  assert.equal(logged[0].level, "error");
  assert.equal(logged[0].message, "open_connection: Access denied");
});

test("non-object failures become an unknown AppError", () => {
  const error = toAppError("boom");

  assert.equal(error.code, "unknown");
  assert.equal(error.message, "boom");
});

test("a label function describes the result, and failures fall back to the command name", async () => {
  const ok = setup(async () => ({ sql: "SELECT 1" }));
  await ok.call("browse_table", {}, { label: (page) => page.sql });
  assert.equal(ok.logged[0].message, "SELECT 1");

  const failing = setup(async () => {
    throw { code: "database", message: "gone" };
  });
  await assert.rejects(failing.call("browse_table", {}, { label: (page) => page.sql }));
  assert.equal(failing.logged[0].message, "browse_table: gone");
});
