// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! `refineid-relay-poc` server binary: bind, admit, spawn legs. No TLS
//! termination in the PoC (deploy behind a TLS-terminating proxy so
//! browsers see `wss`); loopback testing uses plain `ws`.

use std::time::{Duration, Instant};

use refineid_relay_poc::server::{Config, Relay, handle_leg, sweeper};
use tokio::net::TcpListener;

fn usage() -> ! {
    eprintln!(
        "usage: refineid-relay-poc [--bind ADDR:PORT] [--max-legs N] \
         [--joins-per-ip N] [--join-timeout-ms MS]"
    );
    std::process::exit(2);
}

fn parse_args() -> Config {
    let mut config = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--bind" => config.bind = value.parse().unwrap_or_else(|_| usage()),
            "--max-legs" => config.max_legs = value.parse().unwrap_or_else(|_| usage()),
            "--joins-per-ip" => {
                config.joins_per_ip = value.parse().unwrap_or_else(|_| usage());
            }
            "--join-timeout-ms" => {
                let ms: u64 = value.parse().unwrap_or_else(|_| usage());
                config.join_timeout = Duration::from_millis(ms);
            }
            _ => usage(),
        }
    }
    config
}

#[tokio::main]
async fn main() {
    let config = parse_args();
    let listener = TcpListener::bind(config.bind)
        .await
        .unwrap_or_else(|error| {
            eprintln!("cannot bind {}: {error}", config.bind);
            std::process::exit(1);
        });
    eprintln!(
        "relay listening on {} (plain ws; terminate TLS in front for browsers)",
        config.bind
    );
    let relay = std::sync::Arc::new(Relay::new(config));
    tokio::spawn(sweeper(relay.clone()));
    loop {
        let Ok((stream, peer)) = listener.accept().await else {
            continue;
        };
        if relay.full() || !relay.admit_ip(peer.ip(), Instant::now()) {
            continue;
        }
        tokio::spawn(handle_leg(relay.clone(), stream, peer));
    }
}
