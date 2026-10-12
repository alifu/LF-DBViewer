import { h } from "../../lib/dom.js";
import { svg } from "../../lib/svg.js";
import { showMenu } from "../structure/menu.js";
import {
  BOX_PADDING, HEADER_HEIGHT, MAX_NAME_CHARS, ROW_HEIGHT, anchor, boundsOf, chooseSides, edgePath, ellipsize, fitView, panBy, toDiagram, zoomAt,
} from "./geometry.js";
import { openForeignKeyDialog } from "./foreign-key-dialog.js";
import { autoLayout, mergeLayout } from "./layout.js";
import { buildModel, describeEdge, edgesOf } from "./model.js";

const ZOOM_STEP = 1.25;
const WHEEL_ZOOM_SPEED = 0.01;
const SAVE_DELAY_MS = 400;
const DRAG_THRESHOLD = 3;
const EXPORT_PADDING = 30;
const PNG_SCALE = 2;
const TAG_WIDTH = 26;
const STYLE_PROPERTIES = ["fill", "stroke", "stroke-width", "stroke-dasharray", "opacity", "font-family", "font-size", "font-weight", "font-style", "text-anchor"];

/**
 * The relationship diagram of one database: tables as boxes, foreign keys as lines. Boxes drag, the canvas pans and zooms,
 * and dragging from a column's dot to another column proposes a new foreign key (reviewed as SQL before it runs).
 * `data`: `load()`, `loadLayout()`, `saveLayout(layout)`, `openTable(name)`, `openStructure(name)`, `addForeignKey(…)`,
 * `dropForeignKey(…)`, `suggestName(table, columns)` and `saveImage(kind, bytes)`.
 */
export function createDiagramView({ tab, data }) {
  let diagram = null;
  let model = null;
  let positions = {};
  let view = { x: 0, y: 0, scale: 1 };
  let keyColumnsOnly = false;
  let selection = null;
  let needsFit = false;
  let saveTimer = null;
  let drag = null;
  let destroyed = false;
  const boxNodes = new Map();
  const edgeNodes = new Map();

  const edgeLayer = svg("g", { class: "diagram-edges" });
  const boxLayer = svg("g", { class: "diagram-boxes" });
  const draft = svg("path", { class: "diagram-draft", d: "" });
  const viewport = svg("g", { class: "diagram-viewport" }, edgeLayer, boxLayer, draft);
  const canvas = svg(
    "svg",
    { class: "diagram-svg", role: "img", "aria-label": `Relationship diagram of ${tab.database}` },
    svg("defs", {}, svg("marker", { id: "diagram-arrow", viewBox: "0 0 10 10", refX: 9, refY: 5, markerWidth: 9, markerHeight: 9, orient: "auto" }, svg("path", { class: "diagram-arrow", d: "M 0 1 L 10 5 L 0 9 z" })), svg("marker", { id: "diagram-dot", viewBox: "0 0 10 10", refX: 5, refY: 5, markerWidth: 7, markerHeight: 7, orient: "auto" }, svg("circle", { class: "diagram-start", cx: 5, cy: 5, r: 3.5 }))),
    viewport,
  );

  const status = h("span", { class: "muted" });
  const message = h("span", { class: "error-text", role: "alert" });
  const info = h("div", { class: "diagram-info" });
  const search = h("input", { type: "search", list: "diagram-tables", placeholder: "Find a table", "aria-label": "Find a table", spellcheck: false, onChange: () => find(search.value) });
  const tableList = h("datalist", { id: "diagram-tables" });
  const keysOnly = h("input", { type: "checkbox", onChange: (event) => ((keyColumnsOnly = event.target.checked), rebuild({ keepPositions: true }), persist()) });
  const toolbar = h(
    "div",
    { class: "diagram-toolbar" },
    h("button", { type: "button", onClick: () => relayout() }, "Auto layout"),
    h("button", { type: "button", onClick: () => fit() }, "Fit"),
    h("button", { type: "button", "aria-label": "Zoom out", onClick: () => zoomBy(1 / ZOOM_STEP) }, "−"),
    h("button", { type: "button", "aria-label": "Zoom in", onClick: () => zoomBy(ZOOM_STEP) }, "+"),
    h("label", { class: "check" }, keysOnly, "Key columns only"),
    search,
    tableList,
    h("span", { class: "spacer" }),
    status,
    message,
    h("button", { type: "button", onClick: () => void reload() }, "Reload"),
    h("button", { type: "button", onClick: () => void exportImage("svg") }, "Export SVG"),
    h("button", { type: "button", onClick: () => void exportImage("png") }, "Export PNG"),
  );
  const element = h("div", { class: "diagram" }, toolbar, h("div", { class: "diagram-stage" }, canvas), info);

  // ---- drawing -------------------------------------------------------------------------------------

  const applyView = () => viewport.setAttribute("transform", `translate(${view.x} ${view.y}) scale(${view.scale})`);

  function rowNode(box, row, index) {
    const children = [
      svg("rect", { class: "diagram-row-bg", width: box.width, height: ROW_HEIGHT }),
      row.primaryKey || row.foreignKey ? svg("text", { class: row.primaryKey ? "diagram-tag pk" : "diagram-tag fk", x: BOX_PADDING, y: ROW_HEIGHT / 2 + 4 }, row.primaryKey ? "PK" : "FK") : null,
      svg("text", { class: row.nullable || row.primaryKey ? "diagram-column" : "diagram-column required", x: BOX_PADDING + TAG_WIDTH, y: ROW_HEIGHT / 2 + 4 }, ellipsize(row.name, MAX_NAME_CHARS)),
      row.type ? svg("text", { class: "diagram-type", x: box.width - BOX_PADDING, y: ROW_HEIGHT / 2 + 4, "text-anchor": "end" }, ellipsize(row.type, 24)) : null,
      box.external ? null : svg("circle", { class: "diagram-port", cx: box.width, cy: ROW_HEIGHT / 2, r: 5 }),
    ];
    return svg("g", { class: "diagram-row", transform: `translate(0 ${HEADER_HEIGHT + index * ROW_HEIGHT})`, "data-box": box.id, "data-column": row.name }, ...children);
  }

  function boxNode(box) {
    const hidden = box.totalColumns - box.rows.length;
    const group = svg(
      "g",
      { class: box.external ? "diagram-box external" : "diagram-box", "data-box": box.id, transform: `translate(${positions[box.id].x} ${positions[box.id].y})` },
      svg("rect", { class: "diagram-body", width: box.width, height: box.height, rx: 6 }),
      svg("rect", { class: "diagram-header", width: box.width, height: HEADER_HEIGHT, rx: 6, "data-box": box.id }),
      svg("text", { class: "diagram-title", x: BOX_PADDING, y: HEADER_HEIGHT / 2 + 5 }, ellipsize(box.title, MAX_NAME_CHARS)),
      hidden > 0 ? svg("text", { class: "diagram-type", x: box.width - BOX_PADDING, y: HEADER_HEIGHT / 2 + 4, "text-anchor": "end" }, `+${hidden}`) : null,
      ...box.rows.map((row, index) => rowNode(box, row, index)),
    );
    return group;
  }

  function edgePathFor(edge) {
    const from = model.byId.get(edge.from);
    const to = model.byId.get(edge.to);
    const [fromSide, toSide] = chooseSides(from, positions[edge.from], to, positions[edge.to]);
    const start = anchor(from, positions[edge.from], edge.fromRow >= 0 ? edge.fromRow : null, fromSide);
    const end = anchor(to, positions[edge.to], edge.toRow >= 0 ? edge.toRow : null, toSide);
    return edgePath(start, end);
  }

  function edgeNode(edge) {
    const path = edgePathFor(edge);
    const line = svg("path", { class: "diagram-edge", d: path, "marker-start": "url(#diagram-dot)", "marker-end": "url(#diagram-arrow)" });
    const hit = svg("path", { class: "diagram-edge-hit", d: path });
    const group = svg("g", { class: "diagram-edge-group", "data-edge": edge.id }, svg("title", {}, `${edge.name}: ${describeEdge(edge)}`), hit, line);
    edgeNodes.set(edge.id, { group, line, hit });
    return group;
  }

  function refreshEdges(boxId) {
    for (const edge of edgesOf(model, boxId)) {
      const path = edgePathFor(edge);
      const nodes = edgeNodes.get(edge.id);
      nodes.line.setAttribute("d", path);
      nodes.hit.setAttribute("d", path);
    }
  }

  function render() {
    boxNodes.clear();
    edgeNodes.clear();
    edgeLayer.replaceChildren(...model.edges.map(edgeNode));
    boxLayer.replaceChildren(
      ...model.boxes.map((box) => {
        const node = boxNode(box);
        boxNodes.set(box.id, node);
        return node;
      }),
    );
    tableList.replaceChildren(...model.boxes.filter((box) => !box.external).map((box) => h("option", { value: box.id })));
    status.textContent = `${model.boxes.filter((box) => !box.external).length} tables · ${model.edges.length} relations${diagram.truncated ? " · only the first tables are shown" : ""}`;
    applySelection();
    applyView();
  }

  function applySelection() {
    const touched = selection === null ? null : selection.kind === "box" ? new Set(edgesOf(model, selection.id).map((edge) => edge.id)) : new Set([selection.id]);
    const related = new Set();
    if (selection?.kind === "box") for (const edge of edgesOf(model, selection.id)) related.add(edge.from).add(edge.to);
    if (selection?.kind === "edge") model.edges.filter((edge) => edge.id === selection.id).forEach((edge) => related.add(edge.from).add(edge.to));
    canvas.classList.toggle("has-selection", selection !== null);
    for (const [id, node] of boxNodes) {
      node.classList.toggle("selected", selection?.kind === "box" && selection.id === id);
      node.classList.toggle("related", related.has(id));
    }
    for (const [id, nodes] of edgeNodes) nodes.group.classList.toggle("related", touched?.has(id) ?? false);
    drawInfo();
  }

  function drawInfo() {
    if (selection === null) {
      info.replaceChildren(h("span", { class: "muted" }, "Drag tables to arrange them. Drag from the dot at the end of a column onto another column to create a foreign key. Scroll to pan, ⌘-scroll or pinch to zoom."));
      return;
    }
    if (selection.kind === "box") {
      const box = model.byId.get(selection.id);
      info.replaceChildren(
        h("strong", {}, box.title),
        box.estimatedRows !== null && box.estimatedRows !== undefined ? h("span", { class: "muted" }, `about ${box.estimatedRows.toLocaleString("en-US")} rows`) : null,
        box.external ? h("span", { class: "muted" }, "in another database") : h("button", { type: "button", onClick: () => data.openTable(box.id) }, "Open data"),
        box.external ? null : h("button", { type: "button", onClick: () => data.openStructure(box.id) }, "Structure"),
      );
      return;
    }
    const edge = model.edges.find((candidate) => candidate.id === selection.id);
    info.replaceChildren(
      h("strong", {}, edge.name),
      h("span", {}, describeEdge(edge)),
      h("span", { class: "muted" }, `on delete ${edge.onDelete}, on update ${edge.onUpdate}`),
      h("button", { type: "button", class: "danger", onClick: () => void dropKey(edge) }, "Drop foreign key…"),
    );
  }

  function select(next) {
    selection = next;
    applySelection();
  }

  // ---- layout and view ---------------------------------------------------------------------------------

  const stageSize = () => ({ width: canvas.clientWidth, height: canvas.clientHeight });

  function currentBounds() {
    return boundsOf(model.boxes.map((box) => ({ ...positions[box.id], width: box.width, height: box.height })));
  }

  function fit() {
    if (model === null) return;
    const size = stageSize();
    if (size.width === 0) return void (needsFit = true);
    view = fitView(currentBounds(), size);
    applyView();
    persist();
  }

  function zoomBy(factor) {
    const size = stageSize();
    view = zoomAt(view, factor, { x: size.width / 2, y: size.height / 2 });
    applyView();
    persist();
  }

  function relayout() {
    positions = autoLayout(model);
    render();
    fit();
  }

  function persist() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => data.saveLayout({ positions, view, keyColumnsOnly }), SAVE_DELAY_MS);
  }

  function find(name) {
    const box = model?.byId.get(name);
    if (!box) return;
    const size = stageSize();
    const centre = { x: positions[box.id].x + box.width / 2, y: positions[box.id].y + box.height / 2 };
    view = { ...view, x: size.width / 2 - centre.x * view.scale, y: size.height / 2 - centre.y * view.scale };
    applyView();
    select({ kind: "box", id: box.id });
    persist();
  }

  function rebuild({ keepPositions }) {
    model = buildModel(diagram, { keyColumnsOnly });
    positions = keepPositions ? mergeLayout(autoLayout(model), positions, model) : positions;
    render();
  }

  async function load({ firstTime }) {
    message.textContent = "";
    try {
      diagram = await data.load();
      const saved = data.loadLayout();
      if (firstTime) {
        keyColumnsOnly = saved.keyColumnsOnly;
        keysOnly.checked = keyColumnsOnly;
      }
      model = buildModel(diagram, { keyColumnsOnly });
      positions = mergeLayout(autoLayout(model), firstTime ? saved.positions : positions, model);
      selection = selection !== null && (model.byId.has(selection.id) || model.edges.some((edge) => edge.id === selection.id)) ? selection : null;
      render();
      if (firstTime) {
        if (saved.view) view = saved.view;
        else needsFit = true;
        applyView();
        if (needsFit) fit();
      }
    } catch (error) {
      message.textContent = error.message;
    }
  }

  const reload = () => load({ firstTime: false });

  // ---- pointer handling ------------------------------------------------------------------------------------

  function localPoint(event) {
    const rectangle = canvas.getBoundingClientRect();
    return { x: event.clientX - rectangle.left, y: event.clientY - rectangle.top };
  }

  function startDraft(event, row) {
    const boxId = row.dataset.box;
    const start = anchor(model.byId.get(boxId), positions[boxId], model.byId.get(boxId).rows.findIndex((entry) => entry.name === row.dataset.column), "right");
    drag = { type: "draft", boxId, column: row.dataset.column, start };
    event.preventDefault();
  }

  function onPointerDown(event) {
    if (event.button !== 0) return;
    const target = event.target;
    const port = target.closest?.(".diagram-port");
    if (port) return startDraft(event, port.closest(".diagram-row"));
    const edgeGroup = target.closest?.("[data-edge]");
    if (edgeGroup) return select({ kind: "edge", id: edgeGroup.dataset.edge });
    const header = target.closest?.(".diagram-header");
    const owner = target.closest?.("[data-box]");
    if (header && owner) {
      const boxId = owner.dataset.box;
      drag = { type: "box", boxId, origin: { ...positions[boxId] }, pointer: toDiagram(view, localPoint(event)), moved: false, screen: { x: event.clientX, y: event.clientY } };
      return event.preventDefault();
    }
    if (owner) return select({ kind: "box", id: owner.dataset.box });
    drag = { type: "pan", last: { x: event.clientX, y: event.clientY }, moved: false };
    event.preventDefault();
  }

  function onPointerMove(event) {
    if (drag === null) return;
    if (drag.type === "pan") {
      view = panBy(view, event.clientX - drag.last.x, event.clientY - drag.last.y);
      drag.last = { x: event.clientX, y: event.clientY };
      drag.moved = true;
      applyView();
    } else if (drag.type === "box") {
      if (!drag.moved && Math.hypot(event.clientX - drag.screen.x, event.clientY - drag.screen.y) < DRAG_THRESHOLD) return;
      drag.moved = true;
      const now = toDiagram(view, localPoint(event));
      positions = { ...positions, [drag.boxId]: { x: drag.origin.x + now.x - drag.pointer.x, y: drag.origin.y + now.y - drag.pointer.y } };
      boxNodes.get(drag.boxId).setAttribute("transform", `translate(${positions[drag.boxId].x} ${positions[drag.boxId].y})`);
      refreshEdges(drag.boxId);
    } else {
      const point = toDiagram(view, localPoint(event));
      draft.setAttribute("d", `M ${drag.start.x} ${drag.start.y} L ${point.x} ${point.y}`);
      markDropTarget(document.elementFromPoint(event.clientX, event.clientY)?.closest?.(".diagram-row") ?? null);
    }
  }

  let dropTarget = null;
  function markDropTarget(row) {
    if (dropTarget === row) return;
    dropTarget?.classList.remove("drop-target");
    dropTarget = row;
    dropTarget?.classList.add("drop-target");
  }

  function onPointerUp(event) {
    if (drag === null) return;
    const finished = drag;
    drag = null;
    if (finished.type === "box") {
      if (finished.moved) persist();
      else select({ kind: "box", id: finished.boxId });
    } else if (finished.type === "pan") {
      if (finished.moved) persist();
      else select(null);
    } else {
      draft.setAttribute("d", "");
      const row = document.elementFromPoint(event.clientX, event.clientY)?.closest?.(".diagram-row") ?? null;
      markDropTarget(null);
      if (row !== null && canvas.contains(row)) void proposeKey(finished, row);
    }
  }

  function onWheel(event) {
    event.preventDefault();
    if (event.ctrlKey || event.metaKey) view = zoomAt(view, Math.exp(-event.deltaY * WHEEL_ZOOM_SPEED), localPoint(event));
    else view = panBy(view, -event.deltaX, -event.deltaY);
    applyView();
    persist();
  }

  function onContextMenu(event) {
    const edgeGroup = event.target.closest?.("[data-edge]");
    const owner = event.target.closest?.("[data-box]");
    if (edgeGroup) {
      const edge = model.edges.find((candidate) => candidate.id === edgeGroup.dataset.edge);
      select({ kind: "edge", id: edge.id });
      return showMenu(event, [{ label: "Drop foreign key…", danger: true, onSelect: () => void dropKey(edge) }]);
    }
    if (!owner) return;
    const box = model.byId.get(owner.dataset.box);
    select({ kind: "box", id: box.id });
    if (box.external) return event.preventDefault();
    showMenu(event, [{ label: "Open data", onSelect: () => data.openTable(box.id) }, { label: "Structure…", onSelect: () => data.openStructure(box.id) }]);
  }

  function onDoubleClick(event) {
    const header = event.target.closest?.(".diagram-header");
    const owner = header && event.target.closest?.("[data-box]");
    if (owner && !model.byId.get(owner.dataset.box).external) data.openTable(owner.dataset.box);
  }

  canvas.addEventListener("pointerdown", onPointerDown);
  canvas.addEventListener("wheel", onWheel, { passive: false });
  canvas.addEventListener("contextmenu", onContextMenu);
  canvas.addEventListener("dblclick", onDoubleClick);
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  const resizer = new ResizeObserver(() => {
    if (needsFit && model !== null && canvas.clientWidth > 0) {
      needsFit = false;
      fit();
    }
  });
  resizer.observe(canvas);

  // ---- changes to the database -----------------------------------------------------------------------------------

  async function proposeKey(from, row) {
    const parentBox = model.byId.get(row.dataset.box);
    const child = { table: from.boxId, column: from.column };
    const parent = { table: parentBox.external ? parentBox.title.split(".").slice(1).join(".") : parentBox.id, column: row.dataset.column, database: parentBox.external ? parentBox.title.split(".")[0] : null };
    message.textContent = "";
    try {
      const defaultName = await data.suggestName(child.table, [child.column]);
      openForeignKeyDialog({
        child,
        parent,
        defaultName,
        submit: async ({ name, onDelete, onUpdate }) => {
          const applied = await data.addForeignKey({ table: child.table, columns: [child.column], refDatabase: parent.database, refTable: parent.table, refColumns: [parent.column], name, onUpdate, onDelete });
          if (applied) await reload();
          return applied;
        },
      });
    } catch (error) {
      message.textContent = error.message;
    }
  }

  async function dropKey(edge) {
    message.textContent = "";
    try {
      if (await data.dropForeignKey({ table: edge.from, name: edge.name })) {
        selection = null;
        await reload();
      }
    } catch (error) {
      message.textContent = error.message;
    }
  }

  // ---- export -------------------------------------------------------------------------------------------------------

  /** A standalone copy of the drawing: colours that came from the theme are written into each element. */
  function exportDocument() {
    const bounds = currentBounds();
    const copy = canvas.cloneNode(true);
    const copyViewport = copy.querySelector(".diagram-viewport");
    inlineStyles(canvas.querySelector(".diagram-viewport"), copyViewport);
    inlineStyles(canvas.querySelector("defs"), copy.querySelector("defs"));
    copy.querySelectorAll(".diagram-port, .diagram-edge-hit, .diagram-draft, title").forEach((node) => node.remove());
    copyViewport.removeAttribute("transform");
    const x = bounds.x - EXPORT_PADDING;
    const y = bounds.y - EXPORT_PADDING;
    const width = bounds.width + 2 * EXPORT_PADDING;
    const height = bounds.height + 2 * EXPORT_PADDING;
    const background = getComputedStyle(document.documentElement).getPropertyValue("--bg").trim() || "#ffffff";
    copy.insertBefore(svg("rect", { x, y, width, height, fill: background }), copy.firstChild);
    copy.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    copy.setAttribute("viewBox", `${x} ${y} ${width} ${height}`);
    copy.setAttribute("width", width);
    copy.setAttribute("height", height);
    copy.removeAttribute("class");
    return { text: new XMLSerializer().serializeToString(copy), width, height };
  }

  function inlineStyles(source, copy) {
    const computed = getComputedStyle(source);
    copy.setAttribute("style", STYLE_PROPERTIES.map((property) => `${property}:${computed.getPropertyValue(property)}`).join(";"));
    const copies = [...copy.children];
    [...source.children].forEach((child, index) => copies[index] && inlineStyles(child, copies[index]));
  }

  function pngBytes({ text, width, height }) {
    return new Promise((resolve, reject) => {
      const url = URL.createObjectURL(new Blob([text], { type: "image/svg+xml;charset=utf-8" }));
      const image = new Image();
      image.onload = () => {
        const surface = document.createElement("canvas");
        surface.width = Math.ceil(width * PNG_SCALE);
        surface.height = Math.ceil(height * PNG_SCALE);
        surface.getContext("2d").drawImage(image, 0, 0, surface.width, surface.height);
        URL.revokeObjectURL(url);
        surface.toBlob((blob) => (blob ? blob.arrayBuffer().then((buffer) => resolve(new Uint8Array(buffer))) : reject(new Error("The picture could not be made"))), "image/png");
      };
      image.onerror = () => (URL.revokeObjectURL(url), reject(new Error("The picture could not be made")));
      image.src = url;
    });
  }

  async function exportImage(kind) {
    message.textContent = "";
    try {
      const document_ = exportDocument();
      const bytes = kind === "svg" ? new TextEncoder().encode(document_.text) : await pngBytes(document_);
      const path = await data.saveImage(kind, bytes);
      if (path) status.textContent = `Saved ${path}`;
    } catch (error) {
      message.textContent = error.message;
    }
  }

  void load({ firstTime: true });

  return {
    element,
    reload: async () => (await reload(), true),
    hasUnsavedChanges: () => false,
    destroy() {
      clearTimeout(saveTimer);
      if (model !== null && !destroyed) data.saveLayout({ positions, view, keyColumnsOnly });
      destroyed = true;
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      resizer.disconnect();
    },
  };
}
