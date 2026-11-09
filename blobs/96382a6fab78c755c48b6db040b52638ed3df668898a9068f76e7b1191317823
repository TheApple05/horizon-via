//! A server's workstation menus on this client: the server opens the menu
//! (`ClientboundOpenScreenPacket` with the station's `MenuType`), the
//! station screen draws it, and its clicks and buttons are predicted on the
//! menu engine (`Game::menu_session`) and sent, as
//! `MultiPlayerGameMode.handleContainerInput` and `handleInventoryButtonClick`
//! do. The menu's client-side `slotsChanged` follows the server's slots, and
//! its data slots come in `ClientboundContainerSetDataPacket`.
use super::{
    Game, Screen,
    stations::{Kind, Station},
};
use horizon_protocol::packets::play::{self, Serverbound as S};
use std::collections::HashSet;

/// Read the entire canonical update before publishing a 1.16.5 recipe reload.
/// Other server versions keep their existing recipe handling.
pub(super) fn legacy_smithing_additions(
    data: &[u8],
    item_name: impl Fn(i32) -> Option<String>,
) -> horizon_protocol::Result<HashSet<String>> {
    use horizon_protocol::{
        Error,
        buf::Reader,
        packets::recipes::{HolderSet, SlotDisplay},
    };
    let mut r = Reader::new(data);
    let mut properties = HashSet::new();
    let mut additions = HashSet::new();
    for _ in 0..r.count(256)? {
        let name = r.string(32767)?;
        if !properties.insert(name.clone()) {
            return Err(Error::new("duplicate recipe property set"));
        }
        for _ in 0..r.count(65536)? {
            let item =
                item_name(r.varint()?).ok_or_else(|| Error::new("unknown recipe property item"))?;
            if name == "minecraft:smithing_addition" {
                additions.insert(item);
            }
        }
    }
    for _ in 0..r.count(65536)? {
        if let HolderSet::Items(ids) = HolderSet::read(&mut r)? {
            for id in ids {
                if item_name(id).is_none() {
                    return Err(Error::new("unknown stonecutting ingredient item"));
                }
            }
        }
        SlotDisplay::read(&mut r)?;
    }
    if r.remaining() != 0 {
        return Err(Error::new("trailing recipe update bytes"));
    }
    Ok(additions)
}

impl Game<'_> {
    pub(super) fn legacy_smithing(&self) -> bool {
        self.station
            .as_ref()
            .is_some_and(|s| s.kind == Kind::Smithing && s.legacy_smithing_additions.is_some())
            && self.remote.as_ref().is_some_and(|r| {
                r.session.server_version() == horizon_protocol::adapter::ServerVersion::V1_16_5
            })
    }

    /// The server opened workstation menu `menu` in window `container`.
    pub(super) fn remote_open_station(
        &mut self,
        container: i32,
        menu: String,
        kind: Kind,
        title: horizon_client::text::Text,
        pos: [i32; 3],
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((container, menu));
            r.menu_title = Some(title);
        }
        self.craft_size = 2;
        self.container = None;
        let mut station = Station::new_kind(kind, pos);
        if kind == Kind::Smithing {
            station.legacy_smithing_additions = self
                .remote
                .as_ref()
                .filter(|r| {
                    r.session.server_version() == horizon_protocol::adapter::ServerVersion::V1_16_5
                })
                .map(|r| r.legacy_smithing_additions.clone());
        }
        self.station = Some(station);
        self.screen = Screen::Inventory;
        self.capture(false);
    }

    /// Window `container` is the server's workstation menu, open here.
    pub(super) fn remote_station_open(&self, container: i32) -> bool {
        self.station.is_some()
            && self
                .remote
                .as_ref()
                .and_then(|r| r.menu.as_ref())
                .is_some_and(|(id, name)| {
                    *id == container && Self::remote_station_kind(name).is_some()
                })
    }

    /// The server set station slot `internal`: the client menu's
    /// `Slot.set` runs its container's `setChanged`, so the loom lists the
    /// patterns its inputs allow and the stonecutter the recipes of its input.
    pub(super) fn remote_station_slot_changed(&mut self, internal: usize) {
        if !self.station.as_ref().is_some_and(|s| {
            matches!(
                s.kind,
                Kind::Loom
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Cartography
            )
        }) {
            return;
        }
        self.menu_session(|menu, env, to_menu| {
            let i = to_menu(internal)?;
            let stack = menu.slot(i).cloned();
            menu.set_slot(env, i, stack);
            Some(true)
        });
        // AnvilScreen.slotChanged: the box shows the new input's name, which
        // its responder sends.
        self.remote_anvil_rename();
    }

    /// `AbstractContainerMenu.setData` from the server.
    pub(super) fn remote_station_data(&mut self, id: usize, value: i16) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        match (s.kind, id) {
            // LoomMenu.selectedBannerPatternIndex.
            (Kind::Loom, 0) => s.loom.selected = usize::try_from(value).ok(),
            // StonecutterMenu.selectedRecipeIndex.
            (Kind::Stonecutter, 0) => s.selected = usize::try_from(value).ok(),
            // AnvilMenu.cost.
            (Kind::Anvil, 0) => s.anvil.cost = i32::from(value),
            // EnchantmentMenu: costs, the seed (the screen's glyph names),
            // the clues' enchantments and levels.
            (Kind::Enchanting, 0..3) => s.offers.costs[id] = i32::from(value),
            (Kind::Enchanting, 3) => self.level.xp_seed = i32::from(value),
            (Kind::Enchanting, 4..7) => {
                let name = self.remote.as_ref().and_then(|r| {
                    r.registries
                        .name("minecraft:enchantment", i32::from(value))
                        .map(std::sync::Arc::<str>::from)
                });
                let level = s.offers.clues[id - 4].as_ref().map_or(1, |(_, l)| *l);
                s.offers.clues[id - 4] = name.map(|n| (n, level));
            }
            (Kind::Enchanting, 7..10) => {
                if let Some((_, level)) = s.offers.clues[id - 7].as_mut() {
                    *level = value.max(0) as u32;
                }
            }
            // BeaconMenu: the levels, then the powers (BeaconScreen's
            // dataChanged shows the menu's).
            (Kind::Beacon, 0) => s.beacon_levels = Some(i32::from(value)),
            (Kind::Beacon, 1 | 2) => {
                let effect = self.remote_effect_id(i32::from(value) - 1);
                let index =
                    effect.and_then(|e| super::beacon::EFFECTS.iter().position(|(n, _)| *n == e));
                if let Some(s) = self.station.as_mut() {
                    s.beacon[id - 1] = index;
                }
            }
            _ => {}
        }
    }

    /// `AnvilScreen.onNameChanged` on a server: the rename box's text (blank
    /// when it is the input's own name) goes to the server when the menu's
    /// `setItemName` takes it.
    pub(super) fn remote_anvil_rename(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some(s) = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil && s.inputs[0].is_some())
        else {
            return;
        };
        let name = self.anvil_item_name(s);
        if s.anvil_sent.as_deref() == Some(&name) {
            return;
        }
        if let Some(s) = self.station.as_mut() {
            s.anvil_sent = Some(name.clone());
        }
        self.send(S::RenameItem(play::RenameItem { name }));
    }

    /// `MultiPlayerGameMode.handleInventoryButtonClick`: the screen already
    /// ran `clickMenuButton` on its own menu.
    pub(super) fn remote_station_button(&self, button: i32) {
        if let Some((container, _)) = self.remote.as_ref().and_then(|r| r.menu.as_ref())
            && self.remote_station_open(*container)
        {
            self.send(S::ContainerButtonClick(play::ContainerButtonClick {
                container: *container,
                button,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::legacy_smithing_additions;
    use horizon_protocol::buf::Writer;

    fn name(id: i32) -> Option<String> {
        match id {
            1 => Some("minecraft:netherite_ingot".into()),
            2 => Some("custom:addition".into()),
            _ => None,
        }
    }

    fn update(ids: &[i32]) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(1);
        w.string("minecraft:smithing_addition");
        w.varint(ids.len() as i32);
        for id in ids {
            w.varint(*id);
        }
        w.varint(1); // ignored, but fully decoded stonecutting entry
        w.varint(2); // HolderSet with one item
        w.varint(1);
        w.varint(4); // ItemSlotDisplay
        w.varint(2);
        w.data
    }

    #[test]
    fn legacy_smithing_recipe_reload_includes_custom_additions_and_empty_replacement() {
        let mut current = legacy_smithing_additions(&update(&[1, 2]), name).unwrap();
        assert_eq!(current.len(), 2);
        assert!(current.contains("custom:addition"));
        current = legacy_smithing_additions(&update(&[]), name).unwrap();
        assert!(current.is_empty());
    }

    #[test]
    fn legacy_smithing_recipe_reload_rejects_partial_or_invalid_packets() {
        let valid = update(&[1]);
        for end in 0..valid.len() {
            assert!(
                legacy_smithing_additions(&valid[..end], name).is_err(),
                "prefix {end}"
            );
        }
        let mut trailing = valid;
        trailing.push(0);
        assert!(legacy_smithing_additions(&trailing, name).is_err());
        assert!(legacy_smithing_additions(&update(&[99]), name).is_err());
        let mut duplicate = Writer::new();
        duplicate.varint(2);
        for _ in 0..2 {
            duplicate.string("minecraft:smithing_addition");
            duplicate.varint(0);
        }
        duplicate.varint(0);
        assert!(legacy_smithing_additions(&duplicate.data, name).is_err());
    }
}
