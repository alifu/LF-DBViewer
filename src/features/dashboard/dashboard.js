import { h } from "../../lib/dom.js";
import {
  EMPTY_FORM,
  ENGINES,
  SSH_AUTH_METHODS,
  SSL_MODES,
  applyClientDefaults,
  formToRequest,
  hasClientDefaults,
  profileToForm,
} from "./profile-form.js";

const CHECKBOX_FIELDS = ["sshEnabled", "saveSecrets"];

function field(label, control, { wide = false } = {}) {
  return h("label", { class: wide ? "field wide" : "field" }, h("span", {}, label), control);
}

function input(name, value, props = {}) {
  return h("input", { name, value, autocomplete: "off", autocapitalize: "off", spellcheck: false, ...props });
}

function select(name, value, options) {
  return h(
    "select",
    { name },
    options.map((option) => h("option", { value: option.value, selected: option.value === value }, option.label)),
  );
}

function checkbox(name, checked, label) {
  return h("label", { class: "check" }, h("input", { type: "checkbox", name, checked }), label);
}

function readForm(form, base) {
  const data = new FormData(form);
  const text = Object.fromEntries([...data.entries()].filter(([key]) => !CHECKBOX_FIELDS.includes(key)));
  const checks = Object.fromEntries(CHECKBOX_FIELDS.map((key) => [key, data.has(key)]));
  return { ...base, ...text, ...checks };
}

function sshFields(values) {
  const secretLabel = { password: "SSH password", keyFile: "Key passphrase (if any)" }[values.sshAuthMethod];
  return h(
    "div",
    { class: "form-grid" },
    field("SSH host", input("sshHost", values.sshHost, { placeholder: "bastion.example.com" })),
    field("SSH port", input("sshPort", values.sshPort, { inputMode: "numeric" })),
    field("SSH user", input("sshUser", values.sshUser)),
    field("Sign in with", select("sshAuthMethod", values.sshAuthMethod, SSH_AUTH_METHODS)),
    values.sshAuthMethod === "keyFile" && field("Key file", input("sshKeyPath", values.sshKeyPath), { wide: true }),
    secretLabel && field(secretLabel, input("sshSecret", values.sshSecret, { type: "password" }), { wide: true }),
  );
}

function formBody(values, clientDefaults) {
  const passwordHint = values.hasSavedPassword ? "saved in Keychain" : "";
  return [
    h("h2", {}, values.id ? "Edit connection" : "New connection"),
    hasClientDefaults(clientDefaults) &&
      h(
        "div",
        { class: "notice" },
        h("span", {}, `MySQL client settings found in ${clientDefaults.sources.join(", ")}.`),
        h("button", { type: "button", class: "ghost", dataset: { action: "useDefaults" } }, "Use them"),
      ),
    h(
      "div",
      { class: "form-grid" },
      field("Name", input("name", values.name, { placeholder: "Local MySQL", required: true }), { wide: true }),
      field("Database engine", select("engine", values.engine, ENGINES), { wide: true }),
      field("Host", input("host", values.host)),
      field("Port", input("port", values.port, { inputMode: "numeric" })),
      field("User", input("user", values.user)),
      field("Password", input("password", values.password, { type: "password", placeholder: passwordHint })),
      field("Default database (optional)", input("database", values.database)),
      field("Unix socket (optional)", input("socket", values.socket, { placeholder: "/tmp/mysql.sock" })),
      field("TLS", select("sslMode", values.sslMode, SSL_MODES)),
      field("CA certificate file (optional)", input("sslCa", values.sslCa)),
    ),
    h("div", { class: "form-section" }, checkbox("sshEnabled", values.sshEnabled, "Connect through an SSH server")),
    values.sshEnabled && sshFields(values),
    h("div", { class: "form-section" }, checkbox("saveSecrets", values.saveSecrets, "Save passwords in the macOS Keychain")),
  ];
}

function profileItem(profile, selectedId, onSelect, onConnect) {
  return h(
    "li",
    {
      class: profile.id === selectedId ? "profile selected" : "profile",
      onClick: () => onSelect(profile),
      onDblclick: () => onConnect(profile),
      title: "Double-click to connect",
    },
    h("strong", {}, profile.name),
    h("span", { class: "muted" }, `${profile.user}@${profile.host || profile.socket}${profile.ssh ? " via SSH" : ""}`),
  );
}

/**
 * Login screen: saved connections on the left, the connection form on the right.
 * `actions` are async and reject with an AppError, which is shown under the form.
 */
export function createDashboard({ actions }) {
  let values = EMPTY_FORM;
  let profiles = [];
  let clientDefaults = null;
  let openCount = 0;

  const list = h("ul", { class: "profile-list" });
  const fields = h("div", { class: "form-fields" });
  const message = h("p", { class: "form-message", role: "status" });
  const backButton = h("button", { type: "button", class: "ghost", onClick: () => actions.backToWorkspace() }, "Back to open connections");
  const deleteButton = h("button", { type: "button", class: "danger ghost", onClick: () => run("Deleting", remove) }, "Delete");
  const buttons = [
    h("button", { type: "button", onClick: () => run("Testing", test) }, "Test"),
    h("button", { type: "button", onClick: () => run("Saving", save) }, "Save"),
    h("button", { type: "submit", class: "primary" }, "Connect"),
  ];
  const form = h(
    "form",
    { class: "profile-form", onSubmit: (event) => (event.preventDefault(), run("Connecting", connect)) },
    fields,
    message,
    h("div", { class: "form-actions" }, deleteButton, h("span", { class: "spacer" }), buttons),
  );
  // Fields that change which other fields exist redraw the form with what was typed so far.
  form.addEventListener("change", (event) => {
    if (["sshEnabled", "sshAuthMethod"].includes(event.target.name)) draw(readForm(form, values));
  });
  form.addEventListener("click", (event) => {
    if (event.target.dataset?.action === "useDefaults") draw(applyClientDefaults(readForm(form, values), clientDefaults));
  });

  const element = h(
    "main",
    { class: "dashboard" },
    h(
      "aside",
      { class: "dashboard-side" },
      h("h1", {}, "LF-DBViewer"),
      h("div", { class: "panel-header" }, h("strong", {}, "Saved connections"), h("span", { class: "spacer" }), backButton),
      list,
      h("button", { type: "button", onClick: () => draw(EMPTY_FORM) }, "New connection"),
    ),
    h("section", { class: "dashboard-main" }, form),
  );

  function draw(next) {
    values = next;
    fields.replaceChildren(...formBody(values, clientDefaults).filter(Boolean));
    deleteButton.hidden = !values.id;
    showMessage("", "");
    drawList();
  }

  function drawList() {
    const items = profiles.map((profile) => profileItem(profile, values.id, (picked) => draw(profileToForm(picked)), connectSaved));
    list.replaceChildren(...(items.length > 0 ? items : [h("li", { class: "muted empty" }, "Nothing saved yet.")]));
    backButton.hidden = openCount === 0;
  }

  function showMessage(kind, text) {
    message.className = `form-message ${kind}`;
    message.textContent = text;
  }

  async function run(verb, work) {
    values = readForm(form, values);
    showMessage("", `${verb}…`);
    form.querySelectorAll("button").forEach((button) => (button.disabled = true));
    try {
      showMessage("ok", (await work(formToRequest(values))) ?? "");
    } catch (error) {
      showMessage("error", error.message);
    } finally {
      form.querySelectorAll("button").forEach((button) => (button.disabled = false));
    }
  }

  async function test(request) {
    const info = await actions.test(request);
    return `Connected: ${info.engine === "mariadb" ? "MariaDB" : "MySQL"} ${info.serverVersion}`;
  }

  async function save(request) {
    const saved = await actions.save(request);
    values = { ...values, id: saved.id, password: "", sshSecret: "", hasSavedPassword: values.hasSavedPassword || (request.saveSecrets && Boolean(request.password)) };
    return "Saved.";
  }

  async function connect(request) {
    await actions.connect(request);
  }

  async function remove() {
    if (await actions.remove(values)) draw(EMPTY_FORM);
  }

  function connectSaved(profile) {
    draw(profileToForm(profile));
    run("Connecting", connect);
  }

  draw(EMPTY_FORM);

  return {
    element,
    render(state) {
      profiles = state.profiles;
      openCount = state.connections.length;
      drawList();
    },
    setClientDefaults(defaults) {
      clientDefaults = defaults;
      draw(values);
    },
  };
}
