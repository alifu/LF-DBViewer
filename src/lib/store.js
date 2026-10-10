/**
 * Minimal observable store. State is never mutated: `update` takes a function
 * from the current state to the next one, and subscribers get both.
 */
export function createStore(initialState) {
  let state = initialState;
  const listeners = new Set();

  return {
    getState: () => state,
    update(reducer) {
      const next = reducer(state);
      if (next === state) return;
      const previous = state;
      state = next;
      for (const listener of listeners) listener(state, previous);
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}
