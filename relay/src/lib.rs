// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! Untrusted RAPP relay for the `relay-websocket-v1` transport profile.
//!
//! Both peers dial this relay over WebSocket; legs join by token and the
//! relay forwards opaque frames. The relay holds no RAPP keys, parses
//! nothing past the join frame, and cannot emit authenticated errors.
//! Profile spec: `refineid-core/docs/protocols/rapp-relay-websocket-v1.md`.

pub mod engine;
pub mod protocol;
pub mod server;

/// Join timeout: an unmatched leg is closed with no signal after this.
pub const JOIN_TIMEOUT_MS: u64 = 60_000;
/// Largest accepted WebSocket binary message: one RAPP frame.
pub const MAX_MESSAGE_SIZE: usize = 65_535;
/// Session (`rendezvous_token`) token size.
pub const SESSION_TOKEN_SIZE: usize = 16;
/// Pairing (`offer_id`) token size: the full SHA-256, as implemented.
pub const PAIRING_TOKEN_SIZE: usize = 32;
/// Default cap on concurrent legs (deployment-configurable).
pub const DEFAULT_MAX_LEGS: usize = 1024;
/// Default per-source-IP join budget: attempts per window.
pub const DEFAULT_JOINS_PER_IP: usize = 20;
/// Default per-source-IP rate window.
pub const JOIN_WINDOW_MS: u64 = 60_000;
