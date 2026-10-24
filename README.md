# RefineID Web research

Target: one self-contained page for browser-based document signing, with direct
USB card access and no installed helper or application backend.

The first prototype tests reader access only. Standard CCID interfaces are
protected by WebUSB, so proving this boundary comes before PDF or card signing.

## Start the agent

Muse 1.3 is selected because its CLI is already available locally. This choice
does not assert a model-quality comparison. The launcher uses the model ID
`muse-spark-1.3-contributor` found in the existing RefineID tooling guidance;
account access to that model has not been tested.

```sh
./scripts/start-muse.sh --check  # local prerequisite check; no model request
./scripts/start-muse.sh          # interactive Muse session with PROMPT.md
```

The launcher retains Muse's normal approval/sandbox defaults. It starts in this
checkout and bypasses any interactive shell alias for `muse`.

## Preview the generated page

After the agent creates `public/index.html`:

```sh
./scripts/serve.sh
```

Open http://localhost:8080 in Chrome. The development server uses the already
installed Python standard library, binds only to loopback, and serves `public/`
only. Python is development tooling, not a dependency of the delivered page.
Hosted use requires HTTPS. Firefox has no WebUSB implementation, so the page
reports that instead of attempting access.

## Automated tests

Reproducible tests run locally using already installed Node.js and Chrome binaries:

```sh
node tests/webusb-harness.js  # 75 synthetic assertions (baseline + Bug 1 & 2 regressions)
node tests/browser-smoke.js   # 36 real Chrome smoke assertions via loopback & CDP
```

## Use the hardware probe

1. Click **Connect…** and pick the reader in the browser chooser (cancelling
   is a normal outcome and is reported as such).
2. Read the descriptor panel: VID/PID, interface numbers, class codes, and
   endpoint types/directions. Class `0x0B` (Smart Card) is flagged protected.
3. Select the reader interface and click **Claim selected interface**. The
   exact stage and exception name are reported; a standard CCID reader is
   expected to fail with `SecurityError` on an ordinary page.
4. Use **Disconnect**, try unplug/reconnect and double-clicks, and **Copy
   report** to capture each run. The page never sends APDUs, performs
   transfers, powers a card, asks for a PIN, reads USB serial numbers, or
   stores anything.

## mTLS login proof-of-concept (no card needed)

Since ordinary pages cannot touch CCID readers, this experiment flips the
model: the _browser_ presents a client certificate during the TLS handshake
and the _server_ asserts the login. No page JavaScript, extension, or card
access is involved.

```sh
./poc-mtls/mkcerts.sh        # once: throwaway test CA + server/client certs
python3 poc-mtls/selftest.py # verify: login with cert, reject without
python3 poc-mtls/server.py   # serve https://localhost:8443 (loopback only)
```

Then import `poc-mtls/pki/client.p12` (password `refineid-poc`) into your
OS keychain or browser certificate store, open `https://localhost:8443`,
accept the loopback certificate warning, and select the test certificate
when the browser asks. The page shows the server-verified identity. Without
a client certificate the handshake itself fails — there is no login page to
fall back to, by design.

All identities are synthetic and git-ignored. See `RESEARCH.md` for what
this proves and what a FINEID production would still need (DVV/VRK trust,
certificate policy, revocation).

`PROMPT.md` contains the bounded task. `RESEARCH.md` records the verified
browser rules, what was tested (synthetic harness only — no physical reader
was operated in preparation), the remaining hardware steps, and the go/no-go
decision. The original Apache-2.0 license is retained.

## RAPP relay phase (extension + proxy)

Past the USB and mTLS milestones, the prototype moved to phone-held
identity over the untrusted `relay-websocket-v1` transport:
`relay/` (relay PoC), `wasm-requester/` (WASM requester), `extension/`
(MV3 extension), `proxy-cli/` (terminal demo proxy). `LOGIN.md` runs
the demo login end to end and records why suomi.fi itself is not
directly reachable from a clean-Chrome extension.
