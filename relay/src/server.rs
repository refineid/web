// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Relay server: accept WebSocket legs, match them by token, forward
//! opaque frames. Transport only; no RAPP keys, no frame parsing past
//! the join frame.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::engine::{LegId, MatchOutcome, Matcher};
use crate::protocol::{encode_joined, parse_join};
use crate::{
    DEFAULT_JOINS_PER_IP, DEFAULT_MAX_LEGS, JOIN_TIMEOUT_MS, JOIN_WINDOW_MS, MAX_MESSAGE_SIZE,
};
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Server configuration. All knobs have spec-derived defaults.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bind address (loopback default).
    pub bind: SocketAddr,
    /// Unmatched legs close after this long.
    pub join_timeout: Duration,
    /// Concurrent leg cap.
    pub max_legs: usize,
    /// Per-source-IP joins admitted per window.
    pub joins_per_ip: usize,
    /// Per-source-IP rate window.
    pub join_window: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:9443".parse().expect("valid loopback default"),
            join_timeout: Duration::from_millis(JOIN_TIMEOUT_MS),
            max_legs: DEFAULT_MAX_LEGS,
            joins_per_ip: DEFAULT_JOINS_PER_IP,
            join_window: Duration::from_millis(JOIN_WINDOW_MS),
        }
    }
}

/// Outbound traffic to one leg task.
#[derive(Debug)]
enum Outbound {
    /// The joined signal: send once, then the leg is matched.
    Joined,
    /// One opaque frame to the peer.
    Forward(Vec<u8>),
    /// Close now, with no signal to anyone.
    Close,
}

/// Shared relay state. Mutexes guard short critical sections only; no
/// `.await` while held.
pub struct Relay {
    pub config: Config,
    matcher: Mutex<Matcher>,
    senders: Mutex<HashMap<LegId, mpsc::UnboundedSender<Outbound>>>,
    joins_per_ip: Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
}

impl Relay {
    /// Empty relay with the given configuration.
    #[must_use]
    pub fn new(config: Config) -> Self {
        Self {
            config,
            matcher: Mutex::new(Matcher::default()),
            senders: Mutex::new(HashMap::new()),
            joins_per_ip: Mutex::new(HashMap::new()),
        }
    }

    /// Live legs tracked right now.
    #[must_use]
    pub fn leg_count(&self) -> usize {
        self.matcher.lock().map(|m| m.leg_count()).unwrap_or(0)
    }

    fn send(&self, leg: LegId, outbound: Outbound) {
        if let Ok(senders) = self.senders.lock()
            && let Some(sender) = senders.get(&leg)
        {
            let _ignored = sender.send(outbound);
        }
    }

    /// Forget a leg everywhere; close its joined peer if any.
    /// Lock order is always matcher first, senders second; sends happen
    /// with no lock held.
    fn remove_leg(&self, leg: LegId) {
        let peer = self
            .matcher
            .lock()
            .map(|mut m| m.remove(leg))
            .unwrap_or(None);
        let peer_sender = self.senders.lock().ok().and_then(|mut senders| {
            senders.remove(&leg);
            peer.and_then(|peer| senders.remove(&peer))
        });
        if let Some(sender) = peer_sender {
            let _ignored = sender.send(Outbound::Close);
        }
    }

    /// Per-source-IP budget check. True when the connection may proceed.
    pub fn admit_ip(&self, ip: IpAddr, now: Instant) -> bool {
        let Ok(mut table) = self.joins_per_ip.lock() else {
            return false;
        };
        let window_start = now - self.config.join_window;
        let queue = table.entry(ip).or_default();
        while queue.front().is_some_and(|oldest| *oldest < window_start) {
            queue.pop_front();
        }
        if queue.len() >= self.config.joins_per_ip {
            return false;
        }
        queue.push_back(now);
        true
    }

    /// True when no further legs fit.
    pub fn full(&self) -> bool {
        self.matcher
            .lock()
            .is_ok_and(|m| m.leg_count() >= self.config.max_legs)
    }
}

/// Read the next meaningful client message: binary frames only. Ping,
/// pong, and protocol frames are underlay noise. Returns `None` on
/// close, error, or anything that must kill the leg.
async fn next_binary<S>(stream: &mut S) -> Option<Vec<u8>>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match stream.next().await? {
            Ok(Message::Binary(bytes)) => return Some(bytes.to_vec()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            _ => return None,
        }
    }
}

/// Serve one leg over any byte stream (TCP in production, in-memory
/// duplex in tests).
pub async fn handle_leg<S>(relay: std::sync::Arc<Relay>, stream: S, peer: SocketAddr)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_MESSAGE_SIZE);
    let Ok(ws) = tokio_tungstenite::accept_async_with_config(stream, Some(config)).await else {
        return;
    };
    let (mut sink, mut stream) = ws.split();

    // One join frame, bounded by the join timeout. Anything else and the
    // leg closes with no signal.
    let join_bytes =
        match tokio::time::timeout(relay.config.join_timeout, next_binary(&mut stream)).await {
            Ok(Some(bytes)) => bytes,
            _ => return,
        };
    let Ok(join) = parse_join(&join_bytes) else {
        return;
    };

    let (leg, outcome) = match relay.matcher.lock() {
        Ok(mut matcher) => {
            let leg = matcher.issue_leg();
            let outcome = matcher.join(leg, &join, Instant::now());
            (leg, outcome)
        }
        Err(_) => return,
    };
    if outcome == MatchOutcome::Reject {
        return;
    }
    let (sender, mut inbox) = mpsc::unbounded_channel();
    if relay
        .senders
        .lock()
        .map(|mut s| s.insert(leg, sender))
        .is_err()
    {
        relay.remove_leg(leg);
        return;
    }

    let mut matched = false;
    if let MatchOutcome::Matched { peer } = outcome {
        relay.send(peer, Outbound::Joined);
        matched = true;
        if sink
            .send(Message::Binary(encode_joined().into()))
            .await
            .is_err()
        {
            relay.remove_leg(leg);
            return;
        }
    }

    eprintln!(
        "leg {leg:?} from {peer} ({:?}/{:?}) registered: {outcome:?}",
        join.role, join.purpose
    );
    loop {
        tokio::select! {
            incoming = stream.next() => {
                let forward = match incoming {
                    Some(Ok(Message::Binary(bytes))) if matched => Some(bytes.to_vec()),
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => None,
                    _ => break,
                };
                if let Some(bytes) = forward {
                    let peer = relay.matcher.lock().ok().and_then(|m| m.peer_of(leg));
                    match peer {
                        Some(peer) => relay.send(peer, Outbound::Forward(bytes)),
                        None => break,
                    }
                }
            }
            outbound = inbox.recv() => {
                match outbound {
                    Some(Outbound::Forward(bytes)) => {
                        if sink.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Outbound::Joined) => {
                        matched = true;
                        if sink.send(Message::Binary(encode_joined().into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Outbound::Close) | None => break,
                }
            }
        }
    }
    eprintln!("leg {leg:?} closed");
    relay.remove_leg(leg);
    let _ignored = sink.close().await;
}

/// Close legs still unmatched past the deadline. No signals, no state.
pub async fn sweeper(relay: std::sync::Arc<Relay>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        sweep_once(&relay);
    }
}

/// One sweep pass, exposed so tests need no timers of their own.
pub fn sweep_once(relay: &Relay) {
    let expired = relay
        .matcher
        .lock()
        .map(|m| m.expired(Instant::now(), relay.config.join_timeout))
        .unwrap_or_default();
    for leg in expired {
        relay.send(leg, Outbound::Close);
    }
}
