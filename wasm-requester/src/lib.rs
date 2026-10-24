// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! WASM requester: drives the `refineid-rapp` pairing and session flows
//! from JavaScript. The JS side owns the relay WebSocket and pumps frames
//! through this state machine; all RAPP state lives here.
//!
//! Proto limits: pairs live in memory only (a service-worker restart
//! drops them); operations ride in the next slice.

use std::collections::{BTreeMap, HashMap, HashSet};

use refineid_rapp::{
    BinaryFrame, EndpointRole, EstablishedEndpoint, ExplicitUserIntent, MANDATORY_PAIRING_SUITE,
    OFFER_ID_SIZE, OfferId, PAIRING_SECRET_SIZE, PairId, PairRecord, PairStore, PairStoreError,
    PairTombstone, PairingConfirmation, PairingHandshake, PairingOffer, PairingSecret, ProfileName,
    SessionAuthentication, SessionHandshake, TransportCandidate, WireValue,
    generate_pair_key_material,
};
use sha2::{Digest, Sha256};
use wasm_bindgen::prelude::*;

/// Fixed offer TTL for relay pairings (within `OFFER_TTL_MAX_MS`).
const OFFER_TTL_MS: u64 = 120_000;
/// Transport profile and candidate this requester always offers.
const RELAY_PROFILE: &str = "relay-websocket-v1";
const RELAY_CANDIDATE_ID: &str = "relay";
/// Profiles a browser requester asks to be granted.
const REQUESTED_PROFILES: [&str; 2] = [
    "fi.refineid.authentication.v1",
    "fi.refineid.document-signing.v1",
];

/// Debug-format an engine error for JS. Engine `Debug` impls redact
/// secrets (`PairingSecret([redacted])`), so this never leaks key
/// material.
fn engine_error<E: std::fmt::Debug>(error: E) -> String {
    format!("{error:?}")
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err(String::from("odd-length hex"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| String::from("bad hex")))
        .collect()
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(engine_error)?;
    Ok(bytes)
}

/// Six unbiased decimal digits from platform randomness.
fn generate_code() -> Result<String, String> {
    let mut digits = String::with_capacity(6);
    while digits.len() < 6 {
        let bytes = random_bytes::<16>()?;
        for byte in bytes {
            if digits.len() == 6 {
                break;
            }
            if byte < 250 {
                digits.push((b'0' + byte % 10) as char);
            }
        }
    }
    Ok(digits)
}

/// Keep ASCII digits only, truncate to six. Mirrors the platform
/// normalization so both peers derive identical values.
#[cfg(not(target_arch = "wasm32"))]
fn normalize_code(raw: &str) -> String {
    raw.chars().filter(|c| c.is_ascii_digit()).take(6).collect()
}

fn format_code(code: &str) -> String {
    format!("{} {}", &code[..3], &code[3..])
}

/// In-memory pair store. Proto only: nothing persists; revocation is a
/// local tombstone set.
#[derive(Debug, Default)]
struct MemoryPairStore {
    pairs: HashMap<PairId, PairRecord>,
    revoked: HashSet<PairId>,
}

#[derive(Debug)]
struct MemoryStoreError;

impl PairStore for MemoryPairStore {
    type Error = MemoryStoreError;

    fn load(&mut self, pair_id: PairId) -> Result<Option<PairRecord>, Self::Error> {
        // PairRecord is not Clone; the store lends through remove/insert
        // discipline at the call sites instead. This load moves the
        // record out; callers must put it back.
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

/// Noise initiator turn tracking (XX and KK requester side alike):
/// write, read, write, then complete.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    WriteFirst,
    ReadFirst,
    WriteSecond,
    Done,
}

impl Turn {
    fn advance(self) -> Self {
        match self {
            Self::WriteFirst => Self::ReadFirst,
            Self::ReadFirst => Self::WriteSecond,
            Self::WriteSecond | Self::Done => Self::Done,
        }
    }
}

enum PairingStage {
    Handshake(Box<PairingHandshake>, Turn),
    Confirm(Box<PairingConfirmation>),
}

struct SessionStage {
    handshake: Option<SessionHandshake>,
    auth: Option<SessionAuthentication>,
    turn: Turn,
    pending_write: Option<Vec<u8>>,
}

/// One browser requester: pairing state, stored pairs, one live session.
#[derive(Default)]
#[wasm_bindgen]
pub struct Requester {
    pairing: Option<PairingStage>,
    pairs: MemoryPairStore,
    pair_ids: Vec<PairId>,
    session: Option<SessionStage>,
    endpoint: Option<EstablishedEndpoint>,
}

#[wasm_bindgen]
impl Requester {
    /// Empty requester: no pairing, no pairs, no session.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Requester {
        Requester::default()
    }

    /// Start pairing: generate a code, build the fixed relay offer,
    /// open the handshake. Returns JSON `{code, formatted, offer_id}`.
    /// `now_ms` must come from a monotonic clock (`performance.now`).
    pub fn begin_pairing_core(
        &mut self,
        relay_url: String,
        _now_ms: f64,
    ) -> Result<String, String> {
        let code = generate_code()?;
        self.start_pairing(relay_url, &code)
    }

    /// Hex of the stored pair ids, for session selection UIs.
    pub fn pair_ids(&self) -> Vec<String> {
        self.pair_ids.iter().map(|id| hex(id.as_bytes())).collect()
    }

    /// Next handshake frame to send, or `undefined` when it is the
    /// peer's turn or the handshake is done.
    pub fn pairing_next_core(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(PairingStage::Handshake(handshake, turn)) = self.pairing.as_mut() else {
            return Ok(None);
        };
        match turn {
            Turn::WriteFirst | Turn::WriteSecond => {
                let frame = handshake.write_message().map_err(engine_error)?;
                *turn = turn.advance();
                Ok(Some(frame.into_bytes()))
            }
            Turn::ReadFirst | Turn::Done => Ok(None),
        }
    }

    /// Feed one handshake frame from the peer. Returns true when the
    /// handshake completes.
    pub fn pairing_feed_core(&mut self, bytes: &[u8]) -> Result<bool, String> {
        let Some(PairingStage::Handshake(handshake, turn)) = self.pairing.as_mut() else {
            return Err(String::from("no pairing handshake in flight"));
        };
        if *turn != Turn::ReadFirst {
            return Err(String::from("unexpected handshake frame"));
        }
        let frame = BinaryFrame::reconstruct(bytes.to_vec()).map_err(engine_error)?;
        handshake.read_message(&frame).map_err(engine_error)?;
        *turn = turn.advance();
        Ok(handshake.is_complete())
    }

    /// Advance a complete handshake into confirmation.
    pub fn pairing_confirm_begin_core(&mut self) -> Result<(), String> {
        let Some(stage) = self.pairing.take() else {
            return Err(String::from("no pairing in flight"));
        };
        let PairingStage::Handshake(handshake, _) = stage else {
            self.pairing = Some(stage);
            return Err(String::from("pairing already confirming"));
        };
        if !handshake.is_complete() {
            self.pairing = Some(PairingStage::Handshake(handshake, Turn::Done));
            return Err(String::from("handshake incomplete"));
        }
        let confirm = handshake.into_confirmation().map_err(engine_error)?;
        self.pairing = Some(PairingStage::Confirm(Box::new(confirm)));
        Ok(())
    }

    /// Our hello frame (carries the requested profiles).
    pub fn pairing_hello_core(
        &mut self,
        display_name: String,
        platform: String,
    ) -> Result<Vec<u8>, String> {
        let Some(PairingStage::Confirm(confirm)) = self.pairing.as_mut() else {
            return Err(String::from("pairing not confirming"));
        };
        confirm
            .send_hello(display_name, platform)
            .map_err(engine_error)
            .map(into_vec)
    }

    /// Read the peer hello. Returns JSON with the peer label, platform,
    /// and the grant set the proxy proposes.
    pub fn pairing_on_hello_core(&mut self, bytes: &[u8], now_ms: f64) -> Result<String, String> {
        let Some(PairingStage::Confirm(confirm)) = self.pairing.as_mut() else {
            return Err(String::from("pairing not confirming"));
        };
        let frame = BinaryFrame::reconstruct(bytes.to_vec()).map_err(engine_error)?;
        let hello = confirm
            .receive_hello(&frame, now_ms as u64)
            .map_err(engine_error)?;
        let requested: Vec<&str> = hello
            .requested_profiles
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .copied()
            .map(ProfileName::as_str)
            .collect();
        Ok(serde_json::json!({
            "display_name": hello.display_name,
            "platform": hello.platform,
            "requested_profiles": requested,
        })
        .to_string())
    }

    /// Our grant confirmation frame. Names are wire registry strings.
    pub fn pairing_confirm_core(&mut self, granted: Vec<String>) -> Result<Vec<u8>, String> {
        let Some(PairingStage::Confirm(confirm)) = self.pairing.as_mut() else {
            return Err(String::from("pairing not confirming"));
        };
        let mut profiles = Vec::with_capacity(granted.len());
        for name in &granted {
            profiles.push(
                ProfileName::parse(name)
                    .ok_or_else(|| String::from(&format!("unknown profile: {name}")))?,
            );
        }
        confirm
            .send_confirmation(profiles)
            .map_err(engine_error)
            .map(into_vec)
    }

    /// Read the peer grant confirmation. Returns the granted wire names.
    pub fn pairing_on_confirm_core(
        &mut self,
        bytes: &[u8],
        now_ms: f64,
    ) -> Result<Vec<String>, String> {
        let Some(PairingStage::Confirm(confirm)) = self.pairing.as_mut() else {
            return Err(String::from("pairing not confirming"));
        };
        let frame = BinaryFrame::reconstruct(bytes.to_vec()).map_err(engine_error)?;
        let granted = confirm
            .receive_confirmation(&frame, now_ms as u64)
            .map_err(engine_error)?;
        Ok(granted
            .iter()
            .map(|profile| profile.as_str().to_string())
            .collect())
    }

    /// Store the pair after both confirmations. Returns the pair id hex.
    /// `now_ms` stamps the record.
    pub fn pairing_finish_core(&mut self, now_ms: f64) -> Result<String, String> {
        let Some(stage) = self.pairing.take() else {
            return Err(String::from("no pairing in flight"));
        };
        let PairingStage::Confirm(confirm) = stage else {
            self.pairing = Some(stage);
            return Err(String::from("handshake not confirming yet"));
        };
        let record = confirm
            .into_pair_record(now_ms as u64)
            .map_err(engine_error)?;
        let id = record.pair_id();
        let id_hex = hex(id.as_bytes());
        self.pairs.insert(record).map_err(engine_error)?;
        self.pair_ids.push(id);
        self.pairing = None;
        Ok(id_hex)
    }

    /// Rendezvous token hex for the relay session join frame.
    pub fn pair_rendezvous(&self, pair_id_hex: &str) -> Result<String, String> {
        let id = parse_pair_id(pair_id_hex)?;
        self.pairs
            .pairs
            .get(&id)
            .map(|record| hex(record.rendezvous_token().as_bytes()))
            .ok_or_else(|| String::from("unknown pair"))
    }

    /// Open a session handshake as requester (explicit user intent).
    /// Returns the first handshake frame.
    pub fn session_begin_core(&mut self, pair_id_hex: &str) -> Result<Vec<u8>, String> {
        let id = parse_pair_id(pair_id_hex)?;
        let pair = self
            .pairs
            .pairs
            .get(&id)
            .ok_or_else(|| String::from("unknown pair"))?;
        let mut handshake = SessionHandshake::begin_requester(pair, ExplicitUserIntent::record())
            .map_err(engine_error)?;
        let frame = handshake.write_message().map_err(engine_error)?;
        self.session = Some(SessionStage {
            handshake: Some(handshake),
            auth: None,
            turn: Turn::ReadFirst,
            pending_write: None,
        });
        Ok(frame.into_bytes())
    }

    /// Feed the one session handshake frame (message 2). KK has no
    /// third message: a successful read completes the handshake and
    /// [`session_auth_begin_core`](Requester::session_auth_begin_core) runs next.
    /// Returns true when the handshake completes.
    pub fn session_feed_core(&mut self, bytes: &[u8]) -> Result<bool, String> {
        let Some(stage) = self.session.as_mut() else {
            return Err(String::from("no session in flight"));
        };
        let Some(handshake) = stage.handshake.as_mut() else {
            return Err(String::from("handshake already complete"));
        };
        if stage.turn != Turn::ReadFirst {
            return Err(String::from("unexpected handshake frame"));
        }
        let frame = BinaryFrame::reconstruct(bytes.to_vec()).map_err(engine_error)?;
        handshake.read_message(&frame).map_err(engine_error)?;
        stage.turn = Turn::Done;
        Ok(handshake.is_complete())
    }

    /// Next session handshake frame to send, if any.
    pub fn session_next_core(&mut self) -> Result<Option<Vec<u8>>, String> {
        let Some(stage) = self.session.as_mut() else {
            return Ok(None);
        };
        Ok(stage.pending_write.take())
    }

    /// Advance a complete handshake into authentication.
    pub fn session_auth_begin_core(&mut self) -> Result<(), String> {
        let Some(stage) = self.session.as_mut() else {
            return Err(String::from("no session in flight"));
        };
        let Some(handshake) = stage.handshake.take() else {
            return Err(String::from("already authenticating"));
        };
        if !handshake.is_complete() {
            stage.handshake = Some(handshake);
            return Err(String::from("handshake incomplete"));
        }
        let auth = handshake.into_authentication().map_err(engine_error)?;
        stage.auth = Some(auth);
        Ok(())
    }

    /// Our session.ready frame (fresh nonce inside).
    pub fn session_ready_core(&mut self) -> Result<Vec<u8>, String> {
        let Some(stage) = self.session.as_mut() else {
            return Err(String::from("no session in flight"));
        };
        let Some(auth) = stage.auth.as_mut() else {
            return Err(String::from("not authenticating yet"));
        };
        let nonce = random_bytes::<32>()?;
        auth.send_ready(nonce).map_err(engine_error).map(into_vec)
    }

    /// Read the peer ready and establish the endpoint.
    pub fn session_on_ready_core(&mut self, bytes: &[u8], now_ms: f64) -> Result<(), String> {
        let Some(stage) = self.session.as_mut() else {
            return Err(String::from("no session in flight"));
        };
        let Some(auth) = stage.auth.take() else {
            return Err(String::from("not authenticating yet"));
        };
        let frame = BinaryFrame::reconstruct(bytes.to_vec()).map_err(engine_error)?;
        let mut auth = auth;
        auth.receive_ready(&mut self.pairs, &frame, now_ms as u64)
            .map_err(engine_error)?;
        let endpoint = auth.into_established().map_err(engine_error)?;
        self.endpoint = Some(endpoint);
        Ok(())
    }

    /// True once a session endpoint is established.
    pub fn session_established(&self) -> bool {
        self.endpoint.is_some()
    }

    /// Drop session and endpoint state (the pair survives).
    pub fn session_close(&mut self) {
        if let Some(endpoint) = self.endpoint.as_mut() {
            endpoint.close_session();
        }
        self.session = None;
        self.endpoint = None;
    }

    /// Test hook (never compiled to WASM): public key halves as hex
    /// (`local/remote`), for cross-consistency checks. Public keys are
    /// not secrets.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn debug_pair_keys(&self, pair_id_hex: &str) -> Result<Vec<String>, String> {
        let id = parse_pair_id(pair_id_hex)?;
        self.pairs
            .pairs
            .get(&id)
            .map(|record| {
                vec![
                    hex(record.local_static_public()),
                    hex(record.remote_static_public()),
                ]
            })
            .ok_or_else(|| String::from("unknown pair"))
    }

    /// Test hook (never compiled to WASM): redacted record dump for
    /// host interop tests. Secrets stay redacted by the engine.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn debug_pair(&self, pair_id_hex: &str) -> Result<String, String> {
        let id = parse_pair_id(pair_id_hex)?;
        self.pairs
            .pairs
            .get(&id)
            .map(|record| format!("{record:?}"))
            .ok_or_else(|| String::from("unknown pair"))
    }

    /// Test hook (never compiled to WASM): pair with a fixed code so
    /// host tests can run the proxy side deterministically.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn begin_pairing_with_code(
        &mut self,
        relay_url: String,
        code: &str,
    ) -> Result<String, String> {
        let code = normalize_code(code);
        if code.len() != 6 {
            return Err(String::from("code must hold 6 digits"));
        }
        self.start_pairing(relay_url, &code)
    }
}

fn into_vec(frame: BinaryFrame) -> Vec<u8> {
    frame.into_bytes()
}

fn parse_pair_id(hex_text: &str) -> Result<PairId, String> {
    let bytes = unhex(hex_text)?;
    let array: [u8; 16] = bytes
        .try_into()
        .map_err(|_| String::from("pair id must be 16 bytes"))?;
    Ok(PairId::from_array(array))
}

impl Requester {
    /// Build the fixed relay offer for one code and open the handshake.
    fn start_pairing(&mut self, relay_url: String, code: &str) -> Result<String, String> {
        let secret_digest = Sha256::new()
            .chain_update(b"refineid-rapp-pairing-secret-v1:")
            .chain_update(code.as_bytes())
            .finalize();
        let id_digest = Sha256::new()
            .chain_update(b"refineid-rapp-offer-id-v1:")
            .chain_update(code.as_bytes())
            .finalize();
        let mut secret_bytes = [0u8; PAIRING_SECRET_SIZE];
        secret_bytes.copy_from_slice(&secret_digest);
        let mut id_bytes = [0u8; OFFER_ID_SIZE];
        id_bytes.copy_from_slice(&id_digest);
        let mut parameters = BTreeMap::new();
        parameters.insert(String::from("relay_url"), WireValue::Text(relay_url));
        let offer = PairingOffer::reconstruct(
            OfferId::from_array(id_bytes),
            PairingSecret::from_random_bytes(secret_bytes),
            vec![MANDATORY_PAIRING_SUITE.to_string()],
            REQUESTED_PROFILES.iter().map(ToString::to_string).collect(),
            vec![TransportCandidate {
                profile: String::from(RELAY_PROFILE),
                candidate_id: String::from(RELAY_CANDIDATE_ID),
                parameters,
            }],
            OFFER_TTL_MS,
        )
        .map_err(engine_error)?;
        let offer_hex = hex(&id_bytes);
        let keys = generate_pair_key_material().map_err(engine_error)?;
        let handshake =
            PairingHandshake::begin(EndpointRole::Requester, offer, RELAY_CANDIDATE_ID, keys)
                .map_err(|failure| engine_error(failure.into_parts().0))?;
        self.pairing = Some(PairingStage::Handshake(
            Box::new(handshake),
            Turn::WriteFirst,
        ));
        Ok(serde_json::json!({
            "code": code,
            "formatted": format_code(code),
            "offer_id": offer_hex,
        })
        .to_string())
    }
}
