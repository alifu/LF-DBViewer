import assert from "node:assert/strict";
import { test } from "node:test";

import {
  MAX_LOG_ENTRIES,
  VIEW,
  activateConnection,
  activeConnection,
  addConnection,
  appendLog,
  initialState,
  removeConnection,
  replaceConnectionInfo,
  setConnectionStatus,
} from "./app-state.js";

const withTwo = () => addConnection(addConnection(initialState, { id: "a", name: "A" }), { id: "b", name: "B" });

test("adding a connection opens the workspace and makes it active", () => {
  const state = addConnection(initialState, { id: "a", name: "A" });

  assert.equal(state.view, VIEW.workspace);
  assert.equal(activeConnection(state).name, "A");
  assert.equal(initialState.connections.length, 0);
});

test("closing the active connection activates its neighbour", () => {
  const state = removeConnection(withTwo(), "b");

  assert.equal(state.activeConnectionId, "a");
  assert.equal(state.view, VIEW.workspace);
});

test("closing an inactive connection keeps the active one", () => {
  const state = removeConnection(withTwo(), "a");

  assert.equal(state.activeConnectionId, "b");
});

test("closing the last connection returns to the dashboard", () => {
  const state = removeConnection(addConnection(initialState, { id: "a" }), "a");

  assert.equal(state.view, VIEW.dashboard);
  assert.equal(state.activeConnectionId, null);
});

test("unknown ids leave the state untouched", () => {
  const state = withTwo();

  assert.equal(removeConnection(state, "zzz"), state);
  assert.equal(activateConnection(state, "zzz"), state);
});

test("status and info updates touch only the matching connection", () => {
  const down = { connected: false, latencyMs: null, error: "gone" };

  const state = replaceConnectionInfo(setConnectionStatus(withTwo(), "a", down), { id: "a", serverVersion: "9.3.0" });

  assert.deepEqual(state.connections[0].status, down);
  assert.equal(state.connections[0].serverVersion, "9.3.0");
  assert.equal(state.connections[1].status.connected, true);
});

test("the log keeps only the most recent entries", () => {
  let state = initialState;
  for (let index = 0; index < MAX_LOG_ENTRIES + 5; index += 1) {
    state = appendLog(state, { message: String(index) });
  }

  assert.equal(state.log.length, MAX_LOG_ENTRIES);
  assert.equal(state.log[0].message, "5");
});
