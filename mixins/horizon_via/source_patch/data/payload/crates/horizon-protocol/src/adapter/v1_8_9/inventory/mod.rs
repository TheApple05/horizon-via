//! Original protocol47 inventory bridge. Output values already use pinned777 packets/Stacks.
//! SlotClick results and predictions follow official1.8.9 xi/xo/xy, not modern hashed predictions.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
    item::Stack,
    packets::play,
};
use horizon_world::nbt::Tag;
use std::collections::VecDeque;

mod click;
mod item_properties;
mod merchant;
mod mount;
pub use mount::{MountUpdate, OldSlot};
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_extra;

pub trait Context {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag>;
    fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>>;
    fn try_write_old_stack(&self, stack: Option<&Stack>, w: &mut Writer) -> Result<()>;
    /// Only creative slot updates may use the integrator's verified canonical reverse mapper.
    fn try_write_creative_stack(&self, stack: Option<&Stack>, w: &mut Writer) -> Result<()> {
        self.try_write_old_stack(stack, w)
    }
    /// Connection entity cache's pinned777 kind, checked only for EntityHorse menus.
    fn entity_kind(&self, _entity: i32) -> Result<i32> {
        Err(Error::new(
            "horse menu requires the connection entity kind callback",
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OldPacket {
    pub id: i32,
    pub payload: Vec<u8>,
}
fn packet(id: i32, w: Writer) -> OldPacket {
    OldPacket {
        id,
        payload: w.data,
    }
}
fn end(r: &Reader<'_>) -> Result<()> {
    if r.remaining() == 0 {
        Ok(())
    } else {
        Err(Error::new("trailing protocol47 inventory bytes"))
    }
}
fn boolean(r: &mut Reader<'_>) -> Result<bool> {
    match r.u8()? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(Error::new("invalid protocol47 inventory boolean")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Inventory,
    Storage {
        top: usize,
        menu: i32,
    },
    Special {
        top: usize,
        menu: i32,
        brewing: bool,
    },
    Merchant,
    Horse {
        top: usize,
        entity: i32,
        kind: i32,
    },
}
impl Kind {
    fn top(self) -> usize {
        match self {
            Self::Inventory => 9,
            Self::Storage { top, .. } | Self::Special { top, .. } | Self::Horse { top, .. } => top,
            Self::Merchant => 3,
        }
    }
    fn old_len(self) -> usize {
        if self == Self::Inventory {
            45
        } else {
            self.top() + 36
        }
    }
    fn brewing(self) -> bool {
        matches!(self, Self::Special { brewing: true, .. })
    }
    fn new_len(self) -> usize {
        self.old_len() + usize::from(self == Self::Inventory || self.brewing())
    }
    fn up(self, slot: usize) -> usize {
        slot + usize::from(self.brewing() && slot >= 4)
    }
    fn down(self, slot: usize) -> Result<usize> {
        if self == Self::Inventory && slot == 45 || self.brewing() && slot == 4 {
            return Err(Error::new(
                "virtual inventory slot has no protocol47 counterpart",
            ));
        }
        let slot = slot - usize::from(self.brewing() && slot > 4);
        if slot >= self.old_len() {
            return Err(Error::new("inventory slot exceeds protocol47 layout"));
        }
        Ok(slot)
    }
    fn parse(name: &str, slots: u8) -> Result<Self> {
        let fixed = |top, menu, brewing| {
            if slots == 0 || usize::from(slots) == top {
                Ok(Self::Special { top, menu, brewing })
            } else {
                Err(Error::new("unexpected protocol47 fixed menu slot count"))
            }
        };
        match name {
            "minecraft:chest" | "minecraft:container" => {
                let top = usize::from(slots);
                if !(9..=54).contains(&top) || top % 9 != 0 {
                    return Err(Error::new("unsupported protocol47 chest size"));
                }
                Ok(Self::Storage {
                    top,
                    menu: (top / 9 - 1) as i32,
                })
            }
            "minecraft:dispenser" | "minecraft:dropper" if slots == 0 || slots == 9 => {
                Ok(Self::Storage { top: 9, menu: 6 })
            }
            "minecraft:hopper" if slots == 0 || slots == 5 => {
                Ok(Self::Storage { top: 5, menu: 16 })
            }
            "minecraft:crafting_table" => fixed(10, 12, false),
            "minecraft:furnace" => fixed(3, 14, false),
            "minecraft:enchanting_table" => fixed(2, 13, false),
            "minecraft:anvil" => fixed(3, 8, false),
            "minecraft:brewing_stand" => fixed(4, 11, true),
            "minecraft:beacon" => fixed(1, 9, false),
            "minecraft:villager" if slots == 0 || slots == 3 => Ok(Self::Merchant),
            _ => Err(Error::new(format!(
                "unsupported protocol47 inventory type {name}"
            ))),
        }
    }
}

#[derive(Clone, Debug)]
struct Window {
    id: u8,
    kind: Kind,
    slots: Vec<Option<Stack>>,
    known: Vec<bool>,
    state: i32,
    action: i16,
}
impl Window {
    fn new(id: u8, kind: Kind) -> Self {
        Self {
            id,
            kind,
            slots: vec![None; kind.old_len()],
            known: vec![false; kind.old_len()],
            state: 0,
            action: 0,
        }
    }
    fn advance(&mut self) {
        self.state = (self.state + 1) & 32767;
    }
    fn content(&self, carried: &Option<Stack>) -> play::Clientbound {
        let mut items = vec![None; self.kind.new_len()];
        for (slot, item) in self.slots.iter().enumerate() {
            items[self.kind.up(slot)] = item.clone();
        }
        play::Clientbound::ContainerSetContent(play::ContainerSetContent {
            container: i32::from(self.id),
            state: self.state,
            items,
            carried: carried.clone(),
        })
    }
}

#[derive(Clone, Debug, Default)]
struct Drag {
    phase: u8,
    mode: u8,
    slots: Vec<usize>,
}
#[derive(Clone, Debug)]
struct Pending {
    window: u8,
    action: i16,
}
#[derive(Clone, Debug)]
struct Resync {
    window: u8,
    content: bool,
    cursor: bool,
}

#[derive(Clone, Debug)]
pub struct State {
    inventory: Window,
    open: Option<Window>,
    carried: Option<Stack>,
    creative: bool,
    drag: Drag,
    pending: VecDeque<Pending>,
    resync: Option<Resync>,
    outgoing: Vec<OldPacket>,
    incoming: Vec<play::Clientbound>,
    merchant: Option<merchant::Merchant>,
    deferred_trade: Option<i32>,
    mount_updates: Vec<MountUpdate>,
}
impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
impl State {
    pub fn new() -> Self {
        Self {
            inventory: Window::new(0, Kind::Inventory),
            open: None,
            carried: None,
            creative: false,
            drag: Drag::default(),
            pending: VecDeque::new(),
            resync: None,
            outgoing: vec![],
            incoming: vec![],
            merchant: None,
            deferred_trade: None,
            mount_updates: vec![],
        }
    }
    pub fn set_creative(&mut self, creative: bool) {
        self.creative = creative;
    }
    /// Keep the player inventory across portals:1.8 need not resend it. The integrator updates
    /// creative mode separately; only the external menu/cursor/transaction lifecycle ends here.
    pub fn on_respawn(&mut self) {
        self.close_remote_window();
    }
    pub fn close_remote_window(&mut self) {
        self.open = None;
        self.carried = None;
        self.drag = Drag::default();
        self.pending.clear();
        self.resync = None;
        self.outgoing.clear();
        self.incoming.clear();
        self.merchant = None;
        self.deferred_trade = None;
        self.mount_updates.clear();
    }
    pub fn main_hand(&self, selected: u8) -> Result<Option<&Stack>> {
        if selected >= 9 {
            return Err(Error::new("invalid selected protocol47 hotbar slot"));
        }
        let slot = 36 + usize::from(selected);
        if !self.inventory.known[slot] || self.resync.is_some() {
            return Err(Error::new(
                "protocol47 main-hand inventory is not synchronized",
            ));
        }
        Ok(self.inventory.slots[slot].as_ref())
    }
    pub fn take_outgoing(&mut self) -> Vec<OldPacket> {
        std::mem::take(&mut self.outgoing)
    }
    pub fn take_incoming(&mut self) -> Vec<play::Clientbound> {
        std::mem::take(&mut self.incoming)
    }
    /// Apply these to the entity cache before processing subsequent metadata packets.
    pub fn take_mount_updates(&mut self) -> Vec<MountUpdate> {
        std::mem::take(&mut self.mount_updates)
    }
    /// Roll back the native UI's prediction after an unsupported outgoing operation. This uses
    /// complete cached stacks, never a guessed reverse item. During authoritative resync, wait.
    pub fn correction(&self) -> Option<play::Clientbound> {
        (self.resync.is_none() && self.active().known.iter().all(|known| *known))
            .then(|| self.active().content(&self.carried))
    }
    fn active(&self) -> &Window {
        self.open.as_ref().unwrap_or(&self.inventory)
    }
    fn active_mut(&mut self) -> &mut Window {
        self.open.as_mut().unwrap_or(&mut self.inventory)
    }
    fn window_mut(&mut self, id: u8) -> Option<&mut Window> {
        if id == 0 {
            Some(&mut self.inventory)
        } else {
            self.open.as_mut().filter(|w| w.id == id)
        }
    }
    fn mirror(&mut self, source: u8) {
        let Some(open) = self.open.as_mut() else {
            return;
        };
        for i in 0..36 {
            let inv = 9 + i;
            let slot = open.kind.top() + i;
            if source == 0 {
                open.slots[slot] = self.inventory.slots[inv].clone();
                open.known[slot] = self.inventory.known[inv];
            } else {
                self.inventory.slots[inv] = open.slots[slot].clone();
                self.inventory.known[inv] = open.known[slot];
            }
        }
    }
    fn finish_resync(&mut self, cx: &impl Context) -> Result<()> {
        if self.resync.as_ref().is_some_and(|r| r.content && r.cursor) {
            self.resync = None;
            if let Some(index) = self.deferred_trade.take() {
                let packets = self.select_trade(index, cx)?;
                self.outgoing.extend(packets);
            }
        }
        Ok(())
    }

    /// None means a different bridge owns this raw old packet. Some(empty) deliberately consumes it.
    pub fn clientbound(
        &mut self,
        id: i32,
        payload: &[u8],
        cx: &impl Context,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        // Parse into a clone first: invalid framing/callback errors cannot partially mutate a session.
        let mut next = self.clone();
        let out = next.read_packet(id, payload, cx)?;
        if out.is_some() {
            *self = next;
        }
        Ok(out)
    }
    fn read_packet(
        &mut self,
        id: i32,
        payload: &[u8],
        cx: &impl Context,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        use play::Clientbound as C;
        let mut r = Reader::new(payload);
        let mut out = vec![];
        match id {
            0x2d => {
                let id = r.u8()?;
                if id == 0 || id > 127 {
                    return Err(Error::new("unsupported protocol47 window id"));
                }
                let name = r.string(32)?;
                let title = cx.text(&mut r)?;
                let slots = r.u8()?;
                let kind = if name == "EntityHorse" {
                    mount::kind(slots, r.i32()?, cx)?
                } else {
                    Kind::parse(&name, slots)?
                };
                self.open = Some(Window::new(id, kind));
                self.drag = Drag::default();
                self.pending.clear();
                self.resync = None;
                self.merchant = (kind == Kind::Merchant).then(merchant::Merchant::default);
                self.deferred_trade = None;
                self.mount_updates.clear();
                self.mirror(0);
                if let Kind::Horse { top, entity, .. } = kind {
                    out.push(C::MountScreenOpen(play::MountScreenOpen {
                        container: i32::from(id),
                        columns: if top == 17 { 5 } else { 0 },
                        entity,
                    }));
                } else {
                    let menu = match kind {
                        Kind::Storage { menu, .. } | Kind::Special { menu, .. } => menu,
                        Kind::Merchant => 19,
                        _ => unreachable!(),
                    };
                    out.push(C::OpenScreen(play::OpenScreen {
                        container: i32::from(id),
                        menu,
                        title,
                    }));
                }
                if kind.brewing() {
                    out.push(C::ContainerSetData(play::ContainerSetData {
                        container: i32::from(id),
                        id: 1,
                        value: 0,
                    }));
                }
            }
            0x2e => {
                let id = r.u8()?;
                if self.active().id == id {
                    self.open = None;
                    self.carried = None;
                    self.drag = Drag::default();
                    self.pending.clear();
                    self.resync = None;
                    self.merchant = None;
                    self.deferred_trade = None;
                    self.mount_updates.clear();
                    out.push(C::ContainerClose(play::ContainerClose {
                        container: i32::from(id),
                    }));
                    out.push(C::SetCursorItem(play::SetCursorItem { item: None }));
                }
            }
            0x2f => {
                let window = r.i8()?;
                let slot = r.i16()?;
                let start = r.position();
                let item = cx.read_old_slot(&mut r)?;
                let raw_bytes = r.since(start);
                if window == -1 && slot == -1 {
                    self.carried = item;
                    if let Some(sync) = self.resync.as_mut() {
                        sync.cursor = true;
                    }
                    self.active_mut().advance();
                    out.push(self.active().content(&self.carried));
                    self.finish_resync(cx)?;
                } else if window == -2 {
                    let mapped = match slot {
                        0..=8 => 36 + slot as usize,
                        9..=35 => slot as usize,
                        36..=39 => 8 - (slot as usize - 36),
                        _ => {
                            return Err(Error::new(
                                "invalid direct protocol47 player inventory slot",
                            ));
                        }
                    };
                    self.inventory.slots[mapped] = item.clone();
                    self.inventory.known[mapped] = true;
                    self.inventory.advance();
                    self.mirror(0);
                    out.push(C::ContainerSetSlot(play::ContainerSetSlot {
                        container: 0,
                        state: self.inventory.state,
                        slot: mapped as i16,
                        item,
                    }));
                    if self.open.is_some() {
                        self.active_mut().advance();
                        out.push(self.active().content(&self.carried));
                    }
                } else if window >= 0 {
                    let id = window as u8;
                    if let Some(Kind::Horse { entity, kind, .. }) =
                        self.window_mut(id).map(|w| w.kind)
                        && (0..=1).contains(&slot)
                    {
                        let update =
                            mount::update(entity, kind, slot as u8, raw_bytes, item.clone())?;
                        out.push(update.packet(kind));
                        self.mount_updates.push(update);
                    }
                    if let Some(w) = self.window_mut(id) {
                        let slot = usize::try_from(slot)
                            .ok()
                            .filter(|slot| *slot < w.slots.len())
                            .ok_or_else(|| Error::new("invalid protocol47 slot update index"))?;
                        w.slots[slot] = item.clone();
                        w.known[slot] = true;
                        w.advance();
                        out.push(C::ContainerSetSlot(play::ContainerSetSlot {
                            container: i32::from(id),
                            state: w.state,
                            slot: w.kind.up(slot) as i16,
                            item,
                        }));
                        self.mirror(id);
                        if id == 0 && self.open.is_some() {
                            self.active_mut().advance();
                            out.push(self.active().content(&self.carried));
                        }
                    }
                } else {
                    return Err(Error::new("invalid negative protocol47 inventory window"));
                }
            }
            0x30 => {
                let id = r.u8()?;
                let count = r.i16()?;
                let count = usize::try_from(count)
                    .ok()
                    .filter(|n| *n <= 512)
                    .ok_or_else(|| Error::new("invalid protocol47 inventory item count"))?;
                let mut items = Vec::with_capacity(count);
                for slot in 0..count {
                    let start = r.position();
                    items.push(cx.read_old_slot(&mut r)?);
                    if let Some(Kind::Horse { entity, kind, .. }) =
                        self.window_mut(id).map(|w| w.kind)
                        && slot < 2
                    {
                        let update = mount::update(
                            entity,
                            kind,
                            slot as u8,
                            r.since(start),
                            items[slot].clone(),
                        )?;
                        out.push(update.packet(kind));
                        self.mount_updates.push(update);
                    }
                }
                if let Some(w) = self.window_mut(id) {
                    if count != w.kind.old_len() {
                        return Err(Error::new(
                            "protocol47 contents do not match open menu layout",
                        ));
                    }
                    w.slots = items;
                    w.known.fill(true);
                    w.advance();
                    self.mirror(id);
                    let carried = self.carried.clone();
                    out.push(self.window_mut(id).unwrap().content(&carried));
                    if let Some(sync) = self.resync.as_mut().filter(|s| s.window == id) {
                        sync.content = true;
                    }
                    self.finish_resync(cx)?;
                }
            }
            0x31 => {
                let id = r.u8()?;
                let property = r.i16()?;
                let value = r.i16()?;
                if self.window_mut(id).is_some() {
                    out.push(C::ContainerSetData(play::ContainerSetData {
                        container: i32::from(id),
                        id: property,
                        value,
                    }));
                }
            }
            0x32 => {
                let window = r.u8()?;
                let action = r.i16()?;
                let accepted = boolean(&mut r)?;
                let matching = self
                    .pending
                    .iter()
                    .position(|p| p.window == window && p.action == action);
                if !accepted && self.window_mut(window).is_some() {
                    let mut w = Writer::new();
                    w.u8(window);
                    w.i16(action);
                    w.bool(true);
                    self.outgoing.push(packet(0x0f, w));
                }
                if let Some(index) = matching {
                    if accepted {
                        self.pending.remove(index);
                    } else {
                        self.pending.clear();
                        self.drag = Drag::default();
                        self.resync = Some(Resync {
                            window,
                            content: false,
                            cursor: false,
                        });
                    }
                }
            }
            0x3f => return self.merchant_payload(payload, cx),
            _ => return Ok(None),
        }
        end(&r)?;
        Ok(Some(out))
    }

    pub fn serverbound(
        &mut self,
        packet: &play::Serverbound,
        cx: &impl Context,
    ) -> Result<Option<Vec<OldPacket>>> {
        let mut next = self.clone();
        let out = next.write_packet(packet, cx)?;
        if out.is_some() {
            *self = next;
        }
        Ok(out)
    }
    fn write_packet(
        &mut self,
        p: &play::Serverbound,
        cx: &impl Context,
    ) -> Result<Option<Vec<OldPacket>>> {
        use play::Serverbound as S;
        let mut w = Writer::new();
        match p {
            S::SelectTrade(p) => Ok(Some(self.select_trade(p.item, cx)?)),
            S::ContainerClick(p) => {
                let id = u8::try_from(p.container)
                    .map_err(|_| Error::new("container id exceeds protocol47 range"))?;
                if id != self.active().id {
                    return Err(Error::new(
                        "click does not target the active protocol47 container",
                    ));
                }
                if self.resync.is_some() {
                    return Ok(Some(vec![]));
                }
                if self.pending.len() >= 128 {
                    return Err(Error::new(
                        "too many unconfirmed protocol47 inventory clicks",
                    ));
                }
                if self.active().known.iter().any(|known| !*known) {
                    return Err(Error::new(
                        "protocol47 container has not synchronized all slots",
                    ));
                }
                let slot = if p.slot >= 0 {
                    self.active().kind.down(p.slot as usize)? as i16
                } else {
                    p.slot
                };
                let before_slots = self.active().slots.clone();
                let before_carried = self.carried.clone();
                let mut result = self.simulate(slot, p.button, p.input, cx)?;
                let mut returned = result.returned;
                if result.resync {
                    // Unknown recipe/special-slot effects still execute authoritatively on1.8.
                    // A deliberately unequal, valid cached checksum requests vanilla's full resync.
                    let known = returned.clone().or_else(|| {
                        self.active()
                            .slots
                            .iter()
                            .flatten()
                            .next()
                            .or(self.carried.as_ref())
                            .cloned()
                    });
                    if let Some(mut mismatch) = known {
                        mismatch.count = if mismatch.count == 127 {
                            126
                        } else {
                            mismatch.count + 1
                        };
                        returned = Some(mismatch);
                    } else {
                        result.resync = false;
                    } // A completely empty menu/cursor has no item operation.
                }
                let win = self.active_mut();
                win.action = win.action.wrapping_add(1);
                let action = win.action;
                w.u8(id);
                w.i16(slot);
                w.i8(p.button);
                w.i16(action);
                w.u8(u8::try_from(p.input).map_err(|_| Error::new("invalid click input"))?);
                cx.try_write_old_stack(returned.as_ref(), &mut w)?;
                self.pending.push_back(Pending { window: id, action });
                if result.resync {
                    self.resync = Some(Resync {
                        window: id,
                        content: false,
                        cursor: false,
                    });
                } else {
                    // Drag start/add have no inventory effect. Avoid resetting the native screen's
                    // drag selection with a redundant full update between its add-slot messages.
                    let drag_only = p.input == 5 && matches!(p.button & 3, 0 | 1);
                    let unchanged =
                        before_slots == self.active().slots && before_carried == self.carried;
                    let matching_cursor = p.carried.as_ref().map(|s| (s.item, s.count))
                        == self.carried.as_ref().map(|s| (s.item, s.count));
                    if !drag_only || !unchanged || !p.changed.is_empty() || !matching_cursor {
                        self.active_mut().advance();
                        self.mirror(id);
                        self.incoming.push(self.active().content(&self.carried));
                    }
                }
                Ok(Some(vec![packet(0x0e, w)]))
            }
            S::ContainerClose(p) => {
                let id = u8::try_from(p.container)
                    .map_err(|_| Error::new("container id exceeds protocol47 range"))?;
                w.u8(id);
                // Vanilla1.8 ServerNetHandler closes the active menu without checking this byte.
                self.open = None;
                self.carried = None;
                self.drag = Drag::default();
                self.pending.clear();
                self.resync = None;
                self.merchant = None;
                self.deferred_trade = None;
                self.mount_updates.clear();
                Ok(Some(vec![packet(0x0d, w)]))
            }
            S::ContainerButtonClick(p) => {
                let id = u8::try_from(p.container)
                    .map_err(|_| Error::new("container id exceeds protocol47 range"))?;
                if id != self.active().id {
                    return Err(Error::new(
                        "button targets an inactive protocol47 container",
                    ));
                }
                w.u8(id);
                w.u8(u8::try_from(p.button)
                    .map_err(|_| Error::new("button exceeds protocol47 range"))?);
                Ok(Some(vec![packet(0x11, w)]))
            }
            S::SetCreativeModeSlot(p) => {
                if !self.creative {
                    return Err(Error::new("creative slot update outside creative mode"));
                }
                if p.slot != -1 && !(1..=44).contains(&p.slot) {
                    return Err(Error::new("creative slot has no protocol47 counterpart"));
                }
                if p.item
                    .as_ref()
                    .is_some_and(|item| !(1..=64).contains(&item.count))
                {
                    return Err(Error::new(
                        "creative stack count exceeds protocol47 server limit",
                    ));
                }
                w.i16(p.slot);
                cx.try_write_creative_stack(p.item.as_ref(), &mut w)?;
                if p.slot >= 0 {
                    let slot = p.slot as usize;
                    self.inventory.slots[slot] = p.item.clone();
                    self.inventory.known[slot] = true;
                    self.inventory.advance();
                    self.mirror(0);
                }
                Ok(Some(vec![packet(0x10, w)]))
            }
            _ => Ok(None),
        }
    }
}
