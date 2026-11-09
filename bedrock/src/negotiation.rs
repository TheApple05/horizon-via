// SPDX-License-Identifier: MIT
// Integration follows Cinnabar's MIT client guide; retained notices are in notices/.

//! Explicit offline HTTP connection or Login on a caller-owned NetherNet transport.
//! This does not discover servers, authenticate an account or initialize gameplay.

use crate::limits::server_receive_limits;
use bedrock_client::handshake::{ClientHandshake, ClientPackOffer, HandshakeOptions};
use bedrock_client::packs::PackTransferLimits;
use bedrock_client_tokio::{
    connection::Connection,
    handshake::{self, LoginData, Observation},
    transport::Transport,
};
use bedrock_net::{BatchLimits, login::OfflineIdentity};
use p384::ecdsa::SigningKey;
use serde_json::value::RawValue;
use std::{
    io,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::time::{Instant, timeout_at};

/// The caller supplies the exact URL, HTTP policy and server trust policy.
/// Identity is installed from the same proof key used for the following Login.
pub struct OfflineHttp {
    pub signaling: nethernet_tokio::HttpSignaling,
    pub endpoint: String,
    pub config: nethernet_tokio::ConnectionConfig,
    /// Optional exact server proof key, checked before any settings/Login send.
    pub server_key: Option<p384::ecdsa::VerifyingKey>,
}

/// Dial an explicit HTTP-signaled endpoint and perform settings/offline Login
/// under one absolute deadline. No transport fallback or account credentials.
pub async fn connect_offline_http(
    request: OfflineHttp,
    key: Arc<SigningKey>,
    profile: OfflineProfile,
    setup_deadline: Instant,
    metadata: impl FnOnce(&Connection) -> io::Result<Box<RawValue>>,
    observe: &mut (impl FnMut(Observation<'_>) + Send),
) -> io::Result<LoginBoundary> {
    validate_profile(&profile)?;
    let OfflineHttp {
        signaling,
        endpoint,
        mut config,
        server_key,
    } = request;
    if config.identity.is_some() || config.server_trust.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "offline HTTP requires explicit server trust and no replacement proof identity",
        ));
    }
    let lifetime = setup_deadline.saturating_duration_since(Instant::now());
    if lifetime.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Bedrock setup deadline elapsed",
        ));
    }
    config.identity = Some(Arc::new(
        nethernet_tokio::SelfSignedIdentity::from_key(Arc::clone(&key), profile.now, lifetime)
            .map_err(io::Error::other)?,
    ));
    let client = bedrock_client_tokio::connect::http::connect(
        signaling,
        endpoint.clone(),
        config,
        setup_deadline,
        None,
    )
    .await?;
    let admitted = server_key
        .as_ref()
        .is_none_or(|key| client.server_public_key() == Some(key));
    let mut transport = Transport::from_http(client);
    if !admitted {
        let _ = transport.close().await;
        let _ = timeout_at(
            Instant::now() + Duration::from_secs(2),
            transport.finish_close(),
        )
        .await;
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Bedrock server proof key differs from the configured key",
        ));
    }
    login_offline(
        transport,
        key,
        profile,
        setup_deadline,
        |connection| {
            let raw = metadata(connection)?;
            let mut claims = bedrock_net::login::client_data::RawClientData::from_raw(&raw)?;
            claims.insert("ServerAddress", endpoint.into())?;
            serde_json::value::to_raw_value(&claims).map_err(io::Error::other)
        },
        observe,
    )
    .await
}

fn validate_profile(profile: &OfflineProfile) -> io::Result<()> {
    if profile.display_name.is_empty()
        || profile.display_name.len() > 64
        || uuid::Uuid::parse_str(&profile.uuid).is_err()
        || profile.display_name.chars().any(char::is_control)
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid explicit offline profile",
        ))
    } else {
        Ok(())
    }
}

/// Explicit fake/offline identity. Its account-only identity fields remain empty.
pub struct OfflineProfile {
    pub display_name: String,
    pub uuid: String,
    pub now: SystemTime,
}

/// Login is complete but packs/startup still belong to the original connection.
#[must_use = "retain the original connection for packs/startup or close it"]
pub struct LoginBoundary {
    pub connection: Connection,
    pub packs: Option<ClientPackOffer>,
    pub(crate) setup_deadline: Instant,
}

impl LoginBoundary {
    /// Retires pending output and the original transport within one cleanup budget.
    pub async fn close(mut self, budget: Duration) {
        retire(&mut self.connection, budget).await;
    }
}

pub(crate) async fn retire(connection: &mut Connection, budget: Duration) {
    let cutoff = Instant::now() + budget;
    connection.retire_send(cutoff.into_std());
    let _ = timeout_at(cutoff, connection.settle_send()).await;
    let _ = connection.transport_mut().close().await;
    let _ = timeout_at(cutoff, connection.transport_mut().finish_close()).await;
}

/// Uses one connection, proof key and byte-bounded input queue across both phases.
/// Metadata is built after compression settings. On timeout/error the same owner
/// is retired; dropping this future drops that owner instead of reconnecting.
/// The caller must drive packs/startup before presenting this as a playable join.
pub async fn login_offline(
    transport: Transport,
    key: Arc<SigningKey>,
    profile: OfflineProfile,
    setup_deadline: Instant,
    metadata: impl FnOnce(&Connection) -> io::Result<Box<RawValue>>,
    observe: &mut (impl FnMut(Observation<'_>) + Send),
) -> io::Result<LoginBoundary> {
    let limits = BatchLimits {
        max_encoded_bytes: 16 * 1024 * 1024,
        max_incoming_packets: Some(1600),
        max_outgoing_decoded_bytes: Some(16 * 1024 * 1024),
        ..BatchLimits::default()
    };
    let mut connection =
        Connection::new(transport, limits, server_receive_limits(), 8 * 1024 * 1024);
    let attempt = async {
        validate_profile(&profile)?;
        let configured = ClientHandshake::new(
            key,
            HandshakeOptions {
                blob_cache: false,
                max_login_bytes: 1024 * 1024,
                packs: PackTransferLimits {
                    max_packs: Some(8),
                    max_archive_bytes: Some(32 * 1024 * 1024),
                    max_stack_bytes: Some(64 * 1024 * 1024),
                    max_transfer_bytes: Some(128 * 1024 * 1024),
                    max_version_bytes: 256,
                    request_window: 4,
                },
            },
        )
        .map_err(io::Error::other)?;
        let (ready, _) = handshake::settings(&mut connection, configured, observe)
            .await
            .map_err(io::Error::other)?;
        let claims = metadata(&connection)?;
        let input = LoginData::Offline {
            identity: OfflineIdentity {
                display_name: &profile.display_name,
                identity: &profile.uuid,
                xuid: "",
                playfab_id: "",
                playfab_title_id: "",
            },
            claims: &claims,
            now: profile.now,
        };
        handshake::login(&mut connection, ready, input, observe)
            .await
            .map_err(io::Error::other)
    };
    let result = timeout_at(setup_deadline, attempt)
        .await
        .unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "Bedrock setup deadline elapsed",
            ))
        });
    match result {
        Ok(packs) => Ok(LoginBoundary {
            connection,
            packs,
            setup_deadline,
        }),
        Err(error) => {
            retire(&mut connection, Duration::from_secs(2)).await;
            Err(error)
        }
    }
}
