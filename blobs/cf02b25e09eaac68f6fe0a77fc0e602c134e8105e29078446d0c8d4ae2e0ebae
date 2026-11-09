//! Verified old xx and current HorseInventoryMenu both use saddle0/body1/chest2+.
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub struct OldSlot {
    pub id: i16,
    pub count: i8,
    pub damage: i16,
    pub tag: Tag,
}
impl OldSlot {
    pub(super) fn read(r: &mut Reader<'_>) -> Result<Option<Self>> {
        let id = r.i16()?;
        if id == -1 {
            return Ok(None);
        }
        if id < 0 {
            return Err(Error::new("invalid old horse/merchant item id"));
        }
        let count = r.i8()?;
        let damage = r.i16()?;
        let tag = r.named_nbt()?;
        if !matches!(tag, Tag::End | Tag::Compound(_)) {
            return Err(Error::new("old horse/merchant item NBT is not a compound"));
        }
        if count <= 0 {
            return Err(Error::new("invalid old horse/merchant item count"));
        }
        Ok(Some(Self {
            id,
            count,
            damage,
            tag,
        }))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MountUpdate {
    pub entity: i32,
    pub old_slot: u8,
    pub raw: Option<OldSlot>,
    pub item: Option<Stack>,
}
impl MountUpdate {
    pub(super) fn packet(&self, kind: i32) -> play::Clientbound {
        play::Clientbound::SetEquipment(play::SetEquipment {
            id: self.entity,
            slots: vec![(
                if self.old_slot == 0 { 7 } else { 6 },
                if self.old_slot == 1 && kind != 67 {
                    None
                } else {
                    self.item.clone()
                },
            )],
        })
    }
}
pub(super) fn kind(slots: u8, entity: i32, cx: &impl Context) -> Result<Kind> {
    let kind = cx.entity_kind(entity)?;
    if entity < 0 || !matches!(kind, 67 | 37 | 88 | 155 | 119) {
        return Err(Error::new(
            "old horse window references an unknown/nonhorse entity",
        ));
    }
    if slots != 2 && !(slots == 17 && matches!(kind, 37 | 88)) {
        return Err(Error::new(
            "old horse layout must have2 slots or17 for a chested donkey/mule",
        ));
    }
    Ok(Kind::Horse {
        top: usize::from(slots),
        entity,
        kind,
    })
}
pub(super) fn update(
    entity: i32,
    kind: i32,
    slot: u8,
    bytes: &[u8],
    item: Option<Stack>,
) -> Result<MountUpdate> {
    let mut r = Reader::new(bytes);
    let raw = OldSlot::read(&mut r)?;
    end(&r)?;
    if slot > 1 || raw.as_ref().map(|v| i32::from(v.count)) != item.as_ref().map(|v| v.count) {
        return Err(Error::new(
            "horse slot callback disagrees with exact old item",
        ));
    }
    if let Some(raw) = &raw
        && (raw.count != 1
            || (slot == 0 && raw.id != 329)
            || (slot == 1 && (!matches!(raw.id, 417..=419) || kind != 67)))
    {
        return Err(Error::new("invalid item in old horse equipment slot"));
    }
    Ok(MountUpdate {
        entity,
        old_slot: slot,
        raw,
        item,
    })
}
