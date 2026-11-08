//! Direct consumer-packet bridges for releases before the incremental translation chain.
use super::{ServerVersion, ids, v1_8_9, v1_12_2, v1_16_5};
use crate::{Result, State};
use std::sync::Arc;
use v1_8_9::bridge::{Incoming, Outgoing};

pub(super) enum Bridge {
    V47(Box<v1_8_9::bridge::Bridge>),
    V340(Box<v1_12_2::Bridge>),
    V754(Box<v1_16_5::Bridge>),
}

impl Bridge {
    pub fn new(version: ServerVersion) -> Option<Self> {
        match version {
            ServerVersion::V1_8_9 => Some(Self::V47(Box::default())),
            ServerVersion::V1_12_2 => Some(Self::V340(Box::default())),
            ServerVersion::V1_16_5 => Some(Self::V754(Box::default())),
            _ => None,
        }
    }

    pub fn names(&self) -> Arc<ids::LegacyRegistries> {
        match self {
            Self::V47(_) => v1_8_9::bootstrap::names(),
            Self::V340(_) => v1_12_2::names(),
            Self::V754(_) => v1_16_5::names(),
        }
    }

    pub fn clientbound(&mut self, state: State, id: i32, payload: Vec<u8>) -> Result<Incoming> {
        match self {
            Self::V47(bridge) => bridge.clientbound(state, id, payload),
            Self::V340(bridge) => bridge.clientbound(state, id, payload),
            Self::V754(bridge) => bridge.clientbound(state, id, payload),
        }
    }

    pub fn serverbound(&mut self, state: State, id: i32, payload: &[u8]) -> Result<Outgoing> {
        match self {
            Self::V47(bridge) => bridge.serverbound(state, id, payload),
            Self::V340(bridge) => bridge.serverbound(state, id, payload),
            Self::V754(bridge) => bridge.serverbound(state, id, payload),
        }
    }

    pub fn take_predictions(&mut self) -> Vec<(i32, Vec<u8>)> {
        match self {
            Self::V47(bridge) => bridge.take_predictions(),
            Self::V340(bridge) => bridge.take_predictions(),
            Self::V754(bridge) => bridge.take_predictions(),
        }
    }

    pub fn online_mode(&self) -> bool {
        match self {
            Self::V47(bridge) => bridge.online_mode(),
            Self::V340(bridge) => bridge.online_mode(),
            Self::V754(bridge) => bridge.online_mode(),
        }
    }

    pub fn rejected_locally(&self) -> bool {
        match self {
            Self::V47(bridge) => bridge.rejected_locally(),
            Self::V340(bridge) => bridge.rejected_locally(),
            Self::V754(bridge) => bridge.rejected_locally(),
        }
    }

    pub fn finish_delivered(&mut self) {
        match self {
            Self::V47(bridge) => bridge.finish_delivered(),
            Self::V340(bridge) => bridge.finish_delivered(),
            Self::V754(bridge) => bridge.finish_delivered(),
        }
    }

    pub fn take_projectile_acceleration(&mut self, id: i32) -> Option<[f64; 3]> {
        match self {
            Self::V47(bridge) => bridge.take_projectile_acceleration(id),
            Self::V340(bridge) => bridge.take_projectile_acceleration(id),
            Self::V754(bridge) => bridge.take_projectile_acceleration(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        adapter::{ServerVersion, Session},
        buf::{Codec, Reader, Writer},
        packets::handshake,
    };

    #[test]
    fn status_queries_and_replies_keep_the_selected_protocol_and_payload() {
        for version in [
            ServerVersion::V1_8_9,
            ServerVersion::V1_12_2,
            ServerVersion::V1_16_5,
        ] {
            let session = Session::new(version);
            let encoded = session
                .serverbound(
                    &handshake::Serverbound::Intention(handshake::Intention {
                        protocol: 777,
                        host: "fixture.invalid".into(),
                        port: 25565,
                        intent: handshake::STATUS,
                    })
                    .encode(),
                )
                .unwrap()
                .unwrap();
            let mut r = Reader::new(&encoded);
            assert_eq!(r.varint().unwrap(), 0);
            let hello = handshake::Intention::read(&mut r).unwrap();
            assert_eq!(hello.protocol, version.protocol());
            assert_eq!(hello.host, "fixture.invalid");
            assert_eq!(r.remaining(), 0);
            assert!(session.serverbound(&[0]).unwrap().is_none());
            assert_eq!(session.take_outgoing(), vec![vec![0]]);
            let mut ping = Writer::new();
            ping.varint(1);
            ping.i64(0x1122_3344_5566_7788);
            session.serverbound(&ping.data).unwrap();
            assert_eq!(session.take_outgoing(), vec![ping.data]);
            assert_eq!(
                session.clientbound(0, vec![2, b'{', b'}']).unwrap(),
                Some((0, vec![2, b'{', b'}']))
            );
        }
    }
}
