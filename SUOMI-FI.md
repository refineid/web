# Why suomi.fi login from a clean-Chrome extension is blocked

Verdict: the certificate-card login at `kortti.tunnistautuminen.suomi.fi`
authenticates inside the TLS handshake, and desktop Chrome gives
extensions zero participation in that handshake. No extension code,
page script, relay, or proxy on our side changes either half. What
follows is the evidence, all reproducible.

## 1. The server demands TLS-layer client authentication

The sibling Android team recorded the live exchange on a physical
device (refineid-android ADR 0036): after the server handshake, the
WebView receives a client-certificate request, the card signs, and
Suomi.fi returns the identified holder. The gate is the TLS
CertificateVerify, not an application form.

Direct probes from here agree with the shape (TLS 1.2 ECDHE-RSA,
server chain to Telia Root CA v2):

```sh
echo | openssl s_client -connect kortti.tunnistautuminen.suomi.fi:443 \
  -servername kortti.tunnistautuminen.suomi.fi -msg
# ServerHello, Certificate, ServerKeyExchange, ServerHelloDone.
# No CertificateRequest on the initial handshake.
```

```sh
printf 'GET / HTTP/1.1\r\nHost: kortti.tunnistautuminen.suomi.fi\r\n...' | \
  openssl s_client -connect kortti.tunnistautuminen.suomi.fi:443 -ign_eof
# HTTP/1.1 302 Found
# Location: https://tunnistautuminen.suomi.fi/sivut/info/tietoapalvelusta/
```

So the certificate demand is flow-triggered (renegotiation at the
auth step once a service session exists), not on bare `/`. Either
way it happens inside TLS, below everything an extension or page can
see: page JavaScript runs only after the handshake that needs the key
(the timing catch-22, refineid-android
`doc/research/client-certificate-delegation-and-web-limitations.md`,
section 5).

## 2. Desktop Chrome extensions cannot join a TLS handshake

The one API built for exactly this says, verbatim (developer.chrome.com,
`.../extensions/reference/api/certificateProvider/`):

> **Important:** This API works **only on ChromeOS**.

Availability line on the same page: `Chrome 46+ ChromeOS only`.
The neighboring key APIs are likewise gated:

- `chrome.enterprise.platformKeys`: `ChromeOS only`, requires policy.
- `chrome.platformKeys`: `Chrome 45+ ChromeOS only`.

What remains on desktop:

- `webRequest` / `declarativeNetRequest` operate on HTTP, above TLS;
  neither sees the handshake nor selects certificates.
- Client certificates come from the OS store (Keychain on macOS).
  Selection is the native picker or the `AutoSelectCertificateForUrls`
  enterprise policy (machine policy files, not extensions), and only
  certificates matching the server's request are eligible.
- Native Messaging cannot interpose: Chrome performs the handshake
  internally against OS-store identities.

## 3. Key substitution cannot bridge the gap

The card private key is non-exportable (`CKA_EXTRACTABLE = false`).
Presenting the card certificate while signing with any other key
fails CertificateVerify mathematically and aborts the handshake
(same Android research doc, section 2). A proxy that answers
challenges without channel binding is, to the verifier,
indistinguishable from a relay attacker (section 4).

## 4. What would work, and what each costs

- **OS smartcard provider** (macOS TokenD, Windows minidriver,
  Linux PKCS#11): exposes the key to Chrome's TLS stack. This is a
  RefineID install, contradicting the clean-machine goal.
- **`chrome.certificateProvider`**: the designed extension hook,
  ChromeOS-only by vendor decision.
- **WASM TLS through a gateway**: a WASM TLS client could handshake
  via a relay-to-TCP bridge with RAPP-signed CertificateVerify, but
  the login session would live in the extension, not the user's
  suomi.fi tab; moving it over is fragile session handling against a
  service we do not operate.
- **Operator cooperation**: DVV accepting a RAPP-based eID means is
  a business track, not an engineering one.

## 5. Edge changes nothing for the extension route

Microsoft Edge is Chromium, and Microsoft's own extension API support
table lists `certificateProvider` as `ChromeOS only`, exactly like
Chrome. Same wall, same section 2.

Edge on Windows _does_ reach FINEID cards through the OS stack: the
browser calls into CNG, Windows invokes the registered smart-card
minidriver, the minidriver exchanges APDUs with the card. That is the
legacy installed route (refineid-windows already ships
`refineid-minidriver` for it), not the clean-machine extension route.

## 6. Conclusion

The Chrome-extension route to suomi.fi is blocked at the platform
layer on both ends: the server authenticates in TLS, and desktop
Chrome admits no extension into TLS. The productive path stays the
demo in `LOGIN.md` (extension + relay + approving proxy proving the
ceremony on our own service) plus the iPhone relay transport so a
card-backed proxy can execute `BrowserAuthenticate` there.
