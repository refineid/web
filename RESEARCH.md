# RefineID single-page feasibility: reader-access research

## Verified browser rules (primary sources)

1. **WebUSB is Chromium-only.** MDN browser-compat-data `api/USB.json`
   reports `chrome: version_added 61` (Edge/Opera mirror it) and
   `firefox: version_added false`, `safari: version_added false`.
   Raw source fetched and confirmed 2026-09-26:
   https://raw.githubusercontent.com/mdn/browser-compat-data/main/api/USB.json
   (also browsable at
   https://github.com/mdn/browser-compat-data/blob/main/api/USB.json).
   Consequence: Firefox support must be reported as absent. The prototype
   detects a missing `navigator.usb` and says so instead of failing obscurely.

2. **Secure-context gate.** WebUSB requires a secure context.
   `http://localhost` (loopback) is a secure context; an arbitrary `http://`
   LAN URL is not. Reference:
   https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Secure_Contexts
   The prototype checks `isSecureContext` separately from `navigator.usb` and
   explains each failure distinctly. Development URL: `http://localhost:8080`;
   hosted use requires `https://`.

3. **Smart Card class 0x0B is a protected interface class.** The WebUSB spec
   section "protected interface classes" lists 0x01 Audio, 0x03 HID,
   0x08 Mass Storage, 0x09 Hub, **0x0B Smart Card**, 0x0E Video,
   0x10 Audio/Video Devices, 0xE0 Wireless Controller. `claimInterface` (and
   `selectAlternate` / transfers) on a protected interface rejects with
   `SecurityError` unless the document has the `usb-unrestricted` permission
   policy. Verified from the live spec text 2026-09-26:
   https://usb.spec.whatwg.org/#protected-interface-classes and
   https://usb.spec.whatwg.org/#permissions-policy

4. **The `usb-unrestricted` exception is Isolated Web Apps only.**
   Per the permissions-policy integration section, the effective default
   allowlist is `"none"`; the feature must only be enabled for Isolated Web
   Apps declaring it in their manifest. An ordinary `https://` page — this
   prototype's target — can never pass this gate for class 0x0B.

5. **Vendor-specific (0xFF) interfaces are not on the protected list, but
   access alone proves nothing.** Chrome's WebUSB guidance notes OS driver
   ownership/permissions also decide whether a claim succeeds:
   https://developer.chrome.com/docs/capabilities/build-for-webusb
   Even a successful claim says nothing about speaking CCID/PC/SC to the
   device; the prototype performs zero transfers by design.

## Confirmed bug fixes (observed and verified)

1. **Bug 1: `selConfig` change handler interface rebuilding.**
   - *Problem:* Changing `selConfig` updated `state.selection.configurationValue` but did not rebuild the `selInterface` options for the newly selected configuration. If configuration 1 had interface 0 and configuration 2 had interface 7, switching to configuration 2 left the UI and claim button offering interface 0.
   - *Fix:* Added `updateSelectedConfig()` and `populateInterfaces(cfg)`, invoked on initial device inspection and on every `selConfig` change event. It looks up the selected configuration on the device, clears and populates `selInterface` options with formatted interface numbers and class descriptors (including `[PROTECTED: <name>]` annotations), synchronizes `state.selection` (`configurationValue`, `interfaceNumber`), disables `selInterface` and `btnClaim` if a configuration exposes no interfaces, and refreshes the diagnostic report. When `btnClaim` is clicked, `device.selectConfiguration` and `device.claimInterface` strictly target the agreed selection.

2. **Bug 2: Lifecycle races & open device leak prevention.**
   - *Problem:* Calling Disconnect while `device.open()` was pending leaked the device when the open promise resolved later, because the stale callback returned without closing the now-open device. Reconnecting the same device while cleanup was in-flight risked state corruption, and late work could close newer connections or repopulate stale UI.
   - *Fix:* Introduced per-device tracking Maps (`pendingOps` and `pendingCleanups`). When `cleanupDevice()` runs, it delegates to `cleanupSpecificDevice(device, claimed, reason)`, which chains off any in-flight device operation (e.g. pending `device.open()` or `device.claimInterface()`). When a delayed `open()` settles, if the run was canceled (`id !== state.runId`), the device is closed immediately without leaking. If a user reconnects the same device while cleanup is pending, `doConnect()` awaits `pendingCleanups.get(device)` before reopening. Furthermore, late cleanup of Device A never closes a newer active connection to Device B or touches its UI.

## What the prototype actually tested

`public/index.html` is one self-contained file (HTML+CSS+JS inline, ~30 KB,
zero network requests, zero storage APIs, never reads `device.serialNumber`).

- **Static checks (observed):** no `src=`/`href=`/`fetch`/`XHR`, no
  `localStorage`/`sessionStorage`/`indexedDB`/cookies, no `transferIn` /
  `transferOut` / `controlTransfer` calls, no `.serialNumber` reads
  (the string appears only in code comments); `node -e "..."` syntax check passes.

- **Synthetic test harness (observed, `tests/webusb-harness.js`, test doubles, no hardware): 75/75 pass.**
  Command: `node tests/webusb-harness.js`
  - *Baseline assertions preserved (29/29):* Insecure-origin message, missing `navigator.usb` (Firefox-like) message, ready state enables Connect, no auto-prompt on load, permission cancellation (`NotFoundError`) dismissal without errors, CCID device descriptor display (VID/PID, class `0x0B`, bulk endpoints) with protected flagging, claim rejection recorded as `claimInterface failed: SecurityError`, vendor-class (0xFF) claim success, release+close on Disconnect, duplicate connect clicks ignored, unplug cleanup via `disconnect` event, `InvalidStateError` (already claimed) with exact stage, serial number absent from descriptors and report, copy fallback.
  - *Bug 1 regression assertions (20/20):* Dynamic rebuilding of `selInterface` when switching configurations (e.g. config 1 with interface 0 to config 2 with interface 7); removal of stale interface options; synchronization of UI, report selection, and USB claim arguments (`selectConfiguration(2)` and `claimInterface(7)`); configuration with no interfaces (`interfaces: []`) clearing `selInterface`, disabling `selInterface` and `btnClaim`, recording `null` interfaceNumber, and blocking claim calls; switching to configurations containing protected alternate settings (class 0x0B) with visible `PROTECTED` labels, `SecurityError` rejection, and protected-class policy logging.
  - *Bug 2 regression assertions (26/26):* Deferred `device.open()` promise where Disconnect is clicked while `open()` is pending; resolving the delayed open proves the device is closed and not leaked (`dev.opened === false`); stale descriptors are not repopulated; report shows disconnected state; guard against reconnecting the same device while cleanup is pending (`connect waiting` logged, second open called only after first cleanup closes device); late work on slow Device A does not close a newer active connection to Device B or overwrite Device B's UI/report; deferred `claimInterface()` canceled by Disconnect cleanly releases and closes device; unplug event during pending `open()` cleans up safely.

- **Real Chrome browser smoke test (observed, `tests/browser-smoke.js`, real browser execution): 36/36 pass.**
  Command: `node tests/browser-smoke.js`
  - *Browser Environment:* Google Chrome 154.0.8037.58 (`HeadlessChrome/154.0.0.0 Safari/537.36`), V8 15.4.80.11, macOS Darwin.
  - *Server:* Loopback HTTP server (`python3 -m http.server`) serving `public/` on `127.0.0.1:8080` (or ephemeral loopback port).
  - *Page Load & Rendering (14 checks):* Title matches; loopback treated as secure context (`window.isSecureContext === true`); `navigator.usb` natively present; capabilities panel displays `verdict: ready` with class `ok`; initial controls verified (`btnConnect` enabled, `btnDisconnect`/`btnClaim`/`selConfig`/`selInterface` disabled); initial diagnostic report JSON generated; zero console errors and zero uncaught script exceptions on load.
  - *Native Chooser Lifecycle & Cancellation (7 checks):* Native WebUSB `requestDevice({ filters: [] })` invoked via user gesture activation; enters `connect requested: Waiting for the browser device chooser…`; Disconnect while awaiting chooser cleanly cancels and re-enables `btnConnect`; permission dismissal (`NotFoundError`) handled gracefully without errors.
  - *In-Browser Simulated USB Flows (15 checks):* Executed inside real Chrome DOM and JavaScript engine:
    - Simulated CCID reader (0x0B): Descriptors flagged `PROTECTED: Smart Card`, serial number absent, `claimInterface` rejects with `SecurityError`, WebUSB protected-class policy verdict logged, device cleanly closed on disconnect.
    - Bug 1 in Chrome DOM: Switching from Config 1 to Config 2 dynamically updates `selInterface` from interface 0 to interface 7 in the live DOM, claim routes to interface 7, and Config 3 (no interfaces) empties selector and disables claim button.
    - Bug 2 in Chrome DOM: Delayed `open()` followed by Disconnect and late resolution confirms `dev.opened === false` in Chrome (no leak), descriptors remain "No device selected.", and report remains disconnected.

- **Hardware observation (2026-09-26, user-run, physical silicon):** ACS ACR39U
  ICC Reader (VID `0x072F`, PID `0xB100`) — see "Physical hardware
  observation" below. This is the first real-device confirmation.

## Physical hardware observation (2026-09-26, user-run)

Device: ACS ACR39U ICC Reader, VID `0x072F` PID `0xB100`. Single
configuration (value 1, already active), one interface (number 0, class
`0x0B` flagged `[PROTECTED: Smart Card]`), endpoints bulk-out 1, bulk-in 2,
interrupt-in 3 — a textbook CCID descriptor set. No serial number appears
anywhere in the report.

Stage trace: `init` (ready) → `connect requested` → `permission granted`
→ `open` → `selectConfiguration skipped` → `inspect` (1 protected
alternate, `SecurityError` predicted) → `claimInterface` (interface 0) →
`claimInterface failed: SecurityError — The requested interface implements
a protected class` → protected-class policy verdict. Claim round-trip
took ~8 ms. This is exactly the spec-mandated outcome: user permission
granted, `open()` fine, claim blocked at the policy gate.

This confirms the NO-GO for standard CCID readers on physical silicon, not
just from spec text and doubles. Still unexercised on hardware: Disconnect
after a failed claim, unplug-during-connection, reconnect, and
double-click Connect.

## Remaining hardware test steps (for the user, in Chrome)

1. Run `./scripts/serve.sh` and open `http://localhost:8080` in Google Chrome.
2. Click **Connect…**; Chrome will display its native USB device chooser prompt.
3. Select your physical USB smart card reader in the chooser (or click Cancel to observe the normal dismissal path).
4. Confirm the descriptor panel shows VID/PID, interface numbers, class codes, and endpoint types/directions, and that class `0x0B` is flagged `[PROTECTED: Smart Card]`.
5. Select the reader interface from the dropdown (note that changing configurations dynamically rebuilds the interface list), click **Claim selected interface**, and observe the reported stage and exception name (expected: `SecurityError` for standard CCID readers on an ordinary page).
6. Test Disconnect, unplugging the reader during connection, reconnecting the same reader, and double-clicking Connect; use **Copy report** to capture diagnostic runs.

## Go / no-go decision

- **Standard CCID readers (Smart Card class 0x0B): NO-GO for the ordinary
  single-page approach.** The block is spec-level (`SecurityError` even with
  user permission) and unbypassable from a normal page; only an Isolated Web
  App with `usb-unrestricted` could proceed, which is outside this
  prototype's scope.
- **A reader exposing an accessible vendor-specific interface: CONDITIONAL GO
  past the claim gate only.** A successful `claimInterface` would permit a
  second milestone (async CCID-over-WebUSB orchestration against the
  deterministic engine in `refineid-core`, plus WASM portability work), but
  claim success still establishes no card protocol, and signing remains a
  separate future milestone.
- **Firefox: NO-GO, truthfully.** No WebUSB implementation exists; the page
  reports this instead of attempting access.

## mTLS login proof-of-concept (2026-09-26, synthetic PKI only)

Motivation: with page-to-card access proven impossible, the remaining
zero-RefineID-install path is mutual TLS — the *browser* presents a client
certificate (via OS/vendor card middleware) and the *server* asserts the
login. `poc-mtls/` implements this on loopback with Python stdlib only:
`mkcerts.sh` builds a throwaway test CA + server cert + one synthetic
client identity (FINEID-shaped subject, no real person); `server.py`
requires and verifies the client cert (`CERT_REQUIRED`, TLS ≥ 1.2) and
renders the verified identity; `selftest.py` runs the real TLS context and
HTTP handler over `socketpair` (no bind, sandbox-safe).

Observed: `openssl verify` accepts both chains; selftest ALL PASS —
200 + identity page with the client cert, handshake `SSLError` without it.
Live-browser run confirmed by the user 2026-09-26 in **Safari**:
imported `client.p12`, opened `https://localhost:8443`, selected the test
cert → `LOGGED IN` page showing `POC TEST USER 010190-123N`, serial
`0F233ADA…1DC6CF`, matching the generated PKI exactly. Notably, this path
— unlike WebUSB — works outside Chromium entirely. One UX wrinkle
observed: macOS prompted for an admin password ("Safari wants to use the
System keychain") because the test identity lived in the admin-protected
System keychain; importing the `.p12` into the per-user *login* keychain
avoids that. A test-PKI artifact — with a real card the key never sits in
any keychain — but a reminder that client-side identity placement shapes
the login UX. Follow-up observations from the same run: Safari's picker
lists every usable identity (both copies of the test identity — System and
login keychains — plus the user's real FINEID card identity. Correction
to an earlier claim: this Mac has RefineID installed, and stock macOS only
bridges PIV cards natively (built-in CryptoTokenKit token), so the
card → browser path here is almost certainly provided by installed card
middleware, not the bare OS. The honest scope of "zero install" is
therefore: zero *browser-facing* software (no extension, no page-specific
agent) — but *some* card middleware (RefineID's, OpenSC-based, or vendor)
is still required on the client. Verifiable on the user's machine with
`pluginkit -m -p com.apple.ctk-tokens` in a normal terminal; selecting
the login-keychain copy triggers a
per-key authorization prompt ("Safari wants to sign using key…",
Allow/Always Allow), which is standard platform-owned consent for first
key use, distinct from the admin prompt for System-keychain items.

What this proves: the login ceremony needs no page JavaScript, no
extension, and no page-specific agent — the failure mode is the handshake
itself, so there is no weaker fallback page. It does still need card
middleware on the client (see correction above); "no RefineID software"
was overstated for the real-card case. What it does NOT prove:
document signing is still out of reach (no digest goes to the card under
page control), and a FINEID production would still need DVV/VRK CA trust,
certificate-policy/OID enforcement, revocation (CRL/OCSP), and client-side
card middleware on the user's machine.

## Extension route (verified 2026-09-26)

- **Direct card USB from an MV3 extension: NO.** `chrome.usb` was a
  Chrome-Apps-only API; that platform is deprecated and the API is absent
  from the MV3 extension API index
  (https://developer.chrome.com/docs/extensions/reference/api — no usb,
  serial, or bluetooth-device entries; the old reference URL 404s). An
  extension page calling `navigator.usb` is bound by the same WebUSB
  protected-class policy as any page. Extensions buy no USB privilege.
- **Extension as RAPP requester to a phone proxy: viable, two shapes.**
  (a) Extension → WebSocket → RAPP relay → phone needs the
  `relay-websocket-v1` transport profile (currently a reserved design
  target in `docs/protocols/rapp-v26.9.13.md`, section 16) plus relay
  infrastructure; no local install beyond the extension. (b) Extension →
  native-messaging host (reusing `refineid-core`/`refineid-unix`) → RAPP
  over the implemented `fi.refineid.stream.v1`/TCP or `apple-peer-v1`
  transports → phone; no relay, LAN/nearby only.
- **Simplest card-present desktop signing remains extension → native host
  → local PC/SC** (classic web-eid model), skipping the phone entirely
  when the reader is on the desk.
