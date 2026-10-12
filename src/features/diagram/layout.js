/**
 * Places the boxes. Tables that are connected go in columns by distance from the best-connected one, so
 * lines mostly run left to right and parents sit beside their children; lone tables fill a grid below.
 */

export const GAP_X = 90;
export const GAP_Y = 36;
const COMPONENT_GAP = 80;
const ROW_WIDTH_LIMIT = 2600;
const ORDERING_SWEEPS = 3;

function neighbours(model) {
  const adjacent = new Map(model.boxes.map((box) => [box.id, new Set()]));
  for (const edge of model.edges) {
    if (edge.from === edge.to) continue;
    adjacent.get(edge.from)?.add(edge.to);
    adjacent.get(edge.to)?.add(edge.from);
  }
  return adjacent;
}

function connectedGroups(model, adjacent) {
  const seen = new Set();
  const groups = [];
  for (const box of model.boxes) {
    if (seen.has(box.id)) continue;
    const group = [];
    const queue = [box.id];
    seen.add(box.id);
    while (queue.length > 0) {
      const id = queue.shift();
      group.push(id);
      for (const next of [...adjacent.get(id)].sort()) {
        if (!seen.has(next)) {
          seen.add(next);
          queue.push(next);
        }
      }
    }
    groups.push(group);
  }
  return groups;
}

/**
 * Columns by foreign key direction: a table sits one column to the right of every table it refers to, so lookup
 * tables end up on the left and the tables that use them to the right. Keys that would close a cycle are ignored
 * for this. Inside a column, tables are ordered by where their neighbours are, to keep lines short.
 */
function layersOf(group, model, adjacent) {
  const members = new Set(group);
  const parentsOf = new Map(group.map((id) => [id, new Set()]));
  const childrenOf = new Map(group.map((id) => [id, new Set()]));
  for (const edge of model.edges) {
    if (edge.from === edge.to || !members.has(edge.from) || !members.has(edge.to)) continue;
    parentsOf.get(edge.from).add(edge.to);
    childrenOf.get(edge.to).add(edge.from);
  }

  // Depth-first from the tables nothing refers to first, dropping the key that closes a cycle.
  const state = new Map();
  const dropped = new Set();
  const visit = (id) => {
    state.set(id, "open");
    for (const child of [...childrenOf.get(id)].sort()) {
      if (state.get(child) === "open") dropped.add(`${id}>${child}`);
      else if (!state.has(child)) visit(child);
    }
    state.set(id, "done");
  };
  const byParents = [...group].sort((a, b) => parentsOf.get(a).size - parentsOf.get(b).size || (a < b ? -1 : 1));
  for (const id of byParents) if (!state.has(id)) visit(id);

  const layerOf = new Map();
  const resolve = (id) => {
    if (layerOf.has(id)) return layerOf.get(id);
    layerOf.set(id, 0);
    const parents = [...parentsOf.get(id)].filter((parent) => !dropped.has(`${parent}>${id}`));
    const layer = parents.length === 0 ? 0 : Math.max(...parents.map((parent) => resolve(parent))) + 1;
    layerOf.set(id, layer);
    return layer;
  };
  const layers = [];
  for (const id of [...group].sort()) (layers[resolve(id)] ??= []).push(id);

  const fraction = new Map();
  const remember = (layer) => layer.forEach((id, position) => fraction.set(id, (position + 0.5) / layer.length));
  remember(layers[0]);
  for (let sweep = 0; sweep < ORDERING_SWEEPS; sweep += 1) {
    for (let index = 0; index < layers.length; index += 1) {
      const score = (id) => {
        const near = [...adjacent.get(id)].filter((other) => fraction.has(other) && layerOf.get(other) !== index).map((other) => fraction.get(other));
        return near.length === 0 ? fraction.get(id) ?? 0.5 : near.reduce((sum, value) => sum + value, 0) / near.length;
      };
      layers[index] = [...layers[index]].sort((a, b) => score(a) - score(b) || (a < b ? -1 : 1));
      remember(layers[index]);
    }
  }
  return layers;
}

function placeGroup(layers, byId) {
  const positions = new Map();
  const columnHeights = layers.map((layer) => layer.reduce((sum, id) => sum + byId.get(id).height, 0) + (layer.length - 1) * GAP_Y);
  const tallest = Math.max(...columnHeights);
  let x = 0;
  layers.forEach((layer, index) => {
    let y = (tallest - columnHeights[index]) / 2;
    for (const id of layer) {
      positions.set(id, { x, y });
      y += byId.get(id).height + GAP_Y;
    }
    x += Math.max(...layer.map((id) => byId.get(id).width)) + GAP_X;
  });
  return { positions, width: x - GAP_X, height: tallest };
}

/** Shelf packing: items left to right, a new shelf when the row is full. */
function packRows(items, startY) {
  const placed = new Map();
  let x = 0;
  let y = startY;
  let shelf = 0;
  for (const item of items) {
    if (x > 0 && x + item.width > ROW_WIDTH_LIMIT) {
      x = 0;
      y += shelf + COMPONENT_GAP;
      shelf = 0;
    }
    placed.set(item.id, { x, y });
    x += item.width + COMPONENT_GAP;
    shelf = Math.max(shelf, item.height);
  }
  return { placed, bottom: y + shelf };
}

/** `{ [boxId]: { x, y } }` for every box in the model. The same model always gives the same layout. */
export function autoLayout(model) {
  const adjacent = neighbours(model);
  const groups = connectedGroups(model, adjacent);
  const linked = groups.filter((group) => group.length > 1);
  const lone = groups.filter((group) => group.length === 1).map((group) => model.byId.get(group[0]));

  const blocks = linked.map((group, index) => ({ id: String(index), ...placeGroup(layersOf(group, model, adjacent), model.byId) }));
  const packed = packRows(blocks, 0);
  const positions = {};
  blocks.forEach((block) => {
    const origin = packed.placed.get(block.id);
    for (const [id, position] of block.positions) positions[id] = { x: origin.x + position.x, y: origin.y + position.y };
  });
  const grid = packRows(lone, linked.length === 0 ? 0 : packed.bottom + COMPONENT_GAP);
  for (const [id, position] of grid.placed) positions[id] = position;
  return positions;
}

const isPosition = (value) => value !== null && typeof value === "object" && Number.isFinite(value.x) && Number.isFinite(value.y);

/**
 * Saved positions win; boxes without one (new tables) go in a row under everything already placed.
 */
export function mergeLayout(automatic, saved, model) {
  const merged = {};
  const fresh = [];
  for (const box of model.boxes) {
    if (isPosition(saved[box.id])) merged[box.id] = { x: saved[box.id].x, y: saved[box.id].y };
    else fresh.push(box);
  }
  if (fresh.length === 0) return merged;
  if (Object.keys(merged).length === 0) return { ...automatic };
  const placedBoxes = model.boxes.filter((box) => merged[box.id]);
  const left = Math.min(...placedBoxes.map((box) => merged[box.id].x));
  const bottom = Math.max(...placedBoxes.map((box) => merged[box.id].y + box.height));
  const { placed } = packRows(fresh, bottom + COMPONENT_GAP);
  for (const [id, position] of placed) merged[id] = { x: left + position.x, y: position.y };
  return merged;
}

export function overlaps(a, b) {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}
