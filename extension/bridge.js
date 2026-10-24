// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
// Content-script bridge: exposes a narrow page API on RefineID-owned
// origins only. Third-party site access is deferred: no page outside
// the allowlist gets a response, and signing operations do not exist
// yet (they ride in the next slice).

const ALLOWLIST = ["https://card.refineid.fi", "http://localhost:8080", "http://127.0.0.1:8080"];
const CHANNEL = "refineid-extension-v1";

function allowedOrigin(origin) {
  return ALLOWLIST.includes(origin);
}

function workerMessageFor(request) {
  const params = request.params || {};
  if (request.method === "status") return { type: "status" };
  if (request.method === "session") {
    if (!params.pairId || !params.relayUrl)
      return { error: "session needs params.pairId+relayUrl" };
    return { type: "session", pairId: params.pairId, relayUrl: params.relayUrl };
  }
  if (request.method === "close") return { type: "close" };
  return { error: `deferred method: ${request.method}` };
}

if (allowedOrigin(window.location.origin)) {
  window.addEventListener("message", (event) => {
    if (event.source !== window) return;
    if (event.origin !== window.location.origin) return;
    const request = event.data;
    if (!request || request.channel !== CHANNEL || !request.id) return;
    const message = workerMessageFor(request);
    if (message.error) {
      window.postMessage({ channel: CHANNEL, id: request.id, error: message.error }, event.origin);
      return;
    }
    chrome.runtime
      .sendMessage(message)
      .then((reply) => {
        window.postMessage({ channel: CHANNEL, id: request.id, reply }, event.origin);
      })
      .catch((error) => {
        window.postMessage(
          { channel: CHANNEL, id: request.id, error: String(error) },
          event.origin,
        );
      });
  });
}
