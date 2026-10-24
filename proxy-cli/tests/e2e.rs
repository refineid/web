// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! End to end: an engine requester pairs and opens a session with the
//! proxy-cli flows through a real in-process relay over duplex legs.
//! No sockets, no binds; the relay matches and forwards for real.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use refineid_proxy_cli::{Leg, ScriptedApprover, fixed_offer, run_pairing, run_session};
use refineid_rapp::{
    BinaryFrame, EndpointRole, ExplicitUserIntent, PairId, PairRecord, PairStore, PairStoreError,
    PairTombstone, PairingHandshake, ProfileName, SessionHandshake, generate_pair_key_material,
};
use refineid_relay_poc::protocol::{Purpose, Role};
use refineid_relay_poc::server::{Config, Relay, handle_leg};
use tokio_tungstenite::WebSocketStream;

type Client = WebSocketStream<tokio::io::DuplexStream>;

const DUMMY_PEER: &str = "127.0.0.1:1";

fn test_relay() -> Arc<Relay> {
    Arc::new(Relay::new(Config::default()))
}

async fn connect(relay: &Arc<Relay>) -> Client {
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    let peer: std::net::SocketAddr = DUMMY_PEER.parse().unwrap();
    tokio::spawn(handle_leg(relay.clone(), server_io, peer));
    let (client, _) = tokio_tungstenite::client_async("ws://relay/", client_io)
        .await
        .unwrap();
    client
}

fn frame(bytes: Vec<u8>) -> BinaryFrame {
    BinaryFrame::reconstruct(bytes).unwrap()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug, Default)]
struct TestStore {
    pairs: HashMap<PairId, PairRecord>,
    revoked: HashSet<PairId>,
}

#[derive(Debug)]
struct StoreError;

impl PairStore for TestStore {
    type Error = StoreError;

    fn load(&mut self, pair_id: PairId) -> Result<Option<PairRecord>, Self::Error> {
        Ok(self.pairs.remove(&pair_id))
    }

    fn insert(&mut self, record: PairRecord) -> Result<(), PairStoreError<Self::Error>> {
        self.pairs.insert(record.pair_id(), record);
        Ok(())
    }

    fn revoke(&mut self, tombstone: PairTombstone) -> Result<(), PairStoreError<Self::Error>> {
        self.pairs.remove(&tombstone.pair_id);
        self.revoked.insert(tombstone.pair_id);
        Ok(())
    }

    fn is_revoked(&mut self, pair_id: PairId) -> Result<bool, Self::Error> {
        Ok(self.revoked.contains(&pair_id))
    }
}

/// Drive the engine requester side of pairing over one joined leg.
async fn requester_pair(leg: &mut Leg<Client>, offer: refineid_rapp::PairingOffer) -> PairRecord {
    let keys = generate_pair_key_material().unwrap();
    let mut requester =
        PairingHandshake::begin(EndpointRole::Requester, offer, "relay", keys).unwrap();
    leg.send(requester.write_message().unwrap().into_bytes())
        .await
        .unwrap();
    requester
        .read_message(&frame(leg.recv().await.unwrap()))
        .unwrap();
    leg.send(requester.write_message().unwrap().into_bytes())
        .await
        .unwrap();
    assert!(requester.is_complete());
    let mut confirm = requester.into_confirmation().unwrap();
    let hello = confirm
        .send_hello(String::from("e2e"), String::from("test"))
        .unwrap();
    leg.send(hello.into_bytes()).await.unwrap();
    let peer = frame(leg.recv().await.unwrap());
    confirm.receive_hello(&peer, 100).unwrap();
    let grants = vec![ProfileName::Authentication, ProfileName::DocumentSigning];
    let ours = confirm.send_confirmation(grants).unwrap();
    leg.send(ours.into_bytes()).await.unwrap();
    let theirs = frame(leg.recv().await.unwrap());
    confirm.receive_confirmation(&theirs, 200).unwrap();
    confirm.into_pair_record(300).unwrap()
}

/// Drive the engine requester side of one session over a joined leg.
async fn requester_session(leg: &mut Leg<Client>, record: &PairRecord) {
    let mut requester =
        SessionHandshake::begin_requester(record, ExplicitUserIntent::record()).unwrap();
    leg.send(requester.write_message().unwrap().into_bytes())
        .await
        .unwrap();
    requester
        .read_message(&frame(leg.recv().await.unwrap()))
        .unwrap();
    assert!(requester.is_complete());
    let mut auth = requester.into_authentication().unwrap();
    let ready = auth.send_ready([7u8; 32]).unwrap();
    leg.send(ready.into_bytes()).await.unwrap();
    let peer = frame(leg.recv().await.unwrap());
    let mut store = TestStore::default();
    auth.receive_ready(&mut store, &peer, 400).unwrap();
    let _endpoint = auth.into_established().unwrap();
}

#[tokio::test]
async fn pair_then_login_over_relay() {
    let relay = test_relay();
    let clock = Instant::now();
    let (offer, offer_hex) = fixed_offer("ws://relay.example.invalid", "123-456").unwrap();
    assert_eq!(offer_hex.len(), 64, "32-byte offer id as hex");

    let (requester_offer, requester_hex) =
        fixed_offer("ws://relay.example.invalid", "123-456").unwrap();
    assert_eq!(
        requester_hex, offer_hex,
        "both sides derive the same offer id"
    );
    let mut proxy_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Pairing,
        Role::Proxy,
        &offer_hex,
    )
    .await
    .unwrap();
    let mut requester_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Pairing,
        Role::Requester,
        &offer_hex,
    )
    .await
    .unwrap();
    proxy_leg.await_joined().await.unwrap();
    requester_leg.await_joined().await.unwrap();
    let proxy_pair = tokio::spawn(async move {
        let mut approver = ScriptedApprover {
            pair_yes: true,
            session_yes: true,
            seen: vec![],
        };
        let record = run_pairing(&mut proxy_leg, offer, "e2e-proxy", &mut approver, &clock)
            .await
            .unwrap();
        (record, approver)
    });
    let requester_record = requester_pair(&mut requester_leg, requester_offer).await;
    let (proxy_record, approver) = proxy_pair.await.unwrap();
    assert_eq!(requester_record.pair_id(), proxy_record.pair_id());
    assert!(
        approver
            .seen
            .iter()
            .any(|seen| seen.starts_with("pair e2e ")),
        "{:?}",
        approver.seen
    );

    let rendezvous = hex(proxy_record.rendezvous_token().as_bytes());
    let mut proxy_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Session,
        Role::Proxy,
        &rendezvous,
    )
    .await
    .unwrap();
    let mut requester_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Session,
        Role::Requester,
        &rendezvous,
    )
    .await
    .unwrap();
    proxy_leg.await_joined().await.unwrap();
    requester_leg.await_joined().await.unwrap();
    let proxy_session = tokio::spawn(async move {
        let mut approver = ScriptedApprover {
            pair_yes: true,
            session_yes: true,
            seen: vec![],
        };
        let (pair_hex, _endpoint) =
            run_session(&mut proxy_leg, &proxy_record, &mut approver, &clock)
                .await
                .unwrap();
        (pair_hex, approver)
    });
    requester_session(&mut requester_leg, &requester_record).await;
    let (pair_hex, approver) = proxy_session.await.unwrap();
    assert_eq!(pair_hex, hex(requester_record.pair_id().as_bytes()));
    assert!(
        approver
            .seen
            .iter()
            .any(|seen| seen.starts_with("session ")),
        "{:?}",
        approver.seen
    );
}

#[tokio::test]
async fn declined_login_stops_before_ready() {
    let relay = test_relay();
    let clock = Instant::now();
    let (offer, offer_hex) = fixed_offer("ws://relay.example.invalid", "654321").unwrap();

    let (requester_offer, _) = fixed_offer("ws://relay.example.invalid", "654321").unwrap();
    let mut proxy_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Pairing,
        Role::Proxy,
        &offer_hex,
    )
    .await
    .unwrap();
    let mut requester_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Pairing,
        Role::Requester,
        &offer_hex,
    )
    .await
    .unwrap();
    proxy_leg.await_joined().await.unwrap();
    requester_leg.await_joined().await.unwrap();
    let proxy_pair = tokio::spawn(async move {
        let mut approver = ScriptedApprover {
            pair_yes: true,
            session_yes: false,
            seen: vec![],
        };
        run_pairing(&mut proxy_leg, offer, "e2e-proxy", &mut approver, &clock)
            .await
            .unwrap()
    });
    let requester_record = requester_pair(&mut requester_leg, requester_offer).await;
    let proxy_record = proxy_pair.await.unwrap();

    // Requester drives its side; the proxy declines before sending ready,
    // so the requester never completes. Drive only up to our send.
    let rendezvous = hex(requester_record.rendezvous_token().as_bytes());
    let mut proxy_leg = Leg::dial(
        connect(&relay).await,
        Purpose::Session,
        Role::Proxy,
        &rendezvous,
    )
    .await
    .unwrap();
    let mut leg = Leg::dial(
        connect(&relay).await,
        Purpose::Session,
        Role::Requester,
        &rendezvous,
    )
    .await
    .unwrap();
    proxy_leg.await_joined().await.unwrap();
    leg.await_joined().await.unwrap();
    let proxy_session = tokio::spawn(async move {
        let mut approver = ScriptedApprover {
            pair_yes: true,
            session_yes: false,
            seen: vec![],
        };
        run_session(&mut proxy_leg, &proxy_record, &mut approver, &clock).await
    });
    let mut requester =
        SessionHandshake::begin_requester(&requester_record, ExplicitUserIntent::record()).unwrap();
    leg.send(requester.write_message().unwrap().into_bytes())
        .await
        .unwrap();
    requester
        .read_message(&frame(leg.recv().await.unwrap()))
        .unwrap();
    let mut auth = requester.into_authentication().unwrap();
    leg.send(auth.send_ready([7u8; 32]).unwrap().into_bytes())
        .await
        .unwrap();
    let outcome = proxy_session.await.unwrap();
    assert!(
        matches!(
            outcome,
            Err(refineid_proxy_cli::ProxyError::Declined("login"))
        ),
        "{outcome:?}"
    );
}

#[test]
fn offer_codes_normalize_and_validate() {
    assert!(fixed_offer("wss://r.invalid", "123456").is_ok());
    assert!(fixed_offer("wss://r.invalid", "123 456").is_ok());
    assert!(fixed_offer("wss://r.invalid", "12345").is_err());
    assert!(fixed_offer("wss://r.invalid", "abcdef").is_err());
}
