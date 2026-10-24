# refineid-relay-poc

Untrusted WebSocket relay for the RAPP `relay-websocket-v1` transport
profile. Both peers dial in; legs join by token; the relay forwards
opaque frames. It holds no RAPP keys and parses nothing past the join
frame.

Profile spec: `refineid-core/docs/protocols/rapp-relay-websocket-v1.md`.

## Run

```sh
cargo run -p refineid-relay-poc -- --bind 127.0.0.1:9443
```

## Test

```sh
cargo test -p refineid-relay-poc     # 10 unit + 12 end-to-end leg tests
cargo fmt --check -p refineid-relay-poc
cargo clippy --all-targets -p refineid-relay-poc
```

End-to-end tests run over in-memory streams: no sockets, no binds.

## PoC limits (do not deploy as-is)

- No TLS termination: put a TLS proxy in front so browsers see `wss`.
- Rate limits and caps are fixed defaults, not abuse-hardened.
- Relay-to-peer timing/size metadata is inherently visible (spec 17).
- Production home for this code is undecided; it lives here for the
  prototype phase.
