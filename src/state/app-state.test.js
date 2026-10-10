import assert from "node:assert/strict";
import { test } from "node:test";

import {
  MAX_LOG_ENTRIES,
  VIEW,
  activateConnection,
  activeConnection,
  addConnection,
  activateTab,
  appendLog,
  closeTab,
  initialState,
  openTab,
  removeConnection,
  replaceConnectionInfo,
  schemaOf,
  setConnectionStatus,
  setDatabases,
  setObjects,
  tabsOf,
  toggleExpanded,
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

const table = (name, connectionId = "a") => ({ connectionId, database: "shop", name, kind: "table" });

test("schema data is kept per connection and expansion toggles", () => {
  let state = setDatabases(withTwo(), "a", [{ name: "shop" }]);
  state = setObjects(state, "a", "shop", { tables: [{ name: "orders" }] });
  state = toggleExpanded(state, "a", "shop");

  assert.equal(schemaOf(state, "a").objects.shop.tables[0].name, "orders");
  assert.equal(schemaOf(state, "a").expanded.shop, true);
  assert.deepEqual(schemaOf(state, "b").databases, []);
  assert.equal(schemaOf(toggleExpanded(state, "a", "shop"), "a").expanded.shop, undefined);
});

test("reloading databases forgets loaded objects but keeps expansion", () => {
  let state = setObjects(setDatabases(withTwo(), "a", [{ name: "shop" }]), "a", "shop", { tables: [] });
  state = toggleExpanded(state, "a", "shop");

  state = setDatabases(state, "a", [{ name: "shop" }, { name: "crm" }]);

  assert.deepEqual(schemaOf(state, "a").objects, {});
  assert.equal(schemaOf(state, "a").expanded.shop, true);
});

test("opening the same object twice selects the existing tab", () => {
  let state = openTab(openTab(withTwo(), table("orders")), table("items"));
  state = openTab(state, table("orders"));

  assert.equal(tabsOf(state, "a").length, 2);
  assert.equal(state.activeTabIds.a, tabsOf(state, "a")[0].id);
});

test("a table and a view with the same name are different tabs", () => {
  const state = openTab(openTab(withTwo(), table("orders")), { ...table("orders"), kind: "view" });

  assert.equal(state.tabs.length, 2);
});

test("closing the selected tab selects its neighbour on the same connection", () => {
  let state = openTab(openTab(openTab(withTwo(), table("orders")), table("items")), table("other", "b"));
  const [orders, items] = tabsOf(state, "a");

  state = closeTab(state, items.id);

  assert.equal(state.activeTabIds.a, orders.id);
  assert.equal(tabsOf(state, "b").length, 1);
  assert.equal(closeTab(state, orders.id).activeTabIds.a, null);
});

test("selecting a tab only affects its own connection", () => {
  let state = openTab(openTab(withTwo(), table("orders")), table("items"));
  const [orders] = tabsOf(state, "a");

  state = activateTab(state, orders.id);

  assert.equal(state.activeTabIds.a, orders.id);
  assert.equal(activateTab(state, "missing"), state);
});

test("closing a connection drops its tabs and schema", () => {
  let state = openTab(openTab(withTwo(), table("orders")), table("other", "b"));
  state = setDatabases(state, "a", [{ name: "shop" }]);

  state = removeConnection(state, "a");

  assert.deepEqual(state.tabs.map((tab) => tab.name), ["other"]);
  assert.equal(state.schemas.a, undefined);
  assert.equal(state.activeTabIds.a, undefined);
});
