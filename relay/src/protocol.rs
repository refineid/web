// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Join/joined codec for `relay-websocket-v1` legs.
//!
//! The join frame is the only message the relay ever parses:
//! `["RAPP-relay-v1", purpose, role, token]`.

use ciborium::Value;

use crate::{PAIRING_TOKEN_SIZE, SESSION_TOKEN_SIZE};

/// Join purposes: pairing legs match on `offer_id`, session legs on
/// `rendezvous_token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Purpose {
    Pairing,
    Session,
}

/// Leg roles. Only complementary roles ever match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Requester,
    Proxy,
}

impl Role {
    /// True when `other` is the role this leg must match with.
    #[must_use]
    pub const fn complements(self, other: Role) -> bool {
        !matches!(
            (self, other),
            (Role::Requester, Role::Requester) | (Role::Proxy, Role::Proxy)
        )
    }
}

/// A validated join frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Join {
    /// Pairing or session leg.
    pub purpose: Purpose,
    /// Requester or proxy leg.
    pub role: Role,
    /// `offer_id` (32 bytes) or `rendezvous_token` (16 bytes).
    pub token: Vec<u8>,
}

/// Why a join frame was rejected. Never sent back: the leg just closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinError {
    /// Not CBOR, or not a 4-array of the right shape.
    Malformed,
    /// First element is not `"RAPP-relay-v1"`.
    UnknownTag,
    /// Purpose is not `"pairing"` or `"session"`.
    UnknownPurpose,
    /// Role is not `"requester"` or `"proxy"`.
    UnknownRole,
    /// Token is neither a 16-byte rendezvous token nor a 32-byte
    /// offer identifier.
    BadToken,
}

/// Parse one binary WebSocket message as a join frame.
pub fn parse_join(bytes: &[u8]) -> Result<Join, JoinError> {
    let value: Value = ciborium::de::from_reader(bytes).map_err(|_| JoinError::Malformed)?;
    let Value::Array(items) = value else {
        return Err(JoinError::Malformed);
    };
    if items.len() != 4 {
        return Err(JoinError::Malformed);
    }
    let (Value::Text(tag), Value::Text(purpose), Value::Text(role), Value::Bytes(token)) =
        (&items[0], &items[1], &items[2], &items[3])
    else {
        return Err(JoinError::Malformed);
    };
    if tag != "RAPP-relay-v1" {
        return Err(JoinError::UnknownTag);
    }
    let purpose = match purpose.as_str() {
        "pairing" => Purpose::Pairing,
        "session" => Purpose::Session,
        _ => return Err(JoinError::UnknownPurpose),
    };
    let role = match role.as_str() {
        "requester" => Role::Requester,
        "proxy" => Role::Proxy,
        _ => return Err(JoinError::UnknownRole),
    };
    if token.len() != SESSION_TOKEN_SIZE && token.len() != PAIRING_TOKEN_SIZE {
        return Err(JoinError::BadToken);
    }
    Ok(Join {
        purpose,
        role,
        token: token.clone(),
    })
}

/// Encode a join frame (used by tests and, later, peer implementations).
#[must_use]
pub fn encode_join(join: &Join) -> Vec<u8> {
    let purpose = match join.purpose {
        Purpose::Pairing => "pairing",
        Purpose::Session => "session",
    };
    let role = match join.role {
        Role::Requester => "requester",
        Role::Proxy => "proxy",
    };
    let value = Value::Array(vec![
        Value::Text(String::from("RAPP-relay-v1")),
        Value::Text(String::from(purpose)),
        Value::Text(String::from(role)),
        Value::Bytes(join.token.clone()),
    ]);
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&value, &mut bytes).expect("infallible CBOR encode");
    bytes
}

/// Encode the joined signal: `["RAPP-relay-joined-v1"]`.
#[must_use]
pub fn encode_joined() -> Vec<u8> {
    let value = Value::Array(vec![Value::Text(String::from("RAPP-relay-joined-v1"))]);
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(&value, &mut bytes).expect("infallible CBOR encode");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Join {
        Join {
            purpose: Purpose::Session,
            role: Role::Proxy,
            token: vec![7; SESSION_TOKEN_SIZE],
        }
    }

    #[test]
    fn join_round_trip() {
        let join = sample();
        assert_eq!(parse_join(&encode_join(&join)), Ok(join));
    }

    #[test]
    fn pairing_token_is_32_bytes() {
        let join = Join {
            purpose: Purpose::Pairing,
            role: Role::Requester,
            token: vec![9; PAIRING_TOKEN_SIZE],
        };
        assert_eq!(parse_join(&encode_join(&join)), Ok(join));
    }

    #[test]
    fn rejects_in_between_lengths() {
        let array = |items: Vec<Value>| {
            let mut bytes = Vec::new();
            ciborium::ser::into_writer(&Value::Array(items), &mut bytes).unwrap();
            bytes
        };
        let join_bytes = array(vec![
            Value::Text(String::from("RAPP-relay-v1")),
            Value::Text(String::from("session")),
            Value::Text(String::from("proxy")),
            Value::Bytes(vec![1; 24]),
        ]);
        assert_eq!(parse_join(&join_bytes), Err(JoinError::BadToken));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_join(&[]), Err(JoinError::Malformed));
        assert_eq!(
            parse_join(b"not cbor at all..........."),
            Err(JoinError::Malformed)
        );
        assert_eq!(parse_join(&encode_joined()), Err(JoinError::Malformed));
    }

    #[test]
    fn rejects_wrong_shapes() {
        let array = |items: Vec<Value>| {
            let mut bytes = Vec::new();
            ciborium::ser::into_writer(&Value::Array(items), &mut bytes).unwrap();
            bytes
        };
        let text = |s: &str| Value::Text(String::from(s));
        let token = Value::Bytes(vec![1; SESSION_TOKEN_SIZE]);
        let short = Value::Bytes(vec![1; 8]);
        // Wrong tag.
        assert_eq!(
            parse_join(&array(vec![
                text("nope"),
                text("session"),
                text("proxy"),
                token.clone()
            ])),
            Err(JoinError::UnknownTag)
        );
        // Unknown purpose / role.
        assert_eq!(
            parse_join(&array(vec![
                text("RAPP-relay-v1"),
                text("nap"),
                text("proxy"),
                token.clone()
            ])),
            Err(JoinError::UnknownPurpose)
        );
        assert_eq!(
            parse_join(&array(vec![
                text("RAPP-relay-v1"),
                text("session"),
                text("bylurker"),
                token.clone()
            ])),
            Err(JoinError::UnknownRole)
        );
        // Short token.
        assert_eq!(
            parse_join(&array(vec![
                text("RAPP-relay-v1"),
                text("session"),
                text("proxy"),
                short
            ])),
            Err(JoinError::BadToken)
        );
        // Wrong arity / types.
        assert_eq!(
            parse_join(&array(vec![text("RAPP-relay-v1")])),
            Err(JoinError::Malformed)
        );
        assert_eq!(
            parse_join(&array(vec![
                text("RAPP-relay-v1"),
                Value::Integer(1.into()),
                text("proxy"),
                token
            ])),
            Err(JoinError::Malformed)
        );
    }
}
