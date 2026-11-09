// SPDX-License-Identifier: MIT

//! Explicit pinned HTTP connection configuration for the host integration build.
use crate::{
    negotiation::{OfflineHttp, OfflineProfile},
    session::Request,
};
use p384::{
    ecdsa::{SigningKey, VerifyingKey},
    elliptic_curve::Generate,
};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::{
    io,
    sync::Arc,
    time::{Instant, SystemTime},
};

pub fn endpoint(address: &str) -> Option<&str> {
    let value = address.trim().strip_prefix("bedrock:")?;
    (value.starts_with("http://") || value.starts_with("https://"))
        .then_some(value)
        .filter(|value| !value.chars().any(char::is_whitespace))
}

/// The selected URL must match the configured route and its proof key. No
/// discovery, credential acquisition or trust-on-first-use is performed.
pub fn pinned_request(
    bytes: &[u8],
    selected: &str,
    name: &str,
    uuid: u128,
    radius: i32,
    deadline: Instant,
) -> io::Result<Request> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        endpoint: String,
        server_public_key_sec1_hex: String,
        metadata: Box<RawValue>,
    }
    let bad = |message| io::Error::new(io::ErrorKind::InvalidInput, message);
    if bytes.len() > 1024 * 1024 {
        return Err(bad("Bedrock connection configuration exceeds budget"));
    }
    let config: Config = serde_json::from_slice(bytes).map_err(io::Error::other)?;
    if config.endpoint != selected || radius <= 0 {
        return Err(bad(
            "Bedrock connection configuration does not match selected route",
        ));
    }
    let hex = config.server_public_key_sec1_hex.as_bytes();
    if hex.len() != 194 {
        return Err(bad("Bedrock server key must be uncompressed P384 SEC1 hex"));
    }
    let mut encoded = Vec::with_capacity(97);
    for pair in hex.chunks_exact(2) {
        let digit = |b: u8| {
            (b as char)
                .to_digit(16)
                .map(|v| v as u8)
                .ok_or_else(|| bad("invalid Bedrock server key hex"))
        };
        encoded.push(digit(pair[0])? * 16 + digit(pair[1])?);
    }
    let server_key = VerifyingKey::from_sec1_bytes(&encoded).map_err(io::Error::other)?;
    let display_name = name.to_owned();
    Ok(Request {
        http: OfflineHttp {
            signaling: nethernet_tokio::HttpSignaling::new("horizon-offline".into())
                .map_err(io::Error::other)?,
            endpoint: config.endpoint,
            config: nethernet_tokio::ConnectionConfig {
                // The established peer must prove the exact configured key
                // before the backend sends settings or offline Login.
                server_trust: Some(nethernet_tokio::TokenTrust::Any),
                attempts: 1,
                ..Default::default()
            },
            server_key: Some(server_key),
        },
        key: Arc::new(SigningKey::generate()),
        profile: OfflineProfile {
            display_name: display_name.clone(),
            uuid: uuid::Uuid::from_u128(uuid).to_string(),
            now: SystemTime::now(),
        },
        deadline,
        radius: bedrock_protocol::packets::RequestChunkRadius {
            chunk_radius: bedrock_protocol::wire::ZigZag32(radius),
            ..Default::default()
        },
        metadata: Box::new(move |_| {
            let mut raw =
                bedrock_net::login::client_data::RawClientData::from_raw(&config.metadata)?;
            raw.insert("ThirdPartyName", display_name.into())?;
            serde_json::value::to_raw_value(&raw).map_err(io::Error::other)
        }),
    })
}
