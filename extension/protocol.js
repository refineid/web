// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Minimal deterministic-CBOR writer for the relay-websocket-v1 join
// frames, shared by the service worker and the Node tests. Only the
// shapes this profile needs: short text, byte strings, short arrays.

const JOIN_MAGIC = "RAPP-relay-v1";
const JOINED_MAGIC = "RAPP-relay-joined-v1";

// Encode one text string (length < 24, ASCII only).
function encodeText(out, value) {
  if (value.length >= 24) throw new Error("text too long for profile writer");
  out.push(0x60 + value.length);
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code > 0x7f) throw new Error("profile writer is ASCII-only");
    out.push(code);
  }
}

// Encode one byte string (length < 256).
function encodeBytes(out, bytes) {
  if (bytes.length >= 256) throw new Error("bytes too long for profile writer");
  if (bytes.length < 24) {
    out.push(0x40 + bytes.length);
  } else {
    out.push(0x58, bytes.length);
  }
  for (const byte of bytes) out.push(byte);
}

function hexToBytes(hex) {
  if (hex.length % 2 !== 0) throw new Error("odd hex length");
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}

// Join frame: `["RAPP-relay-v1", purpose, role, token]`.
// Pairing legs pass the 32-byte offer_id hex; session legs the
// 16-byte rendezvous token hex.
export function encodeJoin(purpose, role, tokenHex) {
  const token = hexToBytes(tokenHex);
  if (token.length !== 32 && token.length !== 16) {
    throw new Error(`join token must be 32 or 16 bytes, got ${token.length}`);
  }
  const out = [0x84];
  encodeText(out, JOIN_MAGIC);
  encodeText(out, purpose);
  encodeText(out, role);
  encodeBytes(out, token);
  return new Uint8Array(out);
}

// The exact bytes of `["RAPP-relay-joined-v1"]`.
export function joinedBytes() {
  const out = [0x81];
  encodeText(out, JOINED_MAGIC);
  return new Uint8Array(out);
}

// True when the frame is exactly the joined signal.
export function isJoined(bytes) {
  const expected = joinedBytes();
  if (bytes.length !== expected.length) return false;
  for (let index = 0; index < expected.length; index += 1) {
    if (bytes[index] !== expected[index]) return false;
  }
  return true;
}
