# refineid-proxy-cli (demo)

Terminal RAPP proxy for the `relay-websocket-v1` profile: pairs from a
6-digit code, runs one Noise-authenticated session, and asks the human
to approve the login before completing the ready exchange. This is the
counterpart the browser extension pairs with until a card-backed proxy
(the phone app) speaks the relay transport.

Apache-2.0, like the rest of this repository.

## Run

```sh
cd proxy-cli
cargo run -- serve --code 123456 --relay ws://127.0.0.1:9443
```

One-shot by design: pairs live in memory only and die with the
process. Re-pair for every run; codes are cheap. `--auto-approve`
answers yes to every prompt; testing only, never default.

## Test

```sh
cd proxy-cli
cargo test     # pair+login over a real in-process relay
cargo fmt --check
cargo clippy --all-targets
```

End-to-end tests run an engine requester against these proxy flows
through the real relay matcher over in-memory legs: no sockets.

## Demo trust model

No card is attached. Approval proves pair possession over a mutually
authenticated Noise channel plus explicit human consent, not card
identity. Real logins execute `CardOperation::BrowserAuthenticate`
on a card-backed proxy; this CLI stops at the approved session.
