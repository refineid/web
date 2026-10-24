// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// MV3 module service worker: owns the relay WebSocket legs and pumps
// frames through the WASM requester. Pairing and session flows run
// here; the popup and the page bridge drive it via messages.
//
// Proto limits: pairs live in WASM memory, so a worker restart drops
// them; operations (signing) ride in the next slice.

import init, { Requester } from "./wasm/refineid_wasm_requester.js";
import { encodeJoin, isJoined } from "./protocol.js";

const FLOW_TIMEOUT_MS = 120_000;
const APP_LABEL = "RefineID browser extension";
const OS_LABEL = "browser";

// Noise frame plan per flow: the initiator writes m1, reads m2, and
// for XX writes m3. KK has no third message.
const PAIRING_WRITES = 2;

let requester = null;
let flow = null;

function nowMs() {
  return performance.now();
}

function setFlow(next) {
  if (flow && flow.timer) clearTimeout(flow.timer);
  flow = next;
  // Established legs stay open without a timeout; handshakes must finish.
  if (flow && flow.timeout !== false) {
    flow.timer = setTimeout(() => failFlow("flow timed out"), FLOW_TIMEOUT_MS);
  }
}

function failFlow(message) {
  if (flow && flow.socket) {
    try {
      flow.socket.close();
    } catch {
      // Already gone; the error below is what matters.
    }
  }
  const failed = flow ? flow.kind : "idle";
  setFlow(null);
  state.lastError = `${failed}: ${message}`;
}

const state = {
  ready: false,
  lastError: null,
};

async function ensureRequester() {
  if (!requester) {
    await init();
    requester = new Requester();
    state.ready = true;
  }
  return requester;
}

function openLeg(relayUrl, purpose, tokenHex) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(relayUrl);
    socket.binaryType = "arraybuffer";
    socket.onopen = () => {
      socket.send(encodeJoin(purpose, "requester", tokenHex));
    };
    socket.onerror = () => reject(new Error(`relay leg failed: ${relayUrl}`));
    socket.onmessage = (event) => {
      const bytes = new Uint8Array(event.data);
      if (!isJoined(bytes)) {
        reject(new Error("first relay frame was not the joined signal"));
        socket.close();
        return;
      }
      socket.onmessage = (later) => onFrame(new Uint8Array(later.data));
      socket.onerror = () => failFlow("relay leg failed");
      socket.onclose = () => {
        if (flow && flow.socket === socket && !flow.closing) failFlow("relay leg closed");
      };
      resolve(socket);
    };
  });
}

function sendFrame(bytes) {
  flow.socket.send(bytes);
  flow.writes += 1;
}

function drainWrites(next) {
  for (;;) {
    const out = next();
    if (!out) return;
    sendFrame(out);
  }
}

function onFrame(bytes) {
  if (!flow) return;
  try {
    if (flow.kind === "pairing-noise") onPairingNoise(bytes);
    else if (flow.kind === "pairing-hello") onPairingHello(bytes);
    else if (flow.kind === "pairing-confirm") onPairingConfirm(bytes);
    else if (flow.kind === "session-noise") onSessionNoise(bytes);
    else if (flow.kind === "session-ready") onSessionReady(bytes);
  } catch (error) {
    failFlow(String(error));
  }
}

function onPairingNoise(bytes) {
  const done = requester.pairing_feed_core(bytes);
  drainWrites(() => requester.pairing_next_core());
  if (done || flow.writes >= PAIRING_WRITES) {
    requester.pairing_confirm_begin_core();
    const hello = requester.pairing_hello_core(APP_LABEL, OS_LABEL);
    sendFrame(hello);
    flow.kind = "pairing-hello";
  }
}

function onPairingHello(bytes) {
  const peerJson = requester.pairing_on_hello_core(bytes, nowMs());
  flow.kind = "pairing-grants";
  flow.peer = JSON.parse(peerJson);
}

function onPairingConfirm(bytes) {
  const granted = requester.pairing_on_confirm_core(bytes, nowMs());
  const pairId = requester.pairing_finish_core(nowMs());
  flow.closing = true;
  flow.socket.close();
  const finished = { pairId, granted };
  setFlow(null);
  state.lastPairing = finished;
}

function onSessionNoise(bytes) {
  const done = requester.session_feed_core(bytes);
  drainWrites(() => requester.session_next_core());
  // KK completes on the m2 read; nothing is staged after it.
  if (!done) return;
  requester.session_auth_begin_core();
  const ready = requester.session_ready_core();
  sendFrame(ready);
  flow.kind = "session-ready";
}

function onSessionReady(bytes) {
  requester.session_on_ready_core(bytes, nowMs());
  state.session = { pairId: flow.pairId, established: requester.session_established() };
  // The leg stays open: post-handshake envelopes ride it.
  setFlow({
    kind: "established",
    socket: flow.socket,
    pairId: flow.pairId,
    writes: flow.writes,
    timeout: false,
  });
  flow.socket.onclose = () => {
    state.session = null;
    setFlow(null);
  };
}

async function startPairing(relayUrl) {
  if (flow) throw new Error("a flow is already running");
  const started = requester.begin_pairing_core(relayUrl, nowMs());
  const offer = JSON.parse(started);
  const socket = await openLeg(relayUrl, "pairing", offer.offer_id);
  setFlow({ kind: "pairing-noise", socket, writes: 0, code: offer.code });
  drainWrites(() => requester.pairing_next_core());
  return { code: offer.code, formatted: offer.formatted };
}

async function submitGrants(grants) {
  if (!flow || flow.kind !== "pairing-grants") throw new Error("no pairing awaits grants");
  const confirm = requester.pairing_confirm_core(grants);
  sendFrame(confirm);
  flow.kind = "pairing-confirm";
  return true;
}

async function startSession(pairId, relayUrl) {
  if (flow) throw new Error("a flow is already running");
  const m1 = requester.session_begin_core(pairId);
  const rendezvous = requester.pair_rendezvous(pairId);
  const socket = await openLeg(relayUrl, "session", rendezvous);
  setFlow({ kind: "session-noise", socket, writes: 0, pairId });
  sendFrame(m1);
  return true;
}

function snapshot() {
  return {
    ready: state.ready,
    lastError: state.lastError,
    pairs: requester ? requester.pair_ids() : [],
    flow: flow ? { kind: flow.kind, code: flow.code || null, peer: flow.peer || null } : null,
    lastPairing: state.lastPairing || null,
    session: state.session || null,
  };
}

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  (async () => {
    await ensureRequester();
    if (message.type === "status") return snapshot();
    if (message.type === "pair") return startPairing(message.relayUrl);
    if (message.type === "grants") return submitGrants(message.grants);
    if (message.type === "session") return startSession(message.pairId, message.relayUrl);
    if (message.type === "close") {
      requester.session_close();
      state.session = null;
      if (flow && flow.socket) flow.socket.close();
      setFlow(null);
      return true;
    }
    if (message.type === "cancel") {
      if (flow && flow.socket) flow.socket.close();
      setFlow(null);
      return true;
    }
    throw new Error(`unknown message: ${message.type}`);
  })().then(respond, (error) => respond({ error: String((error && error.message) || error) }));
  return true;
});

ensureRequester().catch((error) => {
  state.lastError = String(error);
});
