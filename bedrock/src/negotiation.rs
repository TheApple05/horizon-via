// SPDX-License-Identifier: MIT
// Integration follows Cinnabar's MIT client guide; retained notices are in notices/.

//! Offline fixture Login over an already admitted caller-owned NetherNet transport.
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
}

impl LoginBoundary {
    /// Retires pending output and the original transport within one cleanup budget.
    pub async fn close(mut self, budget: Duration) {
        retire(&mut self.connection, budget).await;
    }
}

async fn retire(connection: &mut Connection, budget: Duration) {
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
        if profile.display_name.is_empty()
            || profile.display_name.len() > 64
            || uuid::Uuid::parse_str(&profile.uuid).is_err()
            || profile.display_name.chars().any(char::is_control)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid explicit offline profile",
            ));
        }
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
        Ok(packs) => Ok(LoginBoundary { connection, packs }),
        Err(error) => {
            retire(&mut connection, Duration::from_secs(2)).await;
            Err(error)
        }
    }
}
