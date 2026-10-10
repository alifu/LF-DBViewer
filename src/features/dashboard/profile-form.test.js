import assert from "node:assert/strict";
import { test } from "node:test";

import { EMPTY_FORM, applyClientDefaults, formToRequest, hasClientDefaults, profileToForm } from "./profile-form.js";

const savedProfile = {
  id: "p1",
  name: "Staging",
  engine: "mariadb",
  host: "db.internal",
  port: 3307,
  user: "app",
  database: "shop",
  socket: null,
  sslMode: "required",
  sslCa: null,
  ssh: { host: "bastion", port: 2222, user: "deploy", auth: { method: "keyFile", path: "~/.ssh/id_rsa" } },
  hasSavedPassword: true,
};

test("a saved profile round-trips through the form", () => {
  const request = formToRequest(profileToForm(savedProfile));

  const { hasSavedPassword, ...expected } = savedProfile;
  assert.deepEqual(request.profile, expected);
});

test("a blank password on a profile with a saved one means use the saved one", () => {
  const request = formToRequest(profileToForm(savedProfile));

  assert.equal(request.password, null);
  assert.equal(request.sshSecret, null);
});

test("a blank password on a new profile means no password", () => {
  assert.equal(formToRequest(EMPTY_FORM).password, "");
});

test("a typed password is always sent", () => {
  const form = { ...profileToForm(savedProfile), password: "new" };

  assert.equal(formToRequest(form).password, "new");
});

test("ssh is omitted when disabled and agent auth carries no path", () => {
  assert.equal(formToRequest(EMPTY_FORM).profile.ssh, null);

  const form = { ...EMPTY_FORM, sshEnabled: true, sshHost: "bastion", sshUser: "me" };
  assert.deepEqual(formToRequest(form).profile.ssh, { host: "bastion", port: 22, user: "me", auth: { method: "agent" } });
});

test("an unusable port becomes 0 so the backend rejects it", () => {
  assert.equal(formToRequest({ ...EMPTY_FORM, port: "abc" }).profile.port, 0);
  assert.equal(formToRequest({ ...EMPTY_FORM, port: "70000" }).profile.port, 0);
});

test("client defaults fill only what they define", () => {
  const form = applyClientDefaults(EMPTY_FORM, { host: "cnfhost", port: 3310, user: null, sources: ["~/.my.cnf"] });

  assert.equal(form.host, "cnfhost");
  assert.equal(form.port, "3310");
  assert.equal(form.user, "root");
  assert.equal(EMPTY_FORM.host, "127.0.0.1");
});

test("empty client defaults are recognised", () => {
  assert.equal(hasClientDefaults({ host: null, port: null, sources: [] }), false);
  assert.equal(hasClientDefaults(null), false);
  assert.equal(hasClientDefaults({ socket: "/tmp/mysql.sock" }), true);
});
