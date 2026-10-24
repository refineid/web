// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Host-side interop: the WASM wrapper's requester flows against the
//! engine's proxy role, proving the fixed relay offer template and the
//! code derivation interoperate with `refineid-rapp` itself.

use std::collections::{BTreeMap, HashMap, HashSet};

use refineid_rapp::{
    BinaryFrame, EndpointRole, MANDATORY_PAIRING_SUITE, OFFER_ID_SIZE, OfferId,
    PAIRING_SECRET_SIZE, PairId, PairRecord, PairStore, PairStoreError, PairTombstone,
    PairingHandshake, PairingOffer, PairingSecret, ProfileName, SessionHandshake,
    TransportCandidate, WireValue, generate_pair_key_material,
};
use refineid_wasm_requester::Requester;
use sha2::{Digest, Sha256};

const RELAY_URL: &str = "wss://relay.example.invalid";
const CODE: &str = "123456";

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
        let id = record.pair_id();
        if self.pairs.contains_key(&id) || self.revoked.contains(&id) {
            return Err(PairStoreError::IdentifierAlreadyUsed);
        }
        self.pairs.insert(id, record);
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

/// Rebuild the proxy side of the fixed relay offer from the same code.
/// This duplicates the wrapper's 3-line derivation on purpose: any drift
/// fails the handshake, which is exactly what this test guards.
fn proxy_offer() -> PairingOffer {
    let secret_digest = Sha256::new()
        .chain_update(b"refineid-rapp-pairing-secret-v1:")
        .chain_update(CODE)
        .finalize();
    let id_digest = Sha256::new()
        .chain_update(b"refineid-rapp-offer-id-v1:")
        .chain_update(CODE)
        .finalize();
    let mut secret_bytes = [0u8; PAIRING_SECRET_SIZE];
    secret_bytes.copy_from_slice(&secret_digest);
    let mut id_bytes = [0u8; OFFER_ID_SIZE];
    id_bytes.copy_from_slice(&id_digest);
    let mut parameters = BTreeMap::new();
    parameters.insert(
        String::from("relay_url"),
        WireValue::Text(String::from(RELAY_URL)),
    );
    PairingOffer::reconstruct(
        OfferId::from_array(id_bytes),
        PairingSecret::from_random_bytes(secret_bytes),
        vec![MANDATORY_PAIRING_SUITE.to_string()],
        vec![
            String::from("fi.refineid.authentication.v1"),
            String::from("fi.refineid.document-signing.v1"),
        ],
        vec![TransportCandidate {
            profile: String::from("relay-websocket-v1"),
            candidate_id: String::from("relay"),
            parameters,
        }],
        120_000,
    )
    .expect("valid fixed offer")
}

fn frame(bytes: Vec<u8>) -> BinaryFrame {
    BinaryFrame::reconstruct(bytes).expect("bounded test frame")
}

/// Full pairing: wrapper requester against engine proxy, then both hold
/// the same pair.
fn paired() -> (Requester, PairRecord) {
    let mut requester = Requester::new();
    let started = requester
        .begin_pairing_with_code(String::from(RELAY_URL), CODE)
        .expect("offer builds");
    assert!(
        started.contains("\"code\":\"123456\""),
        "code echoed: {started}"
    );
    assert!(
        started.contains("\"formatted\":\"123 456\""),
        "formatted: {started}"
    );

    let proxy_keys = generate_pair_key_material().expect("proxy keys");
    let mut proxy =
        PairingHandshake::begin(EndpointRole::Proxy, proxy_offer(), "relay", proxy_keys)
            .expect("proxy handshake opens");

    // Noise XX: requester writes, proxy reads, proxy writes, requester
    // reads, requester writes, proxy reads.
    let m1 = requester
        .pairing_next_core()
        .expect("m1 builds")
        .expect("m1 present");
    proxy.read_message(&frame(m1)).expect("proxy reads m1");
    let m2 = proxy.write_message().expect("m2 builds").into_bytes();
    assert!(!requester.pairing_feed_core(&m2).expect("m2 feeds"));
    let m3 = requester
        .pairing_next_core()
        .expect("m3 builds")
        .expect("m3 present");
    proxy.read_message(&frame(m3)).expect("proxy reads m3");
    assert!(proxy.is_complete());

    requester
        .pairing_confirm_begin_core()
        .expect("requester confirms");
    let requester_hello = requester
        .pairing_hello_core(String::from("test-browser"), String::from("test-os"))
        .expect("hello");
    let mut confirmation = proxy.into_confirmation().expect("proxy confirmation opens");
    let proxy_hello = confirmation
        .send_hello(String::from("test-phone"), String::from("test-os"))
        .expect("proxy hello")
        .into_bytes();
    let peer_json = requester
        .pairing_on_hello_core(&proxy_hello, 2000.0)
        .expect("hello reads");
    assert!(peer_json.contains("test-phone"), "peer label: {peer_json}");
    confirmation
        .receive_hello(&frame(requester_hello), 2000)
        .expect("proxy reads hello");

    let grants = vec![
        String::from("fi.refineid.authentication.v1"),
        String::from("fi.refineid.document-signing.v1"),
    ];
    let requester_confirm = requester
        .pairing_confirm_core(grants)
        .expect("confirm sends");
    let parsed = [ProfileName::Authentication, ProfileName::DocumentSigning].to_vec();
    let proxy_confirm = confirmation
        .send_confirmation(parsed)
        .expect("proxy confirm")
        .into_bytes();
    confirmation
        .receive_confirmation(&frame(requester_confirm), 3000)
        .expect("proxy reads");
    let confirmed = requester
        .pairing_on_confirm_core(&proxy_confirm, 3000.0)
        .expect("confirm reads");
    assert_eq!(confirmed.len(), 2);

    let pair_id_hex = requester.pairing_finish_core(4000.0).expect("pair stores");
    let proxy_record = confirmation
        .into_pair_record(4000)
        .expect("proxy pair stores");
    assert_eq!(requester.pair_ids(), vec![pair_id_hex]);
    (requester, proxy_record)
}

#[test]
fn template_offer_works_engine_only() {
    use refineid_rapp::{EndpointRole, ExplicitUserIntent, SessionHandshake};

    let mut requester = PairingHandshake::begin(
        EndpointRole::Requester,
        proxy_offer(),
        "relay",
        generate_pair_key_material().expect("keys"),
    )
    .expect("requester pairing starts");
    let mut proxy = PairingHandshake::begin(
        EndpointRole::Proxy,
        proxy_offer(),
        "relay",
        generate_pair_key_material().expect("keys"),
    )
    .expect("proxy pairing starts");
    let m1 = requester.write_message().expect("m1").into_bytes();
    proxy.read_message(&frame(m1)).expect("proxy reads m1");
    let m2 = proxy.write_message().expect("m2").into_bytes();
    requester
        .read_message(&frame(m2))
        .expect("requester reads m2");
    let m3 = requester.write_message().expect("m3").into_bytes();
    proxy.read_message(&frame(m3)).expect("proxy reads m3");
    assert!(requester.is_complete() && proxy.is_complete());

    let mut requester = requester.into_confirmation().expect("requester confirm");
    let mut proxy = proxy.into_confirmation().expect("proxy confirm");
    let requester_hello = requester
        .send_hello(String::from("r"), String::from("o"))
        .expect("hello")
        .into_bytes();
    let proxy_hello = proxy
        .send_hello(String::from("p"), String::from("o"))
        .expect("hello")
        .into_bytes();
    requester
        .receive_hello(&frame(proxy_hello), 100)
        .expect("requester hello reads");
    proxy
        .receive_hello(&frame(requester_hello), 100)
        .expect("proxy hello reads");
    let grants = vec![ProfileName::Authentication, ProfileName::DocumentSigning];
    let requester_confirm = requester
        .send_confirmation(grants.clone())
        .expect("confirm")
        .into_bytes();
    let proxy_confirm = proxy
        .send_confirmation(grants)
        .expect("confirm")
        .into_bytes();
    requester
        .receive_confirmation(&frame(proxy_confirm), 200)
        .expect("requester confirm reads");
    proxy
        .receive_confirmation(&frame(requester_confirm), 200)
        .expect("proxy confirm reads");
    let requester_record = requester.into_pair_record(300).expect("requester record");
    let proxy_record = proxy.into_pair_record(300).expect("proxy record");

    let mut requester =
        SessionHandshake::begin_requester(&requester_record, ExplicitUserIntent::record())
            .expect("session starts");
    let mut proxy = SessionHandshake::begin_proxy(&proxy_record).expect("proxy session starts");
    let m1 = requester.write_message().expect("m1").into_bytes();
    proxy.read_message(&frame(m1)).expect("proxy reads m1");
    let m2 = proxy.write_message().expect("m2").into_bytes();
    requester
        .read_message(&frame(m2))
        .expect("requester reads m2");
    assert!(requester.is_complete() && proxy.is_complete());
}

#[test]
fn pairing_produces_equal_pairs_both_sides() {
    let (requester, proxy_record) = paired();
    let pair_id_hex = &requester.pair_ids()[0];
    let rendezvous = requester
        .pair_rendezvous(pair_id_hex)
        .expect("rendezvous reads");
    assert_eq!(rendezvous.len(), 32, "16-byte token as hex");
    let expected: String = proxy_record
        .rendezvous_token()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(rendezvous, expected, "both sides derive the same token");
}

#[test]
fn session_establishes_both_sides() {
    let (mut requester, proxy_record) = paired();
    let pair_id_hex = requester.pair_ids()[0].clone();

    // Noise KK is two messages: the requester writes m1, the proxy
    // reads m1 and writes m2, the requester reads m2 and completes.
    // No third message exists.
    let mut proxy = SessionHandshake::begin_proxy(&proxy_record).expect("proxy session opens");
    let m1 = requester
        .session_begin_core(&pair_id_hex)
        .expect("session opens");
    proxy.read_message(&frame(m1)).expect("proxy reads m1");
    let m2 = proxy.write_message().expect("m2 builds").into_bytes();
    assert!(
        requester.session_feed_core(&m2).expect("m2 feeds"),
        "KK completes on m2"
    );
    assert!(
        requester.session_next_core().expect("next reads").is_none(),
        "no m3 is staged"
    );

    requester.session_auth_begin_core().expect("requester auth");
    let mut proxy_auth = proxy.into_authentication().expect("proxy auth");
    let requester_ready = requester.session_ready_core().expect("ready sends");
    let mut proxy_store = TestStore::default();
    let nonce = [9u8; 32];
    let proxy_ready = proxy_auth
        .send_ready(nonce)
        .expect("proxy ready")
        .into_bytes();
    proxy_auth
        .receive_ready(&mut proxy_store, &frame(requester_ready), 5000)
        .expect("proxy reads ready");
    requester
        .session_on_ready_core(&proxy_ready, 5000.0)
        .expect("requester reads ready");
    assert!(requester.session_established());
    let _established = proxy_auth.into_established().expect("proxy establishes");
}
