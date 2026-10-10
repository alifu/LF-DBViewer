/** Pure state transitions for the whole app. Every function returns a new state. */

export const MAX_LOG_ENTRIES = 500;

export const VIEW = Object.freeze({ dashboard: "dashboard", workspace: "workspace" });

export const initialState = Object.freeze({
  view: VIEW.dashboard,
  profiles: [],
  connections: [],
  activeConnectionId: null,
  log: [],
});

const UNKNOWN_STATUS = Object.freeze({ connected: true, latencyMs: null, error: null });

export function setProfiles(state, profiles) {
  return { ...state, profiles };
}

export function showDashboard(state) {
  return { ...state, view: VIEW.dashboard };
}

export function addConnection(state, info) {
  const connection = { ...info, status: UNKNOWN_STATUS };
  return {
    ...state,
    view: VIEW.workspace,
    connections: [...state.connections, connection],
    activeConnectionId: info.id,
  };
}

export function activateConnection(state, id) {
  if (!state.connections.some((connection) => connection.id === id)) return state;
  return { ...state, view: VIEW.workspace, activeConnectionId: id };
}

/** Closing the active connection activates a neighbour, or returns to the dashboard. */
export function removeConnection(state, id) {
  const index = state.connections.findIndex((connection) => connection.id === id);
  if (index === -1) return state;
  const connections = state.connections.filter((connection) => connection.id !== id);
  const neighbour = connections[Math.min(index, connections.length - 1)] ?? null;
  const activeConnectionId = state.activeConnectionId === id ? (neighbour?.id ?? null) : state.activeConnectionId;
  return {
    ...state,
    connections,
    activeConnectionId,
    view: connections.length === 0 ? VIEW.dashboard : state.view,
  };
}

function mapConnection(state, id, change) {
  return {
    ...state,
    connections: state.connections.map((connection) => (connection.id === id ? change(connection) : connection)),
  };
}

export function setConnectionStatus(state, id, status) {
  return mapConnection(state, id, (connection) => ({ ...connection, status }));
}

/** After a reconnect the server may report a different version; keep the old status slot. */
export function replaceConnectionInfo(state, info) {
  return mapConnection(state, info.id, (connection) => ({ ...connection, ...info }));
}

export function activeConnection(state) {
  return state.connections.find((connection) => connection.id === state.activeConnectionId) ?? null;
}

export function appendLog(state, entry) {
  return { ...state, log: [...state.log, entry].slice(-MAX_LOG_ENTRIES) };
}

export function clearLog(state) {
  return { ...state, log: [] };
}
