import assert from "node:assert/strict";
import { test } from "node:test";

import { hexDump } from "./hex-dump.js";

test("a short value is one padded line with its characters", () => {
  assert.equal(hexDump("4869ff00"), "00000000  48 69 ff 00                                      Hi..");
});

test("long values wrap every sixteen bytes with running offsets", () => {
  const lines = hexDump("00".repeat(20)).split("\n");

  assert.equal(lines.length, 2);
  assert.ok(lines[1].startsWith("00000010  00 00 00 00 "));
});

test("nothing gives nothing", () => {
  assert.equal(hexDump(""), "");
});
