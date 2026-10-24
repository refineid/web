// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Golden vectors for the relay-websocket-v1 join frames: exact bytes
// per the profile spec, plus reader checks. Run: node --test tests/.
import { describe, it } from "node:test";
import assert from "node:assert/strict";

import { encodeJoin, isJoined, joinedBytes } from "../extension/protocol.js";

function hex(bytes) {
  return Buffer.from(bytes).toString("hex");
}

// ["RAPP-relay-v1", "pairing", "requester", h'00..00' (32)].
const JOIN_PAIRING_ZERO =
  "84" +
  "6d524150502d72656c61792d7631" +
  "6770616972696e67" +
  "69726571756573746572" +
  "5820" +
  "00".repeat(32);

// ["RAPP-relay-joined-v1"].
const JOINED = "8174524150502d72656c61792d6a6f696e65642d7631";

describe("relay join frames", () => {
  it("encodes the pairing join byte-exactly", () => {
    assert.equal(hex(encodeJoin("pairing", "requester", "00".repeat(32))), JOIN_PAIRING_ZERO);
  });

  it("encodes a 16-byte session token with one-byte length", () => {
    const frame = hex(encodeJoin("session", "requester", "ab".repeat(16)));
    assert.ok(frame.endsWith("50" + "ab".repeat(16)), frame);
    assert.ok(frame.includes("6773657373696f6e"), frame); // t(7) "session"
  });

  it("builds the exact joined signal", () => {
    assert.equal(hex(joinedBytes()), JOINED);
  });

  it("recognizes only the exact joined signal", () => {
    assert.equal(isJoined(joinedBytes()), true);
    assert.equal(isJoined(joinedBytes().slice(0, 10)), false);
    const wrong = joinedBytes();
    wrong[wrong.length - 1] ^= 0x01;
    assert.equal(isJoined(wrong), false);
  });

  it("rejects bad token shapes", () => {
    assert.throws(() => encodeJoin("pairing", "requester", "00".repeat(15)), /32 or 16/);
    assert.throws(() => encodeJoin("pairing", "requester", "abc"), /odd hex/);
  });
});
