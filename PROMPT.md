# Task: establish whether RefineID can run as one browser page

Build the smallest useful WebUSB feasibility prototype in this repository.
The user wants one self-contained page with everything bundled, zero application
backend, and no native helper, extension, CDN, or user-installed libraries.
Chrome alone is an acceptable initial browser. Firefox support must be reported
truthfully. Develop at http://localhost:8080; deployment would use HTTPS.

## First milestone: reader access

Create `public/index.html` containing all HTML, CSS, and JavaScript inline. No
framework, package installation, build system, or network requests are needed
for this milestone. Keep the UI small, readable, and useful for a hardware test.

Implement:

1. Capability checks for `isSecureContext` and `navigator.usb`, with distinct
   explanations for insecure origin and unsupported browser.
2. A user-clicked Connect button calling `requestDevice`. Permission cancellation
   is a normal outcome. Do not prompt automatically on page load.
3. Inspect configurations/interfaces/alternate settings and show class codes,
   endpoint types/directions, VID/PID, and interface numbers. Never read or show
   USB serial numbers. A permission grant is not proof of usable card access.
4. Select a configuration and explicitly attempt `claimInterface` on a relevant
   reader interface selected by the user. Report the exact failing stage and
   exception name. Record whether any alternate setting is protected. Do not
   claim success from device enumeration alone.
5. A Disconnect action and cleanup of claimed interfaces/devices after failure
   and on unplug. Prevent overlapping connection attempts and stale UI updates.
6. A copyable diagnostic report containing capabilities, descriptors, stages,
   and errors only. Do not persist reports or device permissions in app storage.

Do not send APDUs, perform USB data/control transfers, power a card, ask for PINs,
or sign anything in this milestone. Explain blocked access plainly; do not alter
browser flags, drivers, USB firmware, enterprise policy, or OS permissions.

## Known evidence to verify, not assumptions to work around

- HTTP localhost/loopback is a secure context; an arbitrary HTTP LAN URL is not.
  https://developer.mozilla.org/en-US/docs/Web/Security/Defenses/Secure_Contexts
- Current MDN compatibility data reports Chrome WebUSB support and no Firefox
  WebUSB implementation:
  https://github.com/mdn/browser-compat-data/blob/main/api/USB.json
- WebUSB protects Smart Card interface class 0x0B. Ordinary pages cannot claim
  that CCID interface even with user permission. The unrestricted exception is
  for Isolated Web Apps, which is outside this ordinary-page prototype:
  https://usb.spec.whatwg.org/#protected-interface-classes
  https://usb.spec.whatwg.org/#permissions-policy
- Vendor-specific interfaces might be accessible, but access alone does not
  establish a usable card protocol. OS driver ownership/permissions also matter:
  https://developer.chrome.com/docs/capabilities/build-for-webusb

## Useful local code, if the access gate is passed later

- `refineid-core/crates/ccid/src/engine.rs`: deterministic,
  event-driven CCID engine, separate from physical USB I/O.
- `refineid-core/crates/ccid/src/transport.rs` and
  `refineid-core/crates/apdu/src/transport.rs`: synchronous adapters;
  WebUSB promises require an async orchestration layer, not a direct substitution.
- Core crates `auth`, `pace`, `pkcs15`, and `sign` contain credential/card logic.
- `refineid-unix/crates/refineid-lib-core/src/sign/pades.rs` has
  in-memory PDF prepare/digest/finish operations that may be portable to WASM.

No existing WebUSB/WASM port has been demonstrated. Do not spend this run porting
the stack. A successful interface claim is the first gate; protocol compatibility
and actual signing are separate future milestones. WASM cannot bypass USB policy.

## Verification and completion

Use existing local browser/test tools where available. Check unsupported API,
insecure context, permission cancellation, protected-interface rejection,
already-claimed/busy device, unplug, reconnect, and duplicate clicks. Use test
doubles if hardware cannot be operated; clearly label those results as synthetic.
Do not claim physical reader verification unless observed. Hardware chooser
selection may need the user: finish independently testable work first.

Write a concise `RESEARCH.md`: verified browser rules with primary-source links,
what the prototype actually tested, remaining hardware test steps, and a go/no-go
decision for standard CCID readers versus an accessible vendor-specific reader.
Update `README.md` with usage. Deliver local files and a brief result; do not push,
deploy, install tooling, or start additional agents. Stop after this milestone.
