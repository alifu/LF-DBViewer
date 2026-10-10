/** Pure conversions between a saved profile, the login form's values and a backend request. */

export const DEFAULT_MYSQL_PORT = 3306;
export const DEFAULT_SSH_PORT = 22;

export const ENGINES = Object.freeze([
  { value: "mysql", label: "MySQL" },
  { value: "mariadb", label: "MariaDB" },
]);

export const SSL_MODES = Object.freeze([
  { value: "disabled", label: "Off" },
  { value: "required", label: "Encrypt (no certificate check)" },
  { value: "verifyCa", label: "Verify CA" },
  { value: "verifyIdentity", label: "Verify CA and host name" },
]);

export const SSH_AUTH_METHODS = Object.freeze([
  { value: "agent", label: "SSH agent" },
  { value: "keyFile", label: "Private key file" },
  { value: "password", label: "Password" },
]);

export const EMPTY_FORM = Object.freeze({
  id: "",
  name: "",
  engine: "mysql",
  host: "127.0.0.1",
  port: String(DEFAULT_MYSQL_PORT),
  user: "root",
  password: "",
  database: "",
  socket: "",
  sslMode: "disabled",
  sslCa: "",
  sshEnabled: false,
  sshHost: "",
  sshPort: String(DEFAULT_SSH_PORT),
  sshUser: "",
  sshAuthMethod: "agent",
  sshKeyPath: "~/.ssh/id_ed25519",
  sshSecret: "",
  saveSecrets: true,
  hasSavedPassword: false,
});

export function profileToForm(profile) {
  const ssh = profile.ssh ?? null;
  return {
    ...EMPTY_FORM,
    id: profile.id,
    name: profile.name,
    engine: profile.engine,
    host: profile.host ?? "",
    port: String(profile.port ?? DEFAULT_MYSQL_PORT),
    user: profile.user,
    database: profile.database ?? "",
    socket: profile.socket ?? "",
    sslMode: profile.sslMode ?? "disabled",
    sslCa: profile.sslCa ?? "",
    sshEnabled: ssh !== null,
    sshHost: ssh?.host ?? "",
    sshPort: String(ssh?.port ?? DEFAULT_SSH_PORT),
    sshUser: ssh?.user ?? "",
    sshAuthMethod: ssh?.auth.method ?? EMPTY_FORM.sshAuthMethod,
    sshKeyPath: ssh?.auth.path ?? EMPTY_FORM.sshKeyPath,
    hasSavedPassword: Boolean(profile.hasSavedPassword),
  };
}

/** Fills the form from the terminal's MySQL client settings; unset values keep the form's own. */
export function applyClientDefaults(form, defaults) {
  return {
    ...form,
    host: defaults.host ?? form.host,
    port: defaults.port ? String(defaults.port) : form.port,
    user: defaults.user ?? form.user,
    password: defaults.password ?? form.password,
    socket: defaults.socket ?? form.socket,
    database: defaults.database ?? form.database,
  };
}

export function hasClientDefaults(defaults) {
  return Boolean(defaults) && ["host", "port", "user", "password", "socket", "database"].some((key) => defaults[key]);
}

function toPort(text, fallback) {
  const port = Number.parseInt(text, 10);
  return Number.isInteger(port) && port > 0 && port <= 65535 ? port : fallback;
}

function sshFromForm(form) {
  if (!form.sshEnabled) return null;
  const auth = form.sshAuthMethod === "keyFile" ? { method: "keyFile", path: form.sshKeyPath } : { method: form.sshAuthMethod };
  return { host: form.sshHost, port: toPort(form.sshPort, 0), user: form.sshUser, auth };
}

/**
 * A blank password on a profile that already has one saved means "use the saved one"
 * (sent as null); otherwise the typed text is sent as is. A blank SSH secret is always null.
 */
export function formToRequest(form) {
  const keepSavedPassword = form.password === "" && form.hasSavedPassword;
  return {
    profile: {
      id: form.id,
      name: form.name,
      engine: form.engine,
      host: form.host,
      port: toPort(form.port, 0),
      user: form.user,
      database: form.database || null,
      socket: form.socket || null,
      sslMode: form.sslMode,
      sslCa: form.sslCa || null,
      ssh: sshFromForm(form),
    },
    password: keepSavedPassword ? null : form.password,
    sshSecret: form.sshSecret === "" ? null : form.sshSecret,
    saveSecrets: form.saveSecrets,
  };
}
