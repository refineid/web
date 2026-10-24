# Demo login with the RefineID extension

Logs in to a demo page through the browser extension, the RAPP relay,
and an approving proxy. No card is involved yet; approval proves pair
possession over a mutually authenticated Noise channel plus explicit
human consent at the proxy.

## Run it (three terminals + Chrome)

Terminal 1, the relay:

```sh
cd relay
cargo run -- --bind 127.0.0.1:9443
```

Terminal 2, the demo page:

```sh
./scripts/serve.sh
```

Chrome: load `extension/` unpacked (build it first with
`./scripts/build-extension-wasm.sh`), open the popup, click **Pair new
phone**, and note the 6-digit code.

Terminal 3, the proxy (same code):

```sh
cd proxy-cli
cargo run -- serve --code 123456 --relay ws://127.0.0.1:9443
```

Approve the pairing at the proxy prompt. Back in the popup, tick the
grants and confirm. Then open http://localhost:8080/login.html, click
**Login with RefineID**, and approve the login at the proxy prompt.
The page shows LOGGED IN with the pair id.

## Clean-machine packaging

A test machine needs no Rust and no RefineID install, only Chrome.
Copy from a built checkout:

- `extension/` including the built `extension/wasm/` bundle
- `relay/target/release/refineid-relay-poc`
- `proxy-cli/target/release/refineid-proxy-cli`
- `public/login.html`, `public/login.js` plus any static server on
  `localhost:8080` (the bridge allowlists that origin)

Then run the same flow: relay, static server, unpacked extension,
proxy CLI.

## Why not suomi.fi directly

Suomi.fi e-Identification's certificate-card method (Varmennekortti)
navigates to `kortti.tunnistautuminen.suomi.fi`, which authenticates
with **TLS client certificates**: the browser presents the card
certificate inside the TLS handshake and the card signs the exchange
(verified end to end on Android; see ADR 0036 in refineid-android).

A Chrome extension cannot inject a key into Chrome's TLS handshake:
client certificates come from the platform key store only, and no
extension API participates in the handshake. So a clean-Chrome login
to suomi.fi via our extension is blocked regardless of the relay and
proxy work. Options, honestly ordered:

1. **Demo RP now (this document).** Prove the login ceremony on our
   own service: extension + relay + approving proxy.
2. **Platform smartcard integration.** A native TokenD/minidriver
   would expose the phone-held key to Chrome's TLS stack, but that is
   a RefineID install, contradicting the clean-machine goal.
3. **WASM TLS through a gateway.** A WASM TLS client could handshake
   through a relay-to-TCP gateway with RAPP-signed CertificateVerify
   messages, but the login session would live in the extension, not
   the user's suomi.fi tab; moving it over is fragile session handling
   against a service we do not operate.
4. **Operator cooperation.** DVV accepting a RAPP-based eID means is a
   business track, not an engineering one.

Full evidence and verdict: `SUOMI-FI.md`.

## iPhone gap spec

The dev iPhone has identity primed but the Apple app does not speak
`relay-websocket-v1` (no implementation in Sources/CardCore/Tests).
To pair with the extension it needs, at minimum:

1. A relay leg: open `relay_url`, send the join frame
   `["RAPP-relay-v1", purpose, role, token]`, wait for
   `["RAPP-relay-joined-v1"]`, then forward opaque frames. Spec:
   `refineid-core/docs/protocols/rapp-relay-websocket-v1.md`.
2. The fixed offer template (spec section 5.1) with the same
   `relay_url` default both peers share, and the code-derived
   `offer_id` (32-byte SHA-256) plus `psk3` pairing secret.
3. The proxy side of pairing confirmation and sessions (the engine
   already implements the roles; only the transport is missing).
4. For real logins: execute `CardOperation::BrowserAuthenticate`
   (origin, key profile, algorithm, digest) with local authorizer
   consent and the card key.

The sibling Apple repository is read-only from this checkout, so that
work lands there, not here.

## Next slice

Drive `BrowserAuthenticate` through the extension worker once a
card-backed proxy exists: the demo page supplies origin + digest, the
worker runs the operation over the established leg, the proxy returns
the card signature. The session plumbing it rides on is done.
