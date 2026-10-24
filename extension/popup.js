// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Popup UI: drives pairing and sessions through the worker, polls for
// status. No secrets are displayed: pair ids and the pairing code are
// shown because the user must compare/enter them.

const GRANT_AUTH = "fi.refineid.authentication.v1";
const GRANT_SIGN = "fi.refineid.document-signing.v1";

const relayInput = document.getElementById("relay");
const pairButton = document.getElementById("pair");
const cancelButton = document.getElementById("cancel");
const closeButton = document.getElementById("close");
const codeRow = document.getElementById("codeRow");
const codeDiv = document.getElementById("code");
const peerDiv = document.getElementById("peer");
const grantsRow = document.getElementById("grantsRow");
const grantAuth = document.getElementById("grantAuth");
const grantSign = document.getElementById("grantSign");
const grantsButton = document.getElementById("grants");
const pairsDiv = document.getElementById("pairs");
const statusDiv = document.getElementById("status");

function send(message) {
  return chrome.runtime.sendMessage(message);
}

function showError(value) {
  statusDiv.textContent = value && value.error ? `error: ${value.error}` : JSON.stringify(value);
  statusDiv.classList.toggle("error", Boolean(value && value.error));
}

pairButton.addEventListener("click", async () => {
  const reply = await send({ type: "pair", relayUrl: relayInput.value });
  if (reply && reply.error) {
    showError(reply);
    return;
  }
  codeRow.hidden = false;
  codeDiv.textContent = reply.formatted || reply.code;
  await refresh();
});

grantsButton.addEventListener("click", async () => {
  const grants = [];
  if (grantAuth.checked) grants.push(GRANT_AUTH);
  if (grantSign.checked) grants.push(GRANT_SIGN);
  const reply = await send({ type: "grants", grants });
  showError(reply === true ? "grants sent, waiting for phone..." : reply);
  await refresh();
});

cancelButton.addEventListener("click", async () => {
  await send({ type: "cancel" });
  codeRow.hidden = true;
  grantsRow.hidden = true;
  await refresh();
});

closeButton.addEventListener("click", async () => {
  await send({ type: "close" });
  await refresh();
});

async function refresh() {
  const status = await send({ type: "status" });
  if (!status || status.error) {
    showError(status || "no reply from worker");
    return;
  }
  statusDiv.classList.remove("error");
  const lines = [];
  lines.push(`worker ready: ${status.ready}`);
  if (status.flow) {
    lines.push(`flow: ${status.flow.kind}`);
    if (status.flow.code) {
      codeRow.hidden = false;
      codeDiv.textContent = status.flow.code;
    }
    if (status.flow.kind === "pairing-grants") {
      grantsRow.hidden = false;
      const peer = status.flow.peer || {};
      peerDiv.textContent = `phone says: ${peer.display_name || "?"} / ${peer.platform || "?"}`;
    } else {
      grantsRow.hidden = true;
    }
  } else {
    codeRow.hidden = true;
    grantsRow.hidden = true;
  }
  if (status.lastPairing) {
    lines.push(
      `paired: ${status.lastPairing.pairId} (${status.lastPairing.granted.length} grants)`,
    );
  }
  if (status.session) {
    lines.push(`session: ${status.session.pairId} established=${status.session.established}`);
  }
  if (status.lastError) lines.push(`last error: ${status.lastError}`);
  statusDiv.textContent = lines.join("\n");

  pairsDiv.innerHTML = "";
  if (status.pairs.length === 0) {
    pairsDiv.innerHTML = "<em>none yet</em>";
  }
  for (const pairId of status.pairs) {
    const row = document.createElement("div");
    const label = document.createElement("code");
    label.textContent = `${pairId.slice(0, 12)}...`;
    const button = document.createElement("button");
    button.textContent = "Start session";
    button.addEventListener("click", async () => {
      const reply = await send({ type: "session", pairId, relayUrl: relayInput.value });
      if (reply && reply.error) showError(reply);
      await refresh();
    });
    row.append(label, " ", button);
    pairsDiv.append(row);
  }
}

refresh();
setInterval(refresh, 1000);
