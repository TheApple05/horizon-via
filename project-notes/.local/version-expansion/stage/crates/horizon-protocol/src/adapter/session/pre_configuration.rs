//! A local configuration view for servers which enter play directly after login.
//! No acknowledgement or configuration packet is sent on the old wire. The embedded registry and
//! real enabled features are exposed before the retained play login, using the normal consumer API.
use super::*;
use crate::Error;

const LOGIN: i32 = 40;
const ENABLED_FEATURES: i32 = 107;
const MAX_PACKETS: usize = 128;
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Bootstrap {
    started: bool,
    acknowledged: bool,
    finish_queued: bool,
    finish_delivered: bool,
    login: Option<Vec<u8>>,
    early: Vec<(i32, Vec<u8>)>,
    deferred: Vec<Vec<u8>>,
    bytes: usize,
}

impl Bootstrap {
    pub(super) fn login_finished(&mut self) {
        *self = Self {
            started: true,
            ..Self::default()
        };
    }

    pub(super) fn finish_delivered(&mut self) {
        if self.finish_queued {
            self.finish_delivered = true;
        }
    }

    fn reserve(&mut self, bytes: usize) -> Result<()> {
        if self.early.len() + self.deferred.len() >= MAX_PACKETS
            || bytes > MAX_BYTES.saturating_sub(self.bytes)
        {
            return Err(Error::new("legacy configuration buffer limit exceeded"));
        }
        self.bytes += bytes;
        Ok(())
    }
}

pub(super) fn clientbound(
    session: &Session,
    id: i32,
    payload: Vec<u8>,
) -> Result<Option<(i32, Vec<u8>)>> {
    use ids::configuration::clientbound as cfg;
    if id == LOGIN {
        let registry = translate::v1_20_1::login_registry(&payload)?;
        {
            let mut bootstrap = session.pre_configuration.lock().unwrap();
            if !bootstrap.started || bootstrap.login.is_some() || bootstrap.finish_queued {
                return Err(Error::new("unexpected legacy play login"));
            }
            bootstrap.reserve(payload.len())?;
            bootstrap.login = Some(payload);
        }
        session.set_legacy_secure_chat_enforcement(None);
        let mut w = Writer::new();
        w.nbt(&registry);
        return configuration(session, cfg::REGISTRY_DATA, &w.data).map(Some);
    }
    if id == ENABLED_FEATURES {
        // Validate before changing state or consuming the held login. The actual flags, including
        // experimental ones, must arrive rather than being replaced with assumed vanilla defaults.
        let mut r = Reader::new(&payload);
        let count = r.count(1024)?;
        for _ in 0..count {
            r.string(32767)?;
        }
        if r.remaining() != 0 {
            return Err(Error::new("trailing legacy enabled-feature bytes"));
        }
        let (login, early) = {
            let mut bootstrap = session.pre_configuration.lock().unwrap();
            if bootstrap.finish_queued {
                return Err(Error::new("duplicate legacy enabled features"));
            }
            let login = bootstrap
                .login
                .take()
                .ok_or_else(|| Error::new("legacy features precede the play login"))?;
            bootstrap.finish_queued = true;
            (login, std::mem::take(&mut bootstrap.early))
        };
        let previous = std::mem::take(&mut *session.incoming.lock().unwrap());
        let finish = configuration(session, cfg::FINISH_CONFIGURATION, &[])?;
        let mut queued = previous;
        queued.push_back(finish);
        queued.extend(session.incoming.lock().unwrap().drain(..));
        // From this point raw packets are old play. Consumers still see the queued virtual finish
        // before any play packet; the writer independently waits for the consumer's finish ack.
        session.set_reading(State::Play);
        if let Some(packet) = session.clientbound(LOGIN, login)? {
            queued.push_back(packet);
        }
        queued.extend(session.incoming.lock().unwrap().drain(..));
        for (id, body) in early {
            if let Some(packet) = session.clientbound(id, body)? {
                queued.push_back(packet);
            }
            queued.extend(session.incoming.lock().unwrap().drain(..));
        }
        *session.incoming.lock().unwrap() = queued;
        return Ok(Some((cfg::UPDATE_ENABLED_FEATURES, payload)));
    }
    // A custom server can disconnect or send a heartbeat before its enabled-features packet.
    // Keep their semantics in the local configuration state instead of treating raw play ids as
    // configuration ids. Other packets are bounded and retained until the virtual finish.
    let pinned = session.version.clientbound_id(State::Play, id);
    let configuration_id = match pinned {
        Some(ids::play::clientbound::DISCONNECT) => Some(cfg::DISCONNECT),
        Some(ids::play::clientbound::KEEP_ALIVE) => Some(cfg::KEEP_ALIVE),
        _ => None,
    };
    if let Some(configuration_id) = configuration_id {
        if let Some((_, body)) =
            translate::clientbound(session, State::Play, pinned.unwrap(), &payload)?
        {
            return Ok(Some((configuration_id, body.unwrap_or(payload))));
        }
        return Ok(None);
    }
    let mut bootstrap = session.pre_configuration.lock().unwrap();
    if !bootstrap.started || bootstrap.finish_queued {
        return Err(Error::new("unexpected legacy configuration packet"));
    }
    bootstrap.reserve(payload.len())?;
    bootstrap.early.push((id, payload));
    Ok(None)
}

fn configuration(session: &Session, id: i32, payload: &[u8]) -> Result<(i32, Vec<u8>)> {
    match translate::clientbound(session, State::Configuration, id, payload)? {
        Some((id, body)) => Ok((id, body.unwrap_or_else(|| payload.to_vec()))),
        None => Err(Error::new(
            "legacy virtual configuration packet was dropped",
        )),
    }
}

/// The outer option means this state/id was handled, the inner one is an optional real wire body.
pub(super) fn serverbound(
    session: &Session,
    state: State,
    id: i32,
    payload: &[u8],
) -> Result<Option<Option<Vec<u8>>>> {
    use ids::configuration::serverbound as cfg;
    if state == State::Login {
        if id != ids::login::serverbound::LOGIN_ACKNOWLEDGED {
            return Ok(None);
        }
        let mut bootstrap = session.pre_configuration.lock().unwrap();
        if !payload.is_empty() || !bootstrap.started || bootstrap.acknowledged {
            return Err(Error::new("unexpected virtual login acknowledgement"));
        }
        bootstrap.acknowledged = true;
        session.set_writing(State::Configuration);
        return Ok(Some(None));
    }
    if id == cfg::FINISH_CONFIGURATION {
        let mut bootstrap = session.pre_configuration.lock().unwrap();
        if !payload.is_empty() || !bootstrap.acknowledged || !bootstrap.finish_delivered {
            return Err(Error::new(
                "premature virtual configuration acknowledgement",
            ));
        }
        session.set_writing(State::Play);
        session
            .outgoing
            .lock()
            .unwrap()
            .append(&mut bootstrap.deferred);
        bootstrap.bytes = 0;
        return Ok(Some(None));
    }
    let play = match id {
        cfg::CLIENT_INFORMATION => ids::play::serverbound::CLIENT_INFORMATION,
        cfg::CUSTOM_PAYLOAD => ids::play::serverbound::CUSTOM_PAYLOAD,
        cfg::KEEP_ALIVE => ids::play::serverbound::KEEP_ALIVE,
        _ => return Ok(Some(None)), // No counterpart on the old wire.
    };
    let Some((mapped, body)) = translate::serverbound(session, State::Play, play, payload)? else {
        return Ok(Some(None));
    };
    let mut w = Writer::new();
    w.varint(mapped);
    w.bytes(&body);
    if id == cfg::KEEP_ALIVE {
        return Ok(Some(Some(w.data)));
    }
    let mut bootstrap = session.pre_configuration.lock().unwrap();
    bootstrap.reserve(w.data.len())?;
    bootstrap.deferred.push(w.data);
    Ok(Some(None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::{configuration as current, login, play};
    use horizon_world::nbt::Tag;

    fn body(id: i32, payload: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(id);
        w.bytes(payload);
        w.data
    }

    fn profile_finished() -> Session {
        let s = Session::new(ServerVersion::V1_20_1);
        s.set_reading(State::Login);
        s.set_writing(State::Login);
        let mut profile = Writer::new();
        profile.uuid(1);
        profile.string("BootstrapProbe");
        profile.varint(0);
        let (id, payload) = s.clientbound(2, profile.data).unwrap().unwrap();
        assert!(matches!(
            login::Clientbound::decode(id, &payload).unwrap(),
            login::Clientbound::LoginFinished(_)
        ));
        assert_eq!(
            (s.reading(), s.writing()),
            (State::Configuration, State::Login)
        );
        s
    }

    fn acknowledged() -> Session {
        let s = profile_finished();
        assert!(
            s.serverbound(&body(ids::login::serverbound::LOGIN_ACKNOWLEDGED, &[]))
                .unwrap()
                .is_none()
        );
        assert!(s.take_outgoing().is_empty());
        assert_eq!(s.writing(), State::Configuration);
        s
    }

    // This deliberately synthetic RegistryAccess has the real container shape and one dimension
    // entry. It tests extraction/forwarding; it is not an actual vanilla world-registry capture.
    fn registry(name: &str) -> Tag {
        Tag::compound([(
            "minecraft:dimension_type",
            Tag::compound([
                ("type", Tag::String("minecraft:dimension_type".into())),
                (
                    "value",
                    Tag::List(vec![Tag::compound([
                        ("name", Tag::String(name.into())),
                        ("id", Tag::Int(0)),
                        (
                            "element",
                            Tag::compound([
                                ("height", Tag::Int(256)),
                                ("min_y", Tag::Int(0)),
                                ("synthetic_fixture", Tag::Byte(1)),
                            ]),
                        ),
                    ])]),
                ),
            ]),
        )])
    }

    fn play_login(name: &str, dimension: &str) -> Vec<u8> {
        let mut w = Writer::new();
        w.i32(42);
        w.bool(false);
        w.u8(1);
        w.i8(-1);
        w.varint(1);
        w.string(dimension);
        w.named_nbt(&registry(name));
        w.string(name);
        w.string(dimension);
        w.i64(0x1020304050607080);
        w.varint(20);
        w.varint(12);
        w.varint(10);
        w.bool(false);
        w.bool(true);
        w.bool(false);
        w.bool(false);
        w.bool(false);
        w.varint(300);
        w.data
    }

    fn held_login(s: &Session, name: &str, dimension: &str) {
        let (id, payload) = s
            .clientbound(LOGIN, play_login(name, dimension))
            .unwrap()
            .unwrap();
        let current::Clientbound::RegistryData(packet) =
            current::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!("the embedded dimension registry must be delivered first");
        };
        assert_eq!(packet.registry, "minecraft:dimension_type");
        assert_eq!(packet.entries.len(), 1);
        assert_eq!(packet.entries[0].id, name);
        assert_eq!(
            packet.entries[0].data.as_ref(),
            registry(name)
                .get("minecraft:dimension_type")
                .and_then(|r| r.get("value"))
                .and_then(|r| match r {
                    Tag::List(v) => v.first(),
                    _ => None,
                })
                .and_then(|e| e.get("element"))
        );
        assert_eq!(s.reading(), State::Configuration);
    }

    fn features(s: &Session, names: &[&str]) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(names.len() as i32);
        for name in names {
            w.string(name);
        }
        let payload = w.data;
        let (id, delivered) = s
            .clientbound(ENABLED_FEATURES, payload.clone())
            .unwrap()
            .unwrap();
        assert_eq!(id, ids::configuration::clientbound::UPDATE_ENABLED_FEATURES);
        assert_eq!(delivered, payload);
        let current::Clientbound::UpdateEnabledFeatures(flags) =
            current::Clientbound::decode(id, &delivered).unwrap()
        else {
            panic!("expected the server's actual feature flags");
        };
        assert_eq!(flags.features, names);
        delivered
    }

    fn information() -> Vec<u8> {
        let mut info = current::ClientInformation::new(12);
        info.chat_visibility = 1;
        current::Serverbound::ClientInformation(info).encode()
    }

    fn brand(name: &str) -> Vec<u8> {
        let mut payload = Writer::new();
        payload.string(name);
        current::Serverbound::CustomPayload(current::CustomPayload {
            channel: "minecraft:brand".into(),
            data: payload.data,
        })
        .encode()
    }

    fn deliver_finish(s: &Session) {
        let mut registries = 0;
        loop {
            let (id, payload) = s.next_incoming().expect("finish must be queued");
            if id == ids::configuration::clientbound::FINISH_CONFIGURATION {
                assert!(payload.is_empty());
                break;
            }
            assert!(
                matches!(
                    current::Clientbound::decode(id, &payload).unwrap(),
                    current::Clientbound::RegistryData(_)
                ),
                "no play packet may precede the virtual finish"
            );
            registries += 1;
        }
        assert!(
            registries > 0,
            "required current registries must precede finish"
        );
    }

    #[test]
    fn virtual_flow_preserves_registries_features_and_never_sends_wire_acks() {
        let s = acknowledged();
        // The current consumer sends these before the old server's play Login arrives.
        assert!(s.serverbound(&information()).unwrap().is_none());
        assert!(s.serverbound(&brand("BootstrapBrand")).unwrap().is_none());
        assert!(s.take_outgoing().is_empty());
        held_login(&s, "probe:dimension", "probe:world");
        let mut time = Writer::new();
        time.i64(1234);
        time.i64(5678);
        assert!(s.clientbound(94, time.data).unwrap().is_none());
        features(&s, &["minecraft:vanilla", "probe:experimental"]);
        assert_eq!(
            (s.reading(), s.writing()),
            (State::Play, State::Configuration)
        );
        let ack = body(ids::configuration::serverbound::FINISH_CONFIGURATION, &[]);
        assert!(s.serverbound(&ack).is_err());
        assert_eq!(s.writing(), State::Configuration);
        assert!(s.take_outgoing().is_empty());
        deliver_finish(&s);
        assert!(
            s.take_outgoing().is_empty(),
            "delivery alone must not flush replies"
        );
        assert!(s.serverbound(&ack).unwrap().is_none());
        assert_eq!(s.writing(), State::Play);
        let sent = s.take_outgoing();
        assert_eq!(
            sent.len(),
            2,
            "only the two deferred real play packets leave"
        );
        // Official763 ClientInformation writer: old play id8 and no particle-status byte.
        assert_eq!(
            sent[0],
            vec![8, 5, b'e', b'n', b'_', b'u', b's', 12, 1, 1, 0x7f, 1, 0, 1]
        );
        let mut r = Reader::new(&sent[1]);
        assert_eq!(r.varint().unwrap(), 13);
        assert_eq!(r.string(32767).unwrap(), "minecraft:brand");
        assert_eq!(r.string(32767).unwrap(), "BootstrapBrand");
        assert_eq!(r.remaining(), 0);
        assert!(s.take_outgoing().is_empty());
        let (id, payload) = s.next_incoming().expect("held play Login");
        let play::Clientbound::Login(login) = play::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!("play Login must immediately follow virtual finish");
        };
        assert_eq!(login.player_id, 42);
        assert_eq!(login.levels, ["probe:world"]);
        assert_eq!(login.spawn.dimension_type, 0);
        assert_eq!(login.spawn.dimension, "probe:world");
        assert_eq!(login.spawn.portal_cooldown, 300);
        assert_eq!(login.spawn.sea_level, 63);
        let (id, payload) = s.next_incoming().expect("early play packet");
        assert!(matches!(
            play::Clientbound::decode(id, &payload).unwrap(),
            play::Clientbound::SetTime(_)
        ));
        assert!(s.next_incoming().is_none());
    }

    #[test]
    fn early_ack_and_features_cannot_advance_the_bootstrap() {
        let s = Session::new(ServerVersion::V1_20_1);
        s.set_reading(State::Login);
        s.set_writing(State::Login);
        assert!(
            s.serverbound(&body(ids::login::serverbound::LOGIN_ACKNOWLEDGED, &[]))
                .is_err()
        );
        assert_eq!(s.writing(), State::Login);
        assert!(s.take_outgoing().is_empty());
        let s = acknowledged();
        assert!(
            s.serverbound(&body(
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            ))
            .is_err()
        );
        assert!(s.clientbound(ENABLED_FEATURES, vec![0]).is_err());
        assert_eq!(
            (s.reading(), s.writing()),
            (State::Configuration, State::Configuration)
        );
        assert!(s.next_incoming().is_none());
        assert!(s.take_outgoing().is_empty());
    }

    #[test]
    fn malformed_features_and_duplicate_login_do_not_consume_the_held_login() {
        let s = acknowledged();
        held_login(&s, "probe:first", "probe:world");
        let original = s.pre_configuration.lock().unwrap().login.clone();
        assert!(
            s.clientbound(LOGIN, play_login("probe:second", "probe:other"))
                .is_err()
        );
        for malformed in [vec![1], vec![0, 1], vec![0xff, 0xff, 0xff, 0xff, 0x0f]] {
            assert!(s.clientbound(ENABLED_FEATURES, malformed).is_err());
            let bootstrap = s.pre_configuration.lock().unwrap();
            assert_eq!(bootstrap.login, original);
            assert!(!bootstrap.finish_queued);
            assert_eq!(s.reading(), State::Configuration);
        }
        assert_eq!(
            s.legacy_registries
                .lock()
                .unwrap()
                .get("minecraft:dimension_type")
                .unwrap()
                .name(0),
            Some("probe:first")
        );
        features(&s, &["minecraft:vanilla"]);
        deliver_finish(&s);
        let (id, payload) = s.next_incoming().unwrap();
        let play::Clientbound::Login(login) = play::Clientbound::decode(id, &payload).unwrap()
        else {
            panic!("Login");
        };
        assert_eq!(login.spawn.dimension, "probe:world");
    }

    #[test]
    fn packet_and_byte_budgets_bound_both_directions_without_partial_insertion() {
        let s = acknowledged();
        held_login(&s, "probe:dimension", "probe:world");
        assert!(s.serverbound(&information()).unwrap().is_none());
        for _ in 0..MAX_PACKETS - 1 {
            assert!(s.clientbound(-1, vec![0]).unwrap().is_none());
        }
        let bytes = s.pre_configuration.lock().unwrap().bytes;
        assert!(s.clientbound(-1, vec![0]).is_err());
        assert!(s.serverbound(&brand("too-many")).is_err());
        {
            let bootstrap = s.pre_configuration.lock().unwrap();
            assert_eq!(
                bootstrap.early.len() + bootstrap.deferred.len(),
                MAX_PACKETS
            );
            assert_eq!(bootstrap.bytes, bytes);
        }
        let s = acknowledged();
        held_login(&s, "probe:dimension", "probe:world");
        let room = MAX_BYTES - s.pre_configuration.lock().unwrap().bytes;
        // Unknown early frames are retained too: bound adversarial input before typed decoding.
        assert!(s.clientbound(-1, vec![0; room]).unwrap().is_none());
        assert!(s.clientbound(-1, vec![1]).is_err());
        assert_eq!(s.pre_configuration.lock().unwrap().bytes, MAX_BYTES);
        assert!(s.take_outgoing().is_empty());
    }

    #[test]
    fn bootstrap_registry_names_and_deferred_replies_are_connection_local() {
        let a = acknowledged();
        let b = acknowledged();
        held_login(&a, "probe:a", "probe:world_a");
        held_login(&b, "probe:b", "probe:world_b");
        assert!(a.serverbound(&brand("A")).unwrap().is_none());
        assert!(b.serverbound(&brand("B")).unwrap().is_none());
        features(&a, &["probe:only_a"]);
        deliver_finish(&a);
        assert!(
            a.serverbound(&body(
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            ))
            .unwrap()
            .is_none()
        );
        assert_eq!(a.take_outgoing().len(), 1);
        assert!(b.take_outgoing().is_empty());
        assert_eq!(
            (b.reading(), b.writing()),
            (State::Configuration, State::Configuration)
        );
        assert!(!b.pre_configuration.lock().unwrap().finish_delivered);
        for (s, name) in [(&a, "probe:a"), (&b, "probe:b")] {
            assert_eq!(
                s.legacy_registries
                    .lock()
                    .unwrap()
                    .get("minecraft:dimension_type")
                    .unwrap()
                    .name(0),
                Some(name)
            );
        }
        features(&b, &["probe:only_b"]);
        deliver_finish(&b);
        assert!(
            b.serverbound(&body(
                ids::configuration::serverbound::FINISH_CONFIGURATION,
                &[]
            ))
            .unwrap()
            .is_none()
        );
        let sent = b.take_outgoing();
        assert_eq!(sent.len(), 1);
        let mut r = Reader::new(&sent[0]);
        assert_eq!(r.varint().unwrap(), 13);
        assert_eq!(r.string(32767).unwrap(), "minecraft:brand");
        assert_eq!(r.string(32767).unwrap(), "B");
    }

    #[test]
    fn an_early_keepalive_still_has_a_real_old_play_reply() {
        let s = acknowledged();
        let payload = 0x1020304050607080i64.to_be_bytes().to_vec();
        let (id, delivered) = s.clientbound(35, payload.clone()).unwrap().unwrap();
        assert_eq!(id, ids::configuration::clientbound::KEEP_ALIVE);
        assert_eq!(delivered, payload);
        let sent = s
            .serverbound(&body(ids::configuration::serverbound::KEEP_ALIVE, &payload))
            .unwrap()
            .unwrap();
        let mut r = Reader::new(&sent);
        assert_eq!(r.varint().unwrap(), 18);
        assert_eq!(r.i64().unwrap(), 0x1020304050607080);
        assert_eq!(r.remaining(), 0);
        assert!(s.take_outgoing().is_empty());
        assert_eq!(s.writing(), State::Configuration);
    }
}
