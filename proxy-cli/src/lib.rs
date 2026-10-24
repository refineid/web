// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Demo RAPP proxy: pairs over the relay from a 6-digit code, runs one
//! Noise-authenticated session, and asks the human to approve the login
//! before completing the ready exchange. No card is attached: approval
//! proves pair possession plus explicit consent, not card identity.
//!
//! One-shot by design: pairs live in memory only and die with the
//! process. Re-pair for every run; codes are cheap.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use futures::{Sink, SinkExt, Stream, StreamExt};
use refineid_rapp::{
    BinaryFrame, EndpointRole, EstablishedEndpoint, MANDATORY_PAIRING_SUITE, OFFER_ID_SIZE,
    PAIRING_SECRET_SIZE, PairId, PairRecord, PairStore, PairStoreError, PairTombstone,
    PairingHandshake, PairingOffer, PairingSecret, ProfileName, SessionHandshake,
    TransportCandidate, WireValue, generate_pair_key_material,
};
use refineid_relay_poc::protocol::{Join, Purpose, Role, encode_join, encode_joined};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

const RELAY_CANDIDATE_ID: &str = "relay";
const OFFER_TTL_MS: u64 = 120_000;
const STEP_TIMEOUT_SECS: u64 = 120;

/// What failed, in one printable value. Engine errors keep their `Debug`.
#[derive(Debug)]
pub enum ProxyError {
    /// The human declined at an approval prompt.
    Declined(&'static str),
    /// Relay leg broke: transport, join, or unexpected close.
    Leg(String),
    /// Engine refused a pairing step.
    Pairing(String),
    /// Engine refused a session step.
    Session(String),
    /// Local failure: randomness, code shape, clock.
    Local(String),
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declined(what) => write!(formatter, "declined by user: {what}"),
            Self::Leg(detail) => write!(formatter, "relay leg: {detail}"),
            Self::Pairing(detail) => write!(formatter, "pairing: {detail}"),
            Self::Session(detail) => write!(formatter, "session: {detail}"),
            Self::Local(detail) => write!(formatter, "local: {detail}"),
        }
    }
}

impl std::error::Error for ProxyError {}

/// Peer identity as shown at the pairing approval prompt. Public labels
/// only; the channel is already Noise-authenticated at this point.
#[derive(Debug, Clone)]
pub struct PeerHello {
    /// Requester application label from its hello frame.
    pub display_name: String,
    /// Requester platform label from its hello frame.
    pub platform: String,
    /// Profiles the requester asked for, as wire registry strings.
    pub requested: Vec<String>,
}

/// Human consent hooks. Every approval defaults to *no*.
pub trait Approver {
    /// Approve pairing with this peer for exactly these grants.
    fn approve_pairing(&mut self, peer: &PeerHello, granted: &[String]) -> bool;
    /// Approve completing one login session for this pair id hex.
    fn approve_session(&mut self, pair_id_hex: &str) -> bool;
}

/// Stdin `y/N` approver for the terminal.
#[derive(Debug, Default)]
pub struct ConsoleApprover;

impl ConsoleApprover {
    fn ask(prompt: &str) -> bool {
        use std::io::{Write, stdin, stdout};
        print!("{prompt} [y/N] ");
        stdout().flush().ok();
        let mut line = String::new();
        stdin().read_line(&mut line).is_ok() && line.trim().eq_ignore_ascii_case("y")
    }
}

impl Approver for ConsoleApprover {
    fn approve_pairing(&mut self, peer: &PeerHello, granted: &[String]) -> bool {
        println!(
            "Pairing request from {} ({})",
            peer.display_name, peer.platform
        );
        println!("Requested profiles: {}", peer.requested.join(", "));
        println!("Grants to confirm: {}", granted.join(", "));
        Self::ask("Pair with this browser?")
    }

    fn approve_session(&mut self, pair_id_hex: &str) -> bool {
        println!("Login request for pair {pair_id_hex}");
        Self::ask("Approve this login?")
    }
}

/// Fixed answers for tests. Records what it approved.
#[derive(Debug, Default)]
pub struct ScriptedApprover {
    /// Whether pairing prompts answer yes.
    pub pair_yes: bool,
    /// Whether session prompts answer yes.
    pub session_yes: bool,
    /// Prompts seen, in order.
    pub seen: Vec<String>,
}

impl Approver for ScriptedApprover {
    fn approve_pairing(&mut self, peer: &PeerHello, granted: &[String]) -> bool {
        self.seen
            .push(format!("pair {} {}", peer.display_name, granted.join(",")));
        self.pair_yes
    }

    fn approve_session(&mut self, pair_id_hex: &str) -> bool {
        self.seen.push(format!("session {pair_id_hex}"));
        self.session_yes
    }
}

/// One joined relay leg over any bidirectional WebSocket stream.
pub struct Leg<S> {
    inner: S,
}

type WsError = tokio_tungstenite::tungstenite::Error;

impl<S> Leg<S>
where
    S: Stream<Item = Result<Message, WsError>> + Sink<Message, Error = WsError> + Unpin,
{
    /// Dial one leg: send the join frame as the first message. The leg
    /// is not usable until [`Leg::await_joined`] sees the joined signal.
    /// Split so two legs can dial before either waits for the match.
    pub async fn dial(
        mut inner: S,
        purpose: Purpose,
        role: Role,
        token_hex: &str,
    ) -> Result<Self, ProxyError> {
        let token = unhex(token_hex)?;
        let frame = encode_join(&Join {
            purpose,
            role,
            token,
        });
        inner
            .send(Message::Binary(frame.into()))
            .await
            .map_err(|error| ProxyError::Leg(format!("join send failed: {error}")))?;
        Ok(Self { inner })
    }

    /// Wait for exactly the joined signal before any RAPP frame flows.
    pub async fn await_joined(&mut self) -> Result<(), ProxyError> {
        let first = Self::recv_inner(&mut self.inner).await?;
        if first != encode_joined() {
            return Err(ProxyError::Leg(String::from(
                "first frame was not the joined signal",
            )));
        }
        Ok(())
    }

    /// Dial and wait in one step for the common sequential case.
    pub async fn join(
        inner: S,
        purpose: Purpose,
        role: Role,
        token_hex: &str,
    ) -> Result<Self, ProxyError> {
        let mut leg = Self::dial(inner, purpose, role, token_hex).await?;
        leg.await_joined().await?;
        Ok(leg)
    }

    /// Next binary frame from the peer; anything else kills the leg.
    pub async fn recv(&mut self) -> Result<Vec<u8>, ProxyError> {
        Self::recv_inner(&mut self.inner).await
    }

    /// One opaque frame toward the peer.
    pub async fn send(&mut self, bytes: Vec<u8>) -> Result<(), ProxyError> {
        self.inner
            .send(Message::Binary(bytes.into()))
            .await
            .map_err(|error| ProxyError::Leg(format!("frame send failed: {error}")))
    }

    /// Close the underlying stream.
    pub async fn close(mut self) -> Result<(), ProxyError> {
        self.inner
            .send(Message::Close(None))
            .await
            .map_err(|error| ProxyError::Leg(format!("close failed: {error}")))
    }

    async fn recv_inner(inner: &mut S) -> Result<Vec<u8>, ProxyError> {
        let next = tokio::time::timeout(
            std::time::Duration::from_secs(STEP_TIMEOUT_SECS),
            inner.next(),
        )
        .await
        .map_err(|_| ProxyError::Leg(String::from("timed out waiting for peer")))?;
        match next {
            Some(Ok(Message::Binary(bytes))) => Ok(bytes.to_vec()),
            Some(Ok(other)) => Err(ProxyError::Leg(format!("non-binary leg message: {other}"))),
            Some(Err(error)) => Err(ProxyError::Leg(format!("leg error: {error}"))),
            None => Err(ProxyError::Leg(String::from("leg closed by peer"))),
        }
    }
}

/// Rebuild the fixed relay offer template from the 6-digit code. This
/// duplicates the requester's derivation on purpose: any drift fails
/// the handshake. Returns the offer plus the offer_id hex for the leg.
pub fn fixed_offer(relay_url: &str, code: &str) -> Result<(PairingOffer, String), ProxyError> {
    let digits: String = code.chars().filter(char::is_ascii_digit).collect();
    if digits.len() != 6 {
        return Err(ProxyError::Local(String::from("code must hold 6 digits")));
    }
    let secret_digest = Sha256::new()
        .chain_update(b"refineid-rapp-pairing-secret-v1:")
        .chain_update(&digits)
        .finalize();
    let id_digest = Sha256::new()
        .chain_update(b"refineid-rapp-offer-id-v1:")
        .chain_update(&digits)
        .finalize();
    let mut secret_bytes = [0u8; PAIRING_SECRET_SIZE];
    secret_bytes.copy_from_slice(&secret_digest);
    let mut id_bytes = [0u8; OFFER_ID_SIZE];
    id_bytes.copy_from_slice(&id_digest);
    let mut parameters = BTreeMap::new();
    parameters.insert(
        String::from("relay_url"),
        WireValue::Text(String::from(relay_url)),
    );
    let offer = PairingOffer::reconstruct(
        refineid_rapp::OfferId::from_array(id_bytes),
        PairingSecret::from_random_bytes(secret_bytes),
        vec![MANDATORY_PAIRING_SUITE.to_string()],
        vec![
            String::from("fi.refineid.authentication.v1"),
            String::from("fi.refineid.document-signing.v1"),
        ],
        vec![TransportCandidate {
            profile: String::from("relay-websocket-v1"),
            candidate_id: String::from(RELAY_CANDIDATE_ID),
            parameters,
        }],
        OFFER_TTL_MS,
    )
    .map_err(|error| ProxyError::Local(format!("offer rejected: {error:?}")))?;
    Ok((offer, hex(&id_bytes)))
}

/// Run the proxy side of pairing over one joined pairing leg: XX
/// responder, hello exchange, then read-approve-echo the grant set.
/// Grant lists must match exactly, so the requester's confirmation is
/// read first and echoed back after approval.
pub async fn run_pairing<S>(
    leg: &mut Leg<S>,
    offer: PairingOffer,
    app_label: &str,
    approver: &mut (dyn Approver + Send),
    clock: &Instant,
) -> Result<PairRecord, ProxyError>
where
    S: Stream<Item = Result<Message, WsError>> + Sink<Message, Error = WsError> + Unpin,
{
    let keys =
        generate_pair_key_material().map_err(|error| ProxyError::Local(format!("{error:?}")))?;
    let mut proxy =
        PairingHandshake::begin(EndpointRole::Proxy, offer, RELAY_CANDIDATE_ID, keys)
            .map_err(|failure| ProxyError::Pairing(format!("{:?}", failure.into_parts().0)))?;
    proxy
        .read_message(&frame(leg.recv().await?)?)
        .map_err(|error| ProxyError::Pairing(format!("m1 unreadable: {error:?}")))?;
    let m2 = proxy
        .write_message()
        .map_err(|error| ProxyError::Pairing(format!("m2 failed: {error:?}")))?;
    leg.send(m2.into_bytes()).await?;
    proxy
        .read_message(&frame(leg.recv().await?)?)
        .map_err(|error| ProxyError::Pairing(format!("m3 unreadable: {error:?}")))?;
    if !proxy.is_complete() {
        return Err(ProxyError::Pairing(String::from(
            "handshake incomplete after m3",
        )));
    }

    let mut confirm = proxy
        .into_confirmation()
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))?;
    let hello = confirm
        .send_hello(String::from(app_label), String::from("terminal"))
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))?;
    leg.send(hello.into_bytes()).await?;
    let peer_frame = frame(leg.recv().await?)?;
    let peer = confirm
        .receive_hello(&peer_frame, now_ms(clock))
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))?;
    let peer = PeerHello {
        display_name: peer.display_name.clone(),
        platform: peer.platform.clone(),
        requested: peer
            .requested_profiles
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|profile| profile.as_str().to_string())
            .collect(),
    };

    let grant_frame = frame(leg.recv().await?)?;
    let granted: Vec<String> = confirm
        .receive_confirmation(&grant_frame, now_ms(clock))
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))?
        .iter()
        .map(|profile| profile.as_str().to_string())
        .collect();
    if !approver.approve_pairing(&peer, &granted) {
        return Err(ProxyError::Declined("pairing"));
    }
    let echo: Vec<ProfileName> = granted
        .iter()
        .map(|name| ProfileName::parse(name))
        .collect::<Option<Vec<ProfileName>>>()
        .ok_or_else(|| ProxyError::Pairing(String::from("unknown grant name")))?;
    let ours = confirm
        .send_confirmation(echo)
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))?;
    leg.send(ours.into_bytes()).await?;
    confirm
        .into_pair_record(now_ms(clock))
        .map_err(|error| ProxyError::Pairing(format!("{error:?}")))
}

/// Run the proxy side of one session over a joined session leg: KK
/// responder, then read the requester ready, ask for login approval,
/// and only then send our ready. Returns the pair id hex plus the live
/// endpoint; the caller holds both until the leg closes.
pub async fn run_session<S>(
    leg: &mut Leg<S>,
    record: &PairRecord,
    approver: &mut (dyn Approver + Send),
    clock: &Instant,
) -> Result<(String, EstablishedEndpoint), ProxyError>
where
    S: Stream<Item = Result<Message, WsError>> + Sink<Message, Error = WsError> + Unpin,
{
    let pair_hex = hex(record.pair_id().as_bytes());
    let mut proxy = SessionHandshake::begin_proxy(record)
        .map_err(|error| ProxyError::Session(format!("{error:?}")))?;
    proxy
        .read_message(&frame(leg.recv().await?)?)
        .map_err(|error| ProxyError::Session(format!("m1 unreadable: {error:?}")))?;
    let m2 = proxy
        .write_message()
        .map_err(|error| ProxyError::Session(format!("m2 failed: {error:?}")))?;
    leg.send(m2.into_bytes()).await?;
    let mut auth = proxy
        .into_authentication()
        .map_err(|error| ProxyError::Session(format!("{error:?}")))?;
    let ready_frame = frame(leg.recv().await?)?;
    let mut store = MemoryStore::default();
    auth.receive_ready(&mut store, &ready_frame, now_ms(clock))
        .map_err(|error| ProxyError::Session(format!("{error:?}")))?;
    if !approver.approve_session(&pair_hex) {
        return Err(ProxyError::Declined("login"));
    }
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce)
        .map_err(|error| ProxyError::Local(format!("no randomness: {error}")))?;
    let ready = auth
        .send_ready(nonce)
        .map_err(|error| ProxyError::Session(format!("{error:?}")))?;
    leg.send(ready.into_bytes()).await?;
    let endpoint = auth
        .into_established()
        .map_err(|error| ProxyError::Session(format!("{error:?}")))?;
    Ok((pair_hex, endpoint))
}

/// Hold an established leg open, logging envelope sizes until close.
pub async fn hold<S>(leg: &mut Leg<S>) -> Result<(), ProxyError>
where
    S: Stream<Item = Result<Message, WsError>> + Sink<Message, Error = WsError> + Unpin,
{
    loop {
        match leg.recv().await {
            Ok(bytes) => println!("envelope: {} bytes", bytes.len()),
            Err(error) => {
                println!("leg ended: {error}");
                return Ok(());
            }
        }
    }
}

fn now_ms(start: &Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn frame(bytes: Vec<u8>) -> Result<BinaryFrame, ProxyError> {
    BinaryFrame::reconstruct(bytes).map_err(|error| ProxyError::Leg(format!("{error:?}")))
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

fn unhex(text: &str) -> Result<Vec<u8>, ProxyError> {
    if !text.len().is_multiple_of(2) {
        return Err(ProxyError::Local(String::from("odd hex length")));
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16)
                .map_err(|_| ProxyError::Local(String::from("invalid hex")))
        })
        .collect()
}

/// Minimal in-memory pair store. The ready happy path never touches it;
/// only revocation paths do.
#[derive(Debug, Default)]
struct MemoryStore {
    pairs: HashMap<PairId, PairRecord>,
    revoked: std::collections::HashSet<PairId>,
}

#[derive(Debug)]
struct StoreError;

impl PairStore for MemoryStore {
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
