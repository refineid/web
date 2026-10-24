// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! End-to-end leg tests over in-memory duplex streams: no sockets, no
//! binds. Each test drives scripted WebSocket legs against the real
//! `handle_leg` task and asserts join/match/forward/close behavior.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use refineid_relay_poc::protocol::{Join, Purpose, Role, encode_join, encode_joined};
use refineid_relay_poc::server::{Config, Relay, handle_leg, sweep_once};
use refineid_relay_poc::{JOIN_TIMEOUT_MS, SESSION_TOKEN_SIZE};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

type Client = WebSocketStream<tokio::io::DuplexStream>;

const DUMMY_PEER: &str = "127.0.0.1:1";
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

fn test_relay() -> Arc<Relay> {
    let config = Config {
        join_timeout: Duration::from_millis(JOIN_TIMEOUT_MS),
        ..Config::default()
    };
    Arc::new(Relay::new(config))
}

/// Spawn one server leg task and return the connected client side.
async fn connect(relay: &Arc<Relay>) -> Client {
    let (client_io, server_io) = tokio::io::duplex(256 * 1024);
    let peer: std::net::SocketAddr = DUMMY_PEER.parse().unwrap();
    tokio::spawn(handle_leg(relay.clone(), server_io, peer));
    let (client, _) = tokio_tungstenite::client_async("ws://relay/", client_io)
        .await
        .unwrap();
    client
}

fn join(role: Role, token: u8) -> Vec<u8> {
    encode_join(&Join {
        purpose: Purpose::Session,
        role,
        token: vec![token; SESSION_TOKEN_SIZE],
    })
}

/// Next binary message, or panic on timeout/close.
async fn recv_binary(client: &mut Client) -> Vec<u8> {
    let message = tokio::time::timeout(STEP_TIMEOUT, client.next())
        .await
        .expect("timed out waiting");
    match message {
        Some(Ok(Message::Binary(bytes))) => bytes.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

/// Assert the leg is dead: no more binary frames, only close/EOF/error.
async fn expect_closed(client: &mut Client) {
    let end = Instant::now() + STEP_TIMEOUT;
    loop {
        let remaining = end.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("leg still open after timeout");
        }
        match tokio::time::timeout(remaining, client.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => {
                panic!("leg alive: got {} bytes", bytes.len());
            }
            Ok(Some(Ok(Message::Ping(_)))) => {
                let _ignored = client.send(Message::Pong(vec![].into())).await;
            }
            Ok(_) | Err(_) => return,
        }
    }
}

#[tokio::test]
async fn happy_path_forwards_both_ways() {
    let relay = test_relay();
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(join(Role::Requester, 11).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(join(Role::Proxy, 11).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
    requester
        .send(Message::Binary(b"requester-frame".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(recv_binary(&mut proxy).await, b"requester-frame");
    proxy
        .send(Message::Binary(b"proxy-frame".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(recv_binary(&mut requester).await, b"proxy-frame");
    assert_eq!(relay.leg_count(), 2);
}

#[tokio::test]
async fn pairing_purpose_matches_too() {
    let relay = test_relay();
    let pairing = |role| {
        encode_join(&Join {
            purpose: Purpose::Pairing,
            role,
            token: vec![12; SESSION_TOKEN_SIZE],
        })
    };
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(pairing(Role::Requester).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(pairing(Role::Proxy).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
}

#[tokio::test]
async fn malformed_join_closes_silently() {
    let relay = test_relay();
    for bad in [b"garbage".to_vec(), vec![0x83, 0x01], encode_joined()] {
        let mut leg = connect(&relay).await;
        leg.send(Message::Binary(bad.into())).await.unwrap();
        expect_closed(&mut leg).await;
    }
    assert_eq!(relay.leg_count(), 0);
}

#[tokio::test]
async fn text_message_kills_leg() {
    let relay = test_relay();
    let mut leg = connect(&relay).await;
    leg.send(Message::Text("hello".into())).await.unwrap();
    expect_closed(&mut leg).await;
    assert_eq!(relay.leg_count(), 0);
}

#[tokio::test]
async fn same_role_rejected_first_survives() {
    let relay = test_relay();
    let mut first = connect(&relay).await;
    first
        .send(Message::Binary(join(Role::Requester, 21).into()))
        .await
        .unwrap();
    let mut dup = connect(&relay).await;
    dup.send(Message::Binary(join(Role::Requester, 21).into()))
        .await
        .unwrap();
    expect_closed(&mut dup).await;
    // The first leg still waits: its complement matches it.
    let mut proxy = connect(&relay).await;
    proxy
        .send(Message::Binary(join(Role::Proxy, 21).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut first).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
}

#[tokio::test]
async fn third_leg_rejected_after_match() {
    let relay = test_relay();
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(join(Role::Requester, 22).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(join(Role::Proxy, 22).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
    let mut third = connect(&relay).await;
    third
        .send(Message::Binary(join(Role::Proxy, 22).into()))
        .await
        .unwrap();
    expect_closed(&mut third).await;
    // The pair is undisturbed.
    requester
        .send(Message::Binary(b"still-here".to_vec().into()))
        .await
        .unwrap();
    assert_eq!(recv_binary(&mut proxy).await, b"still-here");
}

#[tokio::test]
async fn pre_join_second_frame_kills_leg() {
    let relay = test_relay();
    let mut leg = connect(&relay).await;
    leg.send(Message::Binary(join(Role::Requester, 23).into()))
        .await
        .unwrap();
    leg.send(Message::Binary(b"sneaky".to_vec().into()))
        .await
        .unwrap();
    expect_closed(&mut leg).await;
}

#[tokio::test]
async fn close_propagates_to_peer() {
    let relay = test_relay();
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(join(Role::Requester, 24).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(join(Role::Proxy, 24).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
    requester.close(None).await.unwrap();
    expect_closed(&mut proxy).await;
}

#[tokio::test]
async fn oversize_frame_kills_leg() {
    let relay = test_relay();
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(join(Role::Requester, 25).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(join(Role::Proxy, 25).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
    requester
        .send(Message::Binary(vec![0xAA; 65_536].into()))
        .await
        .unwrap();
    expect_closed(&mut requester).await;
    expect_closed(&mut proxy).await;
}

#[tokio::test]
async fn pairing_uses_32_byte_tokens() {
    use refineid_relay_poc::PAIRING_TOKEN_SIZE;
    let relay = test_relay();
    let pairing = |role| {
        encode_join(&Join {
            purpose: Purpose::Pairing,
            role,
            token: vec![31; PAIRING_TOKEN_SIZE],
        })
    };
    let mut requester = connect(&relay).await;
    let mut proxy = connect(&relay).await;
    requester
        .send(Message::Binary(pairing(Role::Requester).into()))
        .await
        .unwrap();
    proxy
        .send(Message::Binary(pairing(Role::Proxy).into()))
        .await
        .unwrap();
    let joined = encode_joined();
    assert_eq!(recv_binary(&mut requester).await, joined);
    assert_eq!(recv_binary(&mut proxy).await, joined);
}

#[tokio::test]
async fn purpose_isolates_tokens() {
    let relay = test_relay();
    let mut session_leg = connect(&relay).await;
    session_leg
        .send(Message::Binary(join(Role::Requester, 26).into()))
        .await
        .unwrap();
    // Same token bytes, pairing purpose: must NOT match the session waiter.
    let mut pairing_leg = connect(&relay).await;
    let pairing = encode_join(&Join {
        purpose: Purpose::Pairing,
        role: Role::Proxy,
        token: vec![26; SESSION_TOKEN_SIZE],
    });
    pairing_leg
        .send(Message::Binary(pairing.into()))
        .await
        .unwrap();
    // Neither leg may see a joined signal.
    let quiet = tokio::time::timeout(Duration::from_millis(300), session_leg.next()).await;
    assert!(
        quiet.is_err(),
        "session leg matched across purposes: {quiet:?}"
    );
    let quiet = tokio::time::timeout(Duration::from_millis(300), pairing_leg.next()).await;
    assert!(
        quiet.is_err(),
        "pairing leg matched across purposes: {quiet:?}"
    );
    assert_eq!(relay.leg_count(), 2);
}

#[tokio::test]
async fn sweep_closes_stale_waiters() {
    let config = Config {
        join_timeout: Duration::from_millis(20),
        ..Config::default()
    };
    let relay = Arc::new(Relay::new(config));
    let mut waiter = connect(&relay).await;
    waiter
        .send(Message::Binary(join(Role::Requester, 27).into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    sweep_once(&relay);
    expect_closed(&mut waiter).await;
    assert_eq!(relay.leg_count(), 0);
}

#[tokio::test]
async fn rate_limit_admits_then_blocks() {
    let relay = test_relay();
    let ip: std::net::IpAddr = "127.0.0.1".parse().unwrap();
    let now = Instant::now();
    for _ in 0..20 {
        assert!(relay.admit_ip(ip, now));
    }
    assert!(!relay.admit_ip(ip, now));
    // Another source is unaffected.
    let other: std::net::IpAddr = "127.0.0.2".parse().unwrap();
    assert!(relay.admit_ip(other, now));
}
