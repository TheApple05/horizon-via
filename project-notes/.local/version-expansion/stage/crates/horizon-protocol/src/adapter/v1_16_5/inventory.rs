//! Connection-local 754 windows and acknowledged clicks; item identities retain source NBT.
use super::*;
use crate::item::{Hashed, Stack};

#[derive(Clone)]
struct Window {
    id: u8,
    state: i32,
    action: i16,
    slots: Vec<Option<Stack>>,
    known: bool,
    revisions: Vec<u64>,
}

impl Window {
    fn new(id: u8, len: usize) -> Self {
        Self {
            id,
            state: 0,
            action: 0,
            slots: vec![None; len],
            known: false,
            revisions: vec![0; len],
        }
    }
    fn advance(&mut self) {
        self.state = self.state.wrapping_add(1) & 0x7fff_ffff;
    }
    fn content(&self, carried: &Option<Stack>) -> play::Clientbound {
        play::Clientbound::ContainerSetContent(play::ContainerSetContent {
            container: i32::from(self.id),
            state: self.state,
            items: self.slots.clone(),
            carried: carried.clone(),
        })
    }
}
struct Pending {
    window: u8,
    action: i16,
    before: Vec<Option<Stack>>,
    carried: Option<Stack>,
    revisions: Vec<u64>,
    cursor_revision: u64,
}
struct Resync {
    window: u8,
    content: bool,
    cursor: bool,
}

pub(super) struct Inventory {
    player: Window,
    open: Option<Window>,
    carried: Option<Stack>,
    pending: VecDeque<Pending>,
    retired: VecDeque<(u8, i16)>,
    waiting: Option<Resync>,
    revision: u64,
    cursor_revision: u64,
}
impl Default for Inventory {
    fn default() -> Self {
        Self {
            player: Window::new(0, 46),
            open: None,
            carried: None,
            pending: VecDeque::new(),
            retired: VecDeque::new(),
            waiting: None,
            revision: 0,
            cursor_revision: 0,
        }
    }
}
impl Inventory {
    fn retire_pending(&mut self) {
        self.retired
            .extend(self.pending.drain(..).map(|p| (p.window, p.action)));
    }
    fn next_revision(&mut self) -> Result<u64> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| Error::new("old inventory revision exhausted"))?;
        Ok(self.revision)
    }
    fn finish_resync(&mut self) {
        if self
            .waiting
            .as_ref()
            .is_some_and(|sync| sync.content && sync.cursor)
        {
            self.waiting = None;
        }
    }
    pub(super) fn book(&self, slot: i32) -> Result<Option<Stack>> {
        if !self.player.known {
            return Err(Error::new("book edit precedes inventory synchronization"));
        }
        let slot = match slot {
            0..=8 => slot as usize + 36,
            40 => 45,
            _ => return Err(Error::new("book edit slot is not a hand slot")),
        };
        Ok(self.player.slots[slot].clone())
    }
    fn active(&self) -> &Window {
        self.open.as_ref().unwrap_or(&self.player)
    }
    fn active_mut(&mut self) -> &mut Window {
        self.open.as_mut().unwrap_or(&mut self.player)
    }
    fn window_mut(&mut self, id: u8) -> Option<&mut Window> {
        if id == 0 {
            Some(&mut self.player)
        } else {
            self.open.as_mut().filter(|w| w.id == id)
        }
    }
    pub(super) fn respawn(&mut self) {
        self.open = None;
        self.carried = None;
        self.retire_pending();
        self.waiting = None;
    }
    fn mirror(&mut self, id: u8) {
        let Some(open) = &mut self.open else { return };
        let top = open.slots.len() - 36;
        for n in 0..36 {
            if id == 0 {
                open.slots[top + n] = self.player.slots[9 + n].clone();
                open.revisions[top + n] = self.player.revisions[9 + n];
            } else {
                self.player.slots[9 + n] = open.slots[top + n].clone();
                self.player.revisions[9 + n] = open.revisions[top + n];
            }
        }
    }
    pub(super) fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
    ) -> Result<Option<(Vec<play::Clientbound>, Vec<Vec<u8>>)>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let mut out = vec![];
        let mut extra = vec![];
        match id {
            17 => {
                let window = r.u8()?;
                let action = r.i16()?;
                let accepted = boolean(&mut r)?;
                finish(&r)?;
                let index = self
                    .pending
                    .iter()
                    .position(|p| p.window == window && p.action == action);
                let Some(index) = index else {
                    let index = self
                        .retired
                        .iter()
                        .position(|v| *v == (window, action))
                        .ok_or_else(|| {
                            Error::new("unexpected 1.16.5 transaction acknowledgement")
                        })?;
                    self.retired.remove(index);
                    if !accepted {
                        let mut ack = Writer::new();
                        ack.u8(window);
                        ack.i16(action);
                        ack.bool(true);
                        extra.push(wire(7, ack));
                        if self.active().id == window {
                            self.waiting = Some(Resync {
                                window,
                                content: false,
                                cursor: false,
                            });
                        }
                    }
                    return Ok(Some((out, extra)));
                };
                let pending = self.pending.remove(index).unwrap();
                if !accepted {
                    if let Some(win) = self.window_mut(window) {
                        // A server slot update newer than the rejected prediction remains authoritative.
                        for (i, item) in pending.before.into_iter().enumerate() {
                            if win.revisions[i] == pending.revisions[i] {
                                win.slots[i] = item;
                            }
                        }
                        win.advance();
                    }
                    if self.cursor_revision == pending.cursor_revision {
                        self.carried = pending.carried;
                    }
                    self.pending.retain(|p| p.window != window);
                    self.waiting = Some(Resync {
                        window,
                        content: false,
                        cursor: false,
                    });
                    self.mirror(window);
                    let mut ack = Writer::new();
                    ack.u8(window);
                    ack.i16(action);
                    ack.bool(true);
                    extra.push(wire(7, ack));
                    out.push(self.active().content(&self.carried));
                }
            }
            18 => {
                let id = r.u8()?;
                finish(&r)?;
                if self.open.as_ref().is_some_and(|w| w.id == id) {
                    self.respawn();
                }
                out.push(C::ContainerClose(play::ContainerClose {
                    container: i32::from(id),
                }));
            }
            19 => {
                let id = r.u8()?;
                let count = r.i16()?;
                if !(0..=512).contains(&count) {
                    return Err(Error::new("invalid old inventory length"));
                }
                let items = (0..count)
                    .map(|_| read_slot(&mut r))
                    .collect::<Result<Vec<_>>>()?;
                finish(&r)?;
                if id != 0 && self.open.as_ref().is_none_or(|w| w.id != id) {
                    return Ok(Some((vec![], vec![])));
                }
                let revision = self.next_revision()?;
                let win = self
                    .window_mut(id)
                    .ok_or_else(|| Error::new("old inventory update targets unknown window"))?;
                if items.len() != win.slots.len() {
                    return Err(Error::new("old inventory length disagrees with menu"));
                }
                win.slots = items;
                win.revisions.fill(revision);
                win.known = true;
                win.advance();
                if let Some(sync) = &mut self.waiting
                    && sync.window == id
                {
                    sync.content = true;
                }
                self.finish_resync();
                self.mirror(id);
                let carried = self.carried.clone();
                out.push(self.window_mut(id).unwrap().content(&carried));
            }
            20 => {
                let container = i32::from(r.u8()?);
                let id = r.i16()?;
                let value = r.i16()?;
                out.push(C::ContainerSetData(play::ContainerSetData {
                    container,
                    id,
                    value,
                }));
            }
            21 => {
                let id = r.i8()?;
                let slot = r.i16()?;
                let item = read_slot(&mut r)?;
                finish(&r)?;
                let revision = self.next_revision()?;
                if id == -1 && slot == -1 {
                    self.carried = item;
                    self.cursor_revision = revision;
                    if let Some(sync) = &mut self.waiting {
                        sync.cursor = true;
                    }
                    self.finish_resync();
                    out.push(C::SetCursorItem(play::SetCursorItem {
                        item: self.carried.clone(),
                    }));
                } else if id == -2 {
                    let mapped = match slot {
                        0..=8 => 36 + slot as usize,
                        9..=35 => slot as usize,
                        36..=39 => 44 - slot as usize,
                        40 => 45,
                        _ => return Err(Error::new("invalid old direct inventory slot")),
                    };
                    self.player.slots[mapped] = item.clone();
                    self.player.revisions[mapped] = revision;
                    self.player.advance();
                    self.mirror(0);
                    out.push(C::ContainerSetSlot(play::ContainerSetSlot {
                        container: 0,
                        state: self.player.state,
                        slot: mapped as i16,
                        item,
                    }));
                } else {
                    let id = u8::try_from(id).map_err(|_| Error::new("invalid old window id"))?;
                    if id != 0 && self.open.as_ref().is_none_or(|w| w.id != id) {
                        return Ok(Some((vec![], vec![])));
                    }
                    let win = self
                        .window_mut(id)
                        .ok_or_else(|| Error::new("old slot update targets unknown window"))?;
                    let target = usize::try_from(slot)
                        .ok()
                        .and_then(|s| win.slots.get_mut(s))
                        .ok_or_else(|| Error::new("old slot index outside window"))?;
                    *target = item.clone();
                    win.revisions[slot as usize] = revision;
                    win.advance();
                    let state = win.state;
                    self.mirror(id);
                    out.push(C::ContainerSetSlot(play::ContainerSetSlot {
                        container: i32::from(id),
                        state,
                        slot,
                        item,
                    }));
                }
            }
            30 => {
                let id = r.u8()?;
                let size = r.varint()?;
                let entity = r.i32()?;
                if !(2..=17).contains(&size) || (size - 2) % 3 != 0 {
                    return Err(Error::new("invalid old horse menu length"));
                }
                finish(&r)?;
                self.open = Some(Window::new(id, size as usize + 36));
                self.retire_pending();
                self.waiting = None;
                self.mirror(0);
                out.push(C::MountScreenOpen(play::MountScreenOpen {
                    container: i32::from(id),
                    columns: if size > 2 { (size - 2) / 3 } else { 0 },
                    entity,
                }));
            }
            45 => {
                let id = r.varint()?;
                let menu = r.varint()?;
                let title = text(&mut r)?;
                finish(&r)?;
                let id = u8::try_from(id)
                    .ok()
                    .filter(|v| *v > 0 && *v <= 127)
                    .ok_or_else(|| Error::new("invalid old menu id"))?;
                let top = match menu {
                    0..=5 => (menu + 1) * 9,
                    6 => 9,
                    7 => 3,
                    8 => 1,
                    9 => 3,
                    10 => 5,
                    11 => 10,
                    12 => 2,
                    13 => 3,
                    14 => 3,
                    15 => 5,
                    16 => 1,
                    17 => 4,
                    18 => 3,
                    19 => 27,
                    20 => return Err(Error::new("1.16.5 two-input smithing is not implemented")),
                    21 => 3,
                    22 => 3,
                    23 => 2,
                    _ => return Err(Error::new("unknown old menu kind")),
                };
                let mapped = up(tables::MENU_UP, menu)?;
                self.open = Some(Window::new(id, top as usize + 36));
                self.carried = None;
                self.retire_pending();
                self.waiting = None;
                self.mirror(0);
                out.push(C::OpenScreen(play::OpenScreen {
                    container: i32::from(id),
                    menu: mapped,
                    title,
                }));
            }
            38 => {
                let container = r.varint()?;
                let n = r.u8()?;
                let mut offers = vec![];
                for _ in 0..n {
                    let cost_a =
                        read_slot(&mut r)?.ok_or_else(|| Error::new("empty old trade cost"))?;
                    let result =
                        read_slot(&mut r)?.ok_or_else(|| Error::new("empty old trade result"))?;
                    let cost_b = if boolean(&mut r)? {
                        read_slot(&mut r)?
                    } else {
                        None
                    };
                    offers.push(play::NetOffer {
                        cost_a: cost_a.into(),
                        result,
                        cost_b: cost_b.map(Into::into),
                        out_of_stock: boolean(&mut r)?,
                        uses: r.i32()?,
                        max_uses: r.i32()?,
                        xp: r.i32()?,
                        special_price: r.i32()?,
                        price_multiplier: r.f32()?,
                        demand: r.i32()?,
                    });
                }
                out.push(C::MerchantOffers(play::MerchantOffers {
                    container,
                    offers,
                    level: r.varint()?,
                    xp: r.varint()?,
                    show_progress: boolean(&mut r)?,
                    can_restock: boolean(&mut r)?,
                }));
            }
            _ => return Ok(None),
        }
        finish(&r)?;
        Ok(Some((out, extra)))
    }

    fn resolve(&self, hashed: &Option<Hashed>) -> Result<Option<Stack>> {
        let Some(hashed) = hashed else {
            return Ok(None);
        };
        if !(1..=127).contains(&hashed.count) {
            return Err(Error::new("invalid predicted old stack count"));
        }
        struct Names;
        impl crate::item::Names for Names {
            fn name(&self, registry: &str, id: i32) -> Option<String> {
                registry_ids::legacy_registry_name(registry, id)
            }
        }
        let mut found = None;
        for stack in self
            .player
            .slots
            .iter()
            .chain(self.active().slots.iter())
            .chain(std::iter::once(&self.carried))
            .flatten()
        {
            let mut candidate = stack.clone();
            candidate.count = hashed.count;
            if Hashed::create(&candidate, &Names) == *hashed {
                if found.as_ref().is_some_and(|v| v != &candidate) {
                    return Err(Error::new("ambiguous predicted old item"));
                }
                found = Some(candidate);
            }
        }
        found
            .map(Some)
            .ok_or_else(|| Error::new("predicted item has no verified old stack"))
    }

    pub(super) fn serverbound(
        &mut self,
        id: i32,
        payload: &[u8],
    ) -> Result<Option<(Option<Vec<u8>>, Vec<play::Clientbound>, bool)>> {
        use play::Serverbound as S;
        if !matches!(
            id,
            ids::play::serverbound::CONTAINER_CLICK
                | ids::play::serverbound::CONTAINER_CLOSE
                | ids::play::serverbound::CONTAINER_BUTTON_CLICK
                | ids::play::serverbound::SET_CREATIVE_MODE_SLOT
        ) {
            return Ok(None);
        }
        let packet = play::Serverbound::decode(id, payload)?;
        let mut w = Writer::new();
        match packet {
            S::ContainerClose(p) => {
                let id = u8::try_from(p.container)
                    .map_err(|_| Error::new("old close id out of range"))?;
                w.u8(id);
                self.respawn();
                Ok(Some((Some(wire(10, w)), vec![], false)))
            }
            S::ContainerButtonClick(p) => {
                w.u8(u8::try_from(p.container)
                    .map_err(|_| Error::new("old menu id out of range"))?);
                w.u8(u8::try_from(p.button)
                    .map_err(|_| Error::new("old menu button out of range"))?);
                Ok(Some((Some(wire(8, w)), vec![], false)))
            }
            S::SetCreativeModeSlot(p) => {
                if !(0..46).contains(&p.slot) {
                    return Err(Error::new("old creative slot out of range"));
                }
                w.i16(p.slot);
                if write_slot(&p.item, &mut w).is_err() {
                    return Ok(Some((None, vec![self.player.content(&self.carried)], true)));
                }
                self.player.slots[p.slot as usize] = p.item;
                self.player.advance();
                self.mirror(0);
                Ok(Some((Some(wire(40, w)), vec![], false)))
            }
            S::ContainerClick(p) => {
                if self.waiting.is_some()
                    || !self.active().known
                    || p.container != i32::from(self.active().id)
                    || p.state != self.active().state
                    || self.pending.len() >= 128
                    || self.pending.len() + self.retired.len() >= 1024
                {
                    return Ok(Some((
                        None,
                        vec![self.active().content(&self.carried)],
                        true,
                    )));
                }
                let before = self.active().slots.clone();
                let carried = self.carried.clone();
                let outcome = (|| -> Result<(Vec<Option<Stack>>, Option<Stack>)> {
                    if !(0..=6).contains(&p.input) || p.slot < 0 && p.slot != -999 {
                        return Err(Error::new("invalid old inventory click"));
                    }
                    if p.slot >= 0 && p.slot as usize >= before.len() {
                        return Err(Error::new("old click slot outside menu"));
                    }
                    let mut slots = before.clone();
                    for (slot, hash) in &p.changed {
                        let slot = usize::try_from(*slot)
                            .map_err(|_| Error::new("negative predicted slot"))?;
                        if slot >= slots.len() {
                            return Err(Error::new("predicted slot outside menu"));
                        }
                        slots[slot] = self.resolve(hash)?;
                    }
                    Ok((slots, self.resolve(&p.carried)?))
                })();
                let Ok((slots, new_carried)) = outcome else {
                    return Ok(Some((
                        None,
                        vec![self.active().content(&self.carried)],
                        true,
                    )));
                };
                let returned = if matches!(p.input, 0 | 1) && p.slot >= 0 {
                    before[p.slot as usize].clone()
                } else {
                    None
                };
                let window = self.active().id;
                let action = self.active().action.wrapping_add(1);
                w.u8(window);
                w.i16(p.slot);
                w.i8(p.button);
                w.i16(action);
                w.varint(p.input);
                write_slot(&returned, &mut w)?;
                self.pending.push_back(Pending {
                    window,
                    action,
                    before,
                    carried,
                    revisions: self.active().revisions.clone(),
                    cursor_revision: self.cursor_revision,
                });
                let active = self.active_mut();
                active.slots = slots;
                active.action = action;
                active.advance();
                self.carried = new_carried;
                self.mirror(window);
                Ok(Some((
                    Some(wire(9, w)),
                    vec![self.active().content(&self.carried)],
                    false,
                )))
            }
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn diamond(count: i32) -> Stack {
        let raw = tables::ITEM_NAMES
            .iter()
            .position(|v| *v == "minecraft:diamond")
            .unwrap();
        Stack::new(tables::ITEM_UP[raw], count)
    }
    fn click(inventory: &mut Inventory) -> (Option<Vec<u8>>, Vec<play::Clientbound>, bool) {
        let p = play::Serverbound::ContainerClick(play::ContainerClick {
            container: i32::from(inventory.active().id),
            state: inventory.active().state,
            slot: 10,
            button: 0,
            input: 0,
            changed: vec![(10, None)],
            carried: Some(Hashed::of(&diamond(5))),
        })
        .encode();
        let (id, payload) = packet(p);
        inventory.serverbound(id, &payload).unwrap().unwrap()
    }
    fn reject(inventory: &mut Inventory) -> Vec<Vec<u8>> {
        let mut w = Writer::new();
        w.u8(0);
        w.i16(1);
        w.bool(false);
        inventory.clientbound(17, &w.data).unwrap().unwrap().1
    }
    fn content(inventory: &mut Inventory) {
        let mut w = Writer::new();
        w.u8(0);
        w.i16(46);
        for i in 0..46 {
            write_slot(&if i == 10 { Some(diamond(5)) } else { None }, &mut w).unwrap();
        }
        inventory.clientbound(19, &w.data).unwrap();
    }
    fn cursor(inventory: &mut Inventory, value: Option<Stack>) {
        let mut w = Writer::new();
        w.i8(-1);
        w.i16(-1);
        write_slot(&value, &mut w).unwrap();
        inventory.clientbound(21, &w.data).unwrap();
    }
    #[test]
    fn rejected_click_waits_for_both_full_content_and_cursor_in_either_order() {
        let _names = registry_ids::legacy_registries(names());
        let _cache =
            registry_ids::legacy_items(Arc::new(std::sync::Mutex::new(Default::default())));
        for cursor_first in [false, true] {
            let mut inventory = Inventory::default();
            inventory.player.known = true;
            inventory.player.slots[10] = Some(diamond(5));
            assert!(click(&mut inventory).0.is_some());
            assert_eq!(reject(&mut inventory), vec![vec![7, 0, 0, 1, 1]]);
            if cursor_first {
                cursor(&mut inventory, None);
            } else {
                content(&mut inventory);
            }
            let blocked = click(&mut inventory);
            assert!(blocked.0.is_none() && blocked.2);
            if cursor_first {
                content(&mut inventory);
            } else {
                cursor(&mut inventory, None);
            }
            assert!(inventory.waiting.is_none());
            assert!(click(&mut inventory).0.is_some());
        }
    }
    #[test]
    fn rejecting_a_prediction_preserves_later_authoritative_slot_and_cursor_updates() {
        let _names = registry_ids::legacy_registries(names());
        let _cache =
            registry_ids::legacy_items(Arc::new(std::sync::Mutex::new(Default::default())));
        let mut inventory = Inventory::default();
        inventory.player.known = true;
        inventory.player.slots[10] = Some(diamond(5));
        assert!(click(&mut inventory).0.is_some());
        let mut w = Writer::new();
        w.u8(0);
        w.i16(10);
        write_slot(&Some(diamond(3)), &mut w).unwrap();
        inventory.clientbound(21, &w.data).unwrap();
        cursor(&mut inventory, Some(diamond(2)));
        reject(&mut inventory);
        assert_eq!(inventory.player.slots[10].as_ref().unwrap().count, 3);
        assert_eq!(inventory.carried.as_ref().unwrap().count, 2);
    }
    #[test]
    fn late_click_acknowledgements_and_contents_after_local_close_do_not_reopen_the_menu() {
        let _names = registry_ids::legacy_registries(names());
        let _cache =
            registry_ids::legacy_items(Arc::new(std::sync::Mutex::new(Default::default())));
        for (window, accepted) in [(0, true), (0, false), (2, true), (2, false)] {
            let mut inventory = Inventory::default();
            inventory.player.known = true;
            inventory.player.slots[10] = Some(diamond(5));
            if window != 0 {
                let mut open = Window::new(window, 45);
                open.known = true;
                open.slots[10] = Some(diamond(5));
                inventory.open = Some(open);
            }
            assert!(click(&mut inventory).0.is_some());
            let (id, payload) = packet(
                play::Serverbound::ContainerClose(play::ContainerClose {
                    container: i32::from(window),
                })
                .encode(),
            );
            inventory.serverbound(id, &payload).unwrap();
            let mut ack = Writer::new();
            ack.u8(window);
            ack.i16(1);
            ack.bool(accepted);
            let (_, extra) = inventory.clientbound(17, &ack.data).unwrap().unwrap();
            assert_eq!(extra.is_empty(), accepted);
            assert!(inventory.retired.is_empty() && inventory.open.is_none());
            if window != 0 {
                let mut contents = Writer::new();
                contents.u8(window);
                contents.i16(45);
                for _ in 0..45 {
                    contents.bool(false);
                }
                assert!(
                    inventory
                        .clientbound(19, &contents.data)
                        .unwrap()
                        .unwrap()
                        .0
                        .is_empty()
                );
                let mut slot = Writer::new();
                slot.u8(window);
                slot.i16(10);
                slot.bool(false);
                assert!(
                    inventory
                        .clientbound(21, &slot.data)
                        .unwrap()
                        .unwrap()
                        .0
                        .is_empty()
                );
                assert!(inventory.waiting.is_none());
            }
        }
    }
}
