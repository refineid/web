// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Demo relying party: login ceremony through the extension bridge.
// Flow: status -> pick pair -> session -> poll until established.
// Served from localhost:8080, which the bridge allowlists.

const CHANNEL = "refineid-extension-v1";
const POLL_MS = 1000;
const SESSION_WAIT_MS = 125_000;

const relayInput = document.getElementById("relay");
const loginButton = document.getElementById("login");
const logoutButton = document.getElementById("logout");
const resultDiv = document.getElementById("result");

let nextId = 1;
const pending = new Map();

window.addEventListener("message", (event) => {
  if (event.source !== window) return;
  if (event.origin !== window.location.origin) return;
  const reply = event.data;
  if (!reply || reply.channel !== CHANNEL || !reply.id) return;
  const waiter = pending.get(reply.id);
  if (!waiter) return;
  pending.delete(reply.id);
  if (reply.error) waiter.reject(new Error(reply.error));
  else waiter.resolve(reply.reply);
});

function call(method, params) {
  return new Promise((resolve, reject) => {
    const id = `login-${nextId++}`;
    pending.set(id, { resolve, reject });
    setTimeout(() => {
      if (pending.delete(id)) reject(new Error("extension did not answer (is it installed?)"));
    }, 5000);
    window.postMessage({ channel: CHANNEL, id, method, params }, window.location.origin);
  });
}

function show(text, kind) {
  resultDiv.textContent = text;
  resultDiv.className = kind || "";
}

async function status() {
  const reply = await call("status");
  if (reply && reply.error) throw new Error(reply.error);
  return reply;
}

async function waitEstablished(deadline) {
  for (;;) {
    const state = await status();
    if (state.session && state.session.established) return state.session;
    if (state.lastError && !state.flow) throw new Error(state.lastError);
    if (Date.now() > deadline) throw new Error("timed out waiting for proxy approval");
    await new Promise((done) => setTimeout(done, POLL_MS));
  }
}

loginButton.addEventListener("click", async () => {
  loginButton.disabled = true;
  try {
    show("Asking extension for pairs...");
    const state = await status();
    if (!state.ready) throw new Error("extension worker not ready yet, retry shortly");
    if (state.pairs.length === 0) {
      throw new Error("no pairs yet: run Pair new phone in the popup, approve at the proxy");
    }
    const pairId = state.pairs[0];
    show(`Opening session for pair ${pairId.slice(0, 12)}... approve at the proxy.`);
    const started = await call("session", { pairId, relayUrl: relayInput.value });
    if (started && started.error) throw new Error(started.error);
    const session = await waitEstablished(Date.now() + SESSION_WAIT_MS);
    show(
      `LOGGED IN\npair: ${session.pairId}\n\nDemo session only: pair possession +\napproved consent, no card signature yet.`,
      "logged-in",
    );
    logoutButton.disabled = false;
  } catch (error) {
    show(`Login failed: ${error.message}`, "error");
    loginButton.disabled = false;
  }
});

logoutButton.addEventListener("click", async () => {
  try {
    await call("close");
  } catch {
    // Already gone; the UI reset below is what matters.
  }
  show("Not logged in. Pair your proxy first (extension popup shows the code).");
  loginButton.disabled = false;
  logoutButton.disabled = true;
});
