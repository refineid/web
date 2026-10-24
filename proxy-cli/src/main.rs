// Copyright 2026 Petri Koistinen. Licensed under the Apache License, Version 2.0.
//! `refineid-proxy-cli` demo binary: pair from a 6-digit code, approve
//! one login session, hold the leg. No card attached, no persistence:
//! pairs die with the process and approval proves pair possession plus
//! explicit consent, not card identity.

use std::time::Instant;

use refineid_proxy_cli::{Approver, ConsoleApprover, Leg, ProxyError, fixed_offer, hold};
use refineid_relay_poc::protocol::{Purpose, Role};

struct Args {
    relay_url: String,
    code: String,
    app_label: String,
    auto_approve: bool,
}

fn usage() -> ! {
    eprintln!(
        "usage: refineid-proxy-cli serve --code CODE [--relay URL] \
         [--app-label LABEL] [--auto-approve]"
    );
    std::process::exit(2);
}

fn parse_args() -> Args {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("serve") {
        usage();
    }
    let mut relay_url = String::from("ws://127.0.0.1:9443");
    let mut code = None;
    let mut app_label = String::from("proxy-cli");
    let mut auto_approve = false;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--code" => code = Some(args.next().unwrap_or_else(|| usage())),
            "--relay" => relay_url = args.next().unwrap_or_else(|| usage()),
            "--app-label" => app_label = args.next().unwrap_or_else(|| usage()),
            "--auto-approve" => auto_approve = true,
            _ => usage(),
        }
    }
    Args {
        relay_url,
        code: code.unwrap_or_else(|| usage()),
        app_label,
        auto_approve,
    }
}

struct AutoApprover;

impl Approver for AutoApprover {
    fn approve_pairing(
        &mut self,
        peer: &refineid_proxy_cli::PeerHello,
        granted: &[String],
    ) -> bool {
        println!(
            "auto-approving pairing with {} for {}",
            peer.display_name,
            granted.join(",")
        );
        true
    }

    fn approve_session(&mut self, pair_id_hex: &str) -> bool {
        println!("auto-approving login for pair {pair_id_hex}");
        true
    }
}

async fn connect(
    url: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ProxyError,
> {
    let (stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|error| ProxyError::Leg(format!("connect {url} failed: {error}")))?;
    Ok(stream)
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("proxy-cli: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ProxyError> {
    let args = parse_args();
    if args.auto_approve {
        eprintln!("warning: --auto-approve answers yes to every prompt; testing only");
    }
    let mut approver: Box<dyn Approver + Send> = if args.auto_approve {
        Box::new(AutoApprover)
    } else {
        Box::new(ConsoleApprover)
    };
    let clock = Instant::now();

    let (offer, offer_hex) = fixed_offer(&args.relay_url, &args.code)?;
    println!("pairing leg: joining as proxy (code accepted, offer derived)");
    let stream = connect(&args.relay_url).await?;
    let mut pairing = Leg::join(stream, Purpose::Pairing, Role::Proxy, &offer_hex).await?;
    println!("pairing leg joined; running handshake");
    let record = refineid_proxy_cli::run_pairing(
        &mut pairing,
        offer,
        &args.app_label,
        &mut *approver,
        &clock,
    )
    .await?;
    let pair_hex = hex(record.pair_id().as_bytes());
    println!("paired: {pair_hex}");
    pairing.close().await.ok();

    let rendezvous = hex(record.rendezvous_token().as_bytes());
    println!("session leg: joining as proxy");
    let stream = connect(&args.relay_url).await?;
    let mut session = Leg::join(stream, Purpose::Session, Role::Proxy, &rendezvous).await?;
    println!("session leg joined; running handshake");
    let (pair_hex, mut endpoint) =
        refineid_proxy_cli::run_session(&mut session, &record, &mut *approver, &clock).await?;
    println!("LOGIN APPROVED for pair {pair_hex}; holding session (Ctrl-C ends)");
    hold(&mut session).await?;
    endpoint.close_session();
    Ok(())
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
