//! Connection-owned entity state. Outputs are typed pinned777 packets and never old-chain bodies.
use super::{Error, Result, Spawn, boolean, entity_id, finish, objects, raw, semantic};
use crate::{buf::Reader, item::Stack, packets::play};
use std::collections::{BTreeMap, BTreeSet};

pub struct Callbacks<'a> {
    pub item: &'a mut dyn FnMut(&raw::Slot) -> Result<Stack>,
    /// Input is protocol47 wire state (block id<<4|metadata), not Block.getStateId's particle layout.
    pub block: &'a mut dyn FnMut(u16) -> Result<i32>,
    /// Return unbiased connection-local registry ids; variant holder bias is applied here.
    pub registry: &'a mut dyn FnMut(&str, &str) -> Result<i32>,
    pub component: &'a mut dyn FnMut(&str) -> Result<horizon_world::nbt::Tag>,
}
#[derive(Default, Debug)]
pub struct Output {
    pub packets: Vec<play::Clientbound>,
    pub untranslated: Vec<raw::Entry>,
    pub extras: Vec<objects::Extra>,
    pub held_item_hint: Option<i16>,
}
#[derive(Clone, Debug)]
struct Record {
    spawn: play::AddEntity,
    semantics: semantic::State,
    initial_values: semantic::Values,
    equipment: BTreeMap<u8, Option<Stack>>,
    on_ground: bool,
}
#[derive(Default, Debug)]
pub struct Entities {
    entities: BTreeMap<i32, Record>,
    rides: BTreeMap<i32, i32>,
    leashes: BTreeMap<i32, i32>,
    local_player: Option<i32>,
}
impl Entities {
    pub fn len(&self) -> usize {
        self.entities.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
    pub fn contains(&self, id: i32) -> bool {
        self.entities.contains_key(&id)
    }
    pub fn kind(&self, id: i32) -> Option<i32> {
        self.entities.get(&id).map(|value| value.spawn.kind)
    }
    pub fn uuid(&self, id: i32) -> Option<u128> {
        self.entities.get(&id).map(|value| value.spawn.uuid)
    }
    pub fn position(&self, id: i32) -> Option<[f64; 3]> {
        self.entities.get(&id).map(|value| value.spawn.position)
    }
    pub fn is_armor_stand(&self, id: i32) -> bool {
        self.kind(id) == Some(5)
    }
    /// Inventory packets supply the complete old saddle/armor stacks. Keep their
    /// semantic bits and NBT together so cached metadata cannot recreate bare items.
    pub fn remember_horse_slot(
        &mut self,
        id: i32,
        old_slot: u8,
        raw: Option<&raw::Slot>,
        canonical: Option<Stack>,
    ) -> Result<()> {
        let mut record = self
            .entities
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::new("inventory for unknown horse"))?;
        record
            .semantics
            .remember_horse_slot(old_slot, raw, canonical.clone())?;
        record.equipment.insert(
            if old_slot == 0 { 7 } else { 6 },
            if old_slot == 1 && record.spawn.kind != 67 {
                None
            } else {
                canonical
            },
        );
        self.entities.insert(id, record);
        Ok(())
    }
    pub fn equipment_snapshot(&self, id: i32, slot: u8) -> Option<Option<Stack>> {
        self.entities.get(&id)?.equipment.get(&slot).cloned()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn clear_remote(&mut self) {
        let local = self.local_player;
        self.entities.retain(|id, _| Some(*id) == local);
        self.rides.clear();
        self.leashes.clear();
    }
    /// The local player has no SpawnPlayer packet. Register it after JoinGame and keep profile identity external.
    pub fn register_local_player(&mut self, id: i32, uuid: u128) -> Result<()> {
        if id < 0 {
            return Err(Error::new("negative local entity id"));
        }
        let entity = play::AddEntity {
            id,
            uuid,
            kind: 159,
            position: [0.; 3],
            velocity: [0.; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        };
        self.entities.insert(
            id,
            Record {
                spawn: entity,
                semantics: semantic::State::new(-1)?,
                initial_values: Vec::new(),
                equipment: BTreeMap::new(),
                on_ground: false,
            },
        );
        self.local_player = Some(id);
        Ok(())
    }
    fn full_values(record: &Record, update: semantic::Values) -> semantic::Values {
        let mut values = BTreeMap::new();
        for (index, serializer, value) in record.initial_values.iter().cloned().chain(update) {
            values.insert(index, (serializer, value));
        }
        values
            .into_iter()
            .map(|(index, (serializer, value))| (index, serializer, value))
            .collect()
    }
    fn metadata_packet(record: &Record, values: semantic::Values) -> play::Clientbound {
        play::Clientbound::SetEntityData(play::SetEntityData {
            id: record.spawn.id,
            values,
        })
    }
    fn passengers(&self, vehicle: i32) -> play::Clientbound {
        play::Clientbound::SetPassengers(play::SetPassengers {
            vehicle,
            passengers: self
                .rides
                .iter()
                .filter_map(|(&child, &parent)| (parent == vehicle).then_some(child))
                .collect(),
        })
    }
    fn replay_links(&self, id: i32, out: &mut Vec<play::Clientbound>) {
        if let Some(&vehicle) = self.rides.get(&id) {
            out.push(self.passengers(vehicle));
        }
        if self.rides.values().any(|&vehicle| vehicle == id) {
            out.push(self.passengers(id));
        }
        for (&source, &dest) in &self.leashes {
            if source == id || dest == id {
                out.push(play::Clientbound::SetEntityLink(play::SetEntityLink {
                    source,
                    dest,
                }));
            }
        }
    }
    pub fn install(
        &mut self,
        spawn: Spawn,
        initial_values: semantic::Values,
        callbacks: &mut Callbacks<'_>,
    ) -> Result<Output> {
        let id = spawn.entity.id;
        let mut record = Record {
            spawn: spawn.entity,
            semantics: semantic::State::new(spawn.old_kind)?,
            initial_values,
            equipment: BTreeMap::new(),
            on_ground: false,
        };
        if spawn.old_kind == -1 {
            // Vanilla47's SpawnPlayer handler constructs this initial main-hand stack with count1,
            // damage0 and no tag. Later S04 equipment packets replace it with the complete stack.
            let item = spawn
                .held_item_hint
                .map(|id| {
                    (callbacks.item)(&raw::Slot {
                        id,
                        count: 1,
                        damage: 0,
                        tag: horizon_world::nbt::Tag::End,
                    })
                })
                .transpose()?;
            record.equipment.insert(0, item);
        }
        let mut update =
            record
                .semantics
                .apply(&spawn.metadata, &mut *callbacks.item, &mut *callbacks.block)?;
        update.resolve_registries(&mut *callbacks.registry)?;
        update.resolve_components(&mut *callbacks.component)?;
        record.spawn.kind = update.pinned_kind;
        record.equipment.extend(update.equipment);
        let values = Self::full_values(&record, update.values);
        let mut packets = Vec::new();
        if self.entities.contains_key(&id) {
            packets.push(play::Clientbound::RemoveEntities(play::RemoveEntities {
                ids: vec![id],
            }));
        }
        packets.push(play::Clientbound::AddEntity(record.spawn.clone()));
        if !values.is_empty() {
            packets.push(Self::metadata_packet(&record, values));
        }
        if !record.equipment.is_empty() {
            packets.push(play::Clientbound::SetEquipment(play::SetEquipment {
                id,
                slots: record
                    .equipment
                    .iter()
                    .map(|(&slot, item)| (slot, item.clone()))
                    .collect(),
            }));
        }
        self.entities.insert(id, record);
        // A reused id starts a new entity lifetime and must not inherit relationships or equipment.
        self.rides.remove(&id);
        self.rides.retain(|_, vehicle| *vehicle != id);
        self.leashes.remove(&id);
        self.leashes.retain(|_, holder| *holder != id);
        Ok(Output {
            packets,
            untranslated: update.untranslated,
            held_item_hint: spawn.held_item_hint,
            ..Default::default()
        })
    }
    pub fn metadata(&mut self, payload: &[u8], callbacks: &mut Callbacks<'_>) -> Result<Output> {
        let data = raw::read(payload)?;
        let mut record = self
            .entities
            .get(&data.entity)
            .cloned()
            .ok_or_else(|| Error::new("metadata for unknown protocol47 entity"))?;
        let mut update =
            record
                .semantics
                .apply(&data.entries, &mut *callbacks.item, &mut *callbacks.block)?;
        update.resolve_registries(&mut *callbacks.registry)?;
        update.resolve_components(&mut *callbacks.component)?;
        record.spawn.kind = update.pinned_kind;
        record.equipment.extend(update.equipment);
        let mut packets = Vec::new();
        if update.kind_changed {
            packets.push(play::Clientbound::RemoveEntities(play::RemoveEntities {
                ids: vec![data.entity],
            }));
            packets.push(play::Clientbound::AddEntity(record.spawn.clone()));
            if record.on_ground {
                packets.push(play::Clientbound::TeleportEntity(play::TeleportEntity {
                    id: data.entity,
                    position: record.spawn.position,
                    delta: [0.; 3],
                    yaw: f32::from(record.spawn.yaw) * (360. / 256.),
                    pitch: f32::from(record.spawn.pitch) * (360. / 256.),
                    relatives: 0xe0,
                    on_ground: true,
                }));
            }
        }
        packets.push(Self::metadata_packet(
            &record,
            Self::full_values(&record, update.values),
        ));
        if !record.equipment.is_empty() {
            packets.push(play::Clientbound::SetEquipment(play::SetEquipment {
                id: data.entity,
                slots: record
                    .equipment
                    .iter()
                    .map(|(&slot, item)| (slot, item.clone()))
                    .collect(),
            }));
        }
        if update.kind_changed {
            self.replay_links(data.entity, &mut packets);
        }
        self.entities.insert(data.entity, record);
        Ok(Output {
            packets,
            untranslated: update.untranslated,
            ..Default::default()
        })
    }
    pub fn equipment(&mut self, payload: &[u8], callbacks: &mut Callbacks<'_>) -> Result<Output> {
        let data = super::equipment(payload)?;
        if !self.entities.contains_key(&data.entity) {
            return Err(Error::new("equipment for unknown protocol47 entity"));
        }
        let item = data.item.as_ref().map(&mut *callbacks.item).transpose()?;
        self.entities
            .get_mut(&data.entity)
            .unwrap()
            .equipment
            .insert(data.slot, item.clone());
        Ok(Output {
            packets: vec![play::Clientbound::SetEquipment(play::SetEquipment {
                id: data.entity,
                slots: vec![(data.slot, item)],
            })],
            ..Default::default()
        })
    }
    pub fn attachment(&mut self, payload: &[u8]) -> Result<Output> {
        let data = objects::attachment(payload)?;
        let mut packets = Vec::new();
        if data.leash {
            match data.holder {
                Some(holder) => {
                    self.leashes.insert(data.entity, holder);
                }
                None => {
                    self.leashes.remove(&data.entity);
                }
            }
            packets.push(play::Clientbound::SetEntityLink(play::SetEntityLink {
                source: data.entity,
                // The47 consumer uses -1 detach, preserving a real holder with entity id0.
                dest: data.holder.unwrap_or(-1),
            }));
        } else {
            if let Some(holder) = data.holder {
                let mut parent = holder;
                for _ in 0..=self.rides.len() {
                    if parent == data.entity {
                        return Err(Error::new("cyclic protocol47 passengers"));
                    }
                    let Some(&next) = self.rides.get(&parent) else {
                        break;
                    };
                    parent = next;
                }
            }
            if let Some(previous) = self.rides.remove(&data.entity) {
                packets.push(self.passengers(previous));
            }
            if let Some(holder) = data.holder {
                // Vanilla1.8 permits one passenger per vehicle.
                self.rides.retain(|_, vehicle| *vehicle != holder);
                self.rides.insert(data.entity, holder);
                packets.push(self.passengers(holder));
            }
        }
        Ok(Output {
            packets,
            ..Default::default()
        })
    }
    pub fn destroy(&mut self, payload: &[u8]) -> Result<Output> {
        let ids = objects::destroy(payload)?;
        let removed = ids.iter().copied().collect::<BTreeSet<_>>();
        let affected = self
            .rides
            .iter()
            .filter_map(|(&child, &vehicle)| {
                (removed.contains(&child) && !removed.contains(&vehicle)).then_some(vehicle)
            })
            .collect::<BTreeSet<_>>();
        let links = self
            .leashes
            .iter()
            .filter_map(|(&source, &dest)| {
                (!removed.contains(&source) && removed.contains(&dest)).then_some(source)
            })
            .collect::<Vec<_>>();
        self.entities.retain(|id, _| !removed.contains(id));
        self.rides
            .retain(|child, parent| !removed.contains(child) && !removed.contains(parent));
        self.leashes
            .retain(|source, dest| !removed.contains(source) && !removed.contains(dest));
        let mut packets = vec![play::Clientbound::RemoveEntities(play::RemoveEntities {
            ids,
        })];
        packets.extend(affected.into_iter().map(|vehicle| self.passengers(vehicle)));
        packets.extend(links.into_iter().map(|source| {
            play::Clientbound::SetEntityLink(play::SetEntityLink { source, dest: -1 })
        }));
        Ok(Output {
            packets,
            ..Default::default()
        })
    }
    /// None means this module does not own the packet. UUID factory is used only for nonplayer spawns.
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        mut uuid: impl FnMut(i32) -> u128,
        callbacks: &mut Callbacks<'_>,
    ) -> Result<Option<Output>> {
        let result = match id {
            4 => self.equipment(payload, callbacks)?,
            12 => {
                let spawn = super::spawn_player(payload)?;
                self.install(spawn, Vec::new(), callbacks)?
            }
            13 => {
                let mut r = Reader::new(payload);
                let item = entity_id(&mut r)?;
                let player = entity_id(&mut r)?;
                finish(&r)?;
                let amount = self
                    .entities
                    .get(&item)
                    .and_then(|record| record.semantics.pickup_count())
                    .ok_or_else(|| Error::new("pickup before protocol47 item count"))?;
                Output {
                    packets: vec![play::Clientbound::TakeItemEntity(play::TakeItemEntity {
                        item,
                        player,
                        amount,
                    })],
                    ..Default::default()
                }
            }
            14..=17 => {
                let mut r = Reader::new(payload);
                let entity = entity_id(&mut r)?;
                let uuid = uuid(entity);
                if id == 15 {
                    self.install(super::spawn_mob(payload, uuid)?, Vec::new(), callbacks)?
                } else {
                    let converted = match id {
                        14 => objects::convert(
                            payload,
                            uuid,
                            &mut *callbacks.block,
                            &mut *callbacks.item,
                        )?,
                        16 => objects::painting(payload, uuid, &mut *callbacks.registry)?,
                        17 => objects::experience_orb(payload, uuid)?,
                        _ => unreachable!(),
                    };
                    let mut out = self.install(converted.spawn, converted.values, callbacks)?;
                    out.extras = converted.extras;
                    out
                }
            }
            18 => {
                let (id, velocity) = super::entity_velocity(payload)?;
                let record = self
                    .entities
                    .get_mut(&id)
                    .ok_or_else(|| Error::new("motion for unknown protocol47 entity"))?;
                record.spawn.velocity = velocity;
                Output {
                    packets: vec![play::Clientbound::SetEntityMotion(play::SetEntityMotion {
                        id,
                        velocity,
                    })],
                    ..Default::default()
                }
            }
            19 => self.destroy(payload)?,
            20 => {
                let mut r = Reader::new(payload);
                entity_id(&mut r)?;
                finish(&r)?;
                Output::default()
            }
            21 | 23 => {
                let data = super::relative_move(payload, id == 23)?;
                let record = self
                    .entities
                    .get_mut(&data.entity)
                    .ok_or_else(|| Error::new("move for unknown protocol47 entity"))?;
                record.spawn.position = play::VecDelta::apply(record.spawn.position, data.delta);
                record.on_ground = data.on_ground;
                if let Some((yaw, pitch)) = data.rotation {
                    record.spawn.yaw = yaw;
                    record.spawn.pitch = pitch;
                }
                let current = play::MoveEntity {
                    id: data.entity,
                    delta: Some(play::VecDelta::Linear(data.delta)),
                    rotation: data.rotation,
                    on_ground: data.on_ground,
                };
                Output {
                    packets: vec![if id == 23 {
                        play::Clientbound::MoveEntityPosRot(play::MoveEntityPosRot(current))
                    } else {
                        play::Clientbound::MoveEntityPos(play::MoveEntityPos(current))
                    }],
                    ..Default::default()
                }
            }
            22 => {
                let mut r = Reader::new(payload);
                let id = entity_id(&mut r)?;
                let yaw = r.i8()?;
                let pitch = r.i8()?;
                let on_ground = boolean(&mut r)?;
                finish(&r)?;
                let record = self
                    .entities
                    .get_mut(&id)
                    .ok_or_else(|| Error::new("rotation for unknown protocol47 entity"))?;
                record.spawn.yaw = yaw;
                record.spawn.pitch = pitch;
                record.on_ground = on_ground;
                Output {
                    packets: vec![play::Clientbound::MoveEntityRot(play::MoveEntityRot(
                        play::MoveEntity {
                            id,
                            delta: None,
                            rotation: Some((yaw, pitch)),
                            on_ground,
                        },
                    ))],
                    ..Default::default()
                }
            }
            24 => {
                let current = super::teleport(payload)?;
                let record = self
                    .entities
                    .get_mut(&current.id)
                    .ok_or_else(|| Error::new("teleport for unknown protocol47 entity"))?;
                record.spawn.position = current.position;
                record.on_ground = current.on_ground;
                record.spawn.yaw = (current.yaw * (256. / 360.)) as i8;
                record.spawn.pitch = (current.pitch * (256. / 360.)) as i8;
                Output {
                    packets: vec![play::Clientbound::TeleportEntity(current)],
                    ..Default::default()
                }
            }
            25 => {
                let (id, head_yaw) = super::head_rotation(payload)?;
                let record = self
                    .entities
                    .get_mut(&id)
                    .ok_or_else(|| Error::new("head rotation for unknown protocol47 entity"))?;
                record.spawn.head_yaw = head_yaw;
                Output {
                    packets: vec![play::Clientbound::RotateHead(play::RotateHead {
                        id,
                        head_yaw,
                    })],
                    ..Default::default()
                }
            }
            26 => {
                let mut r = Reader::new(payload);
                let id = r.i32()?;
                let event = r.i8()?;
                finish(&r)?;
                Output {
                    packets: vec![if event == 2 {
                        play::Clientbound::HurtAnimation(play::HurtAnimation { id, yaw: 0. })
                    } else {
                        play::Clientbound::EntityEvent(play::EntityEvent { id, event })
                    }],
                    ..Default::default()
                }
            }
            27 => self.attachment(payload)?,
            28 => self.metadata(payload, callbacks)?,
            29 => Output {
                packets: vec![play::Clientbound::UpdateMobEffect(super::effects::update(
                    payload,
                )?)],
                ..Default::default()
            },
            30 => Output {
                packets: vec![play::Clientbound::RemoveMobEffect(super::effects::remove(
                    payload,
                )?)],
                ..Default::default()
            },
            _ => return Ok(None),
        };
        Ok(Some(result))
    }
}
