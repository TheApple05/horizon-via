//! Vanilla47 uses MC|TrList/MC|TrSel. Selection only changes the recipe index; the
//! modern remote screen expects the server to move payments. We send verified
//! old pickup clicks, then request vanilla's complete authoritative result.
use super::*;
use crate::item::{Component, component};

#[derive(Clone, Debug)]
struct Offer {
    a: OldSlot,
    b: Option<OldSlot>,
    result: Stack,
}
#[derive(Clone, Debug, Default)]
pub(super) struct Merchant {
    offers: Vec<Offer>,
}

fn old_item(r: &mut Reader<'_>, cx: &impl Context) -> Result<(OldSlot, Stack)> {
    let start = r.position();
    let item = cx
        .read_old_slot(r)?
        .ok_or_else(|| Error::new("empty old merchant offer item"))?;
    let mut raw = Reader::new(r.since(start));
    let old =
        OldSlot::read(&mut raw)?.ok_or_else(|| Error::new("empty old merchant offer item"))?;
    end(&raw)?;
    if item.count != i32::from(old.count) {
        return Err(Error::new("merchant item callback changed count"));
    }
    Ok((old, item))
}
fn cost(raw: &OldSlot, item: Stack) -> Result<play::NetCost> {
    // Old costs match a subset of raw NBT, which is not equivalent to exact
    // modern component predicates. Vanilla costs have no NBT; refuse custom
    // tagged costs instead of inventing a lossy or falsely restrictive match.
    if raw.tag != Tag::End {
        return Err(Error::new(
            "tagged old merchant costs require an NBT subset predicate adapter",
        ));
    }
    // Tag-free old costs accept arbitrary candidate NBT. Include only the old
    // numeric damage constraint; subtype material has already become item id.
    let mut components = vec![];
    if damageable(raw.id) {
        components.push((component::DAMAGE, Component::Int(i32::from(raw.damage))));
    }
    Ok(play::NetCost {
        item: item.item,
        count: item.count,
        components,
    })
}
pub(super) fn damageable(id: i16) -> bool {
    // Actual337-item1.8.9 registry (Item.getMaxDamage>0), not a modern id range.
    matches!(id,256..=259|261|267..=279|283..=286|290..=294|298..=317|346|359|398)
}
impl State {
    pub(super) fn merchant_payload(
        &mut self,
        payload: &[u8],
        cx: &impl Context,
    ) -> Result<Option<Vec<play::Clientbound>>> {
        let mut r = Reader::new(payload);
        if r.string(20)? != "MC|TrList" {
            return Ok(None);
        }
        let id = r.i32()?;
        let n = usize::from(r.u8()?);
        let mut raw_offers = Vec::with_capacity(n);
        let mut offers = Vec::with_capacity(n);
        for _ in 0..n {
            let (a, stack_a) = old_item(&mut r, cx)?;
            let (_, result) = old_item(&mut r, cx)?;
            let (b, cost_b) = if boolean(&mut r)? {
                let (b, stack_b) = old_item(&mut r, cx)?;
                let cost = cost(&b, stack_b)?;
                (Some(b), Some(cost))
            } else {
                (None, None)
            };
            let out_of_stock = boolean(&mut r)?;
            let uses = r.i32()?;
            let max_uses = r.i32()?;
            if uses < 0 || max_uses < 0 {
                return Err(Error::new("negative old merchant usage count"));
            }
            offers.push(play::NetOffer {
                cost_a: cost(&a, stack_a)?,
                result: result.clone(),
                cost_b,
                out_of_stock,
                uses,
                max_uses,
                xp: 0,
                special_price: 0,
                price_multiplier: 0.,
                demand: 0,
            });
            raw_offers.push(Offer { a, b, result });
        }
        end(&r)?;
        if self.active().kind != Kind::Merchant || id != i32::from(self.active().id) {
            return Ok(Some(vec![]));
        }
        self.merchant = Some(Merchant { offers: raw_offers });
        Ok(Some(vec![play::Clientbound::MerchantOffers(
            play::MerchantOffers {
                container: id,
                offers,
                level: 0,
                xp: 0,
                show_progress: false,
                can_restock: false,
            },
        )]))
    }
    pub(super) fn select_trade(&mut self, index: i32, cx: &impl Context) -> Result<Vec<OldPacket>> {
        if self.active().kind != Kind::Merchant {
            return Err(Error::new("trade selection outside old merchant menu"));
        }
        let offer = usize::try_from(index)
            .ok()
            .and_then(|i| self.merchant.as_ref()?.offers.get(i))
            .cloned()
            .ok_or_else(|| Error::new("trade selection has no synchronized old offer"))?;
        if self.resync.is_some() {
            self.deferred_trade = Some(index);
            return Ok(vec![]);
        }
        if self.active().known.iter().any(|v| !*v) {
            return Err(Error::new("merchant slots are not synchronized"));
        }
        self.drag = Drag::default();
        let mut w = Writer::new();
        w.string("MC|TrSel");
        w.i32(index);
        let mut out = vec![packet(0x17, w)];
        if self.carried.is_none() {
            // Only the two inputs/player slots use ordinary pickup rules. Never
            // touch result2 or guess recipe matching/consumption/enchanted NBT.
            let kind = self.active().kind;
            self.active_mut().kind = Kind::Storage { top: 3, menu: 19 };
            let plan = self.fill_trade(&offer, cx, &mut out);
            self.active_mut().kind = kind;
            plan?;
        }
        // Mode3 on slot-1 is an exact no-op even in creative. Vanilla always
        // returns null, so a complete nonempty cached offer result guarantees
        // a false transaction after all payments have executed. Its full
        // slots/cursor snapshot supplies the authoritative recipe result.
        self.script_click(-1, 3, Some(&offer.result), cx, &mut out)?;
        self.resync = Some(Resync {
            window: self.active().id,
            content: false,
            cursor: false,
        });
        Ok(out)
    }
    fn script_click(
        &mut self,
        slot: i16,
        mode: u8,
        checksum: Option<&Stack>,
        cx: &impl Context,
        out: &mut Vec<OldPacket>,
    ) -> Result<()> {
        if self.pending.len() >= 128 {
            return Err(Error::new("merchant fill exceeds old transaction budget"));
        }
        let win = self.active_mut();
        win.action = win.action.wrapping_add(1);
        let (id, action) = (win.id, win.action);
        let mut w = Writer::new();
        w.u8(id);
        w.i16(slot);
        w.i8(0);
        w.i16(action);
        w.u8(mode);
        cx.try_write_old_stack(checksum, &mut w)?;
        self.pending.push_back(Pending { window: id, action });
        out.push(packet(0x0e, w));
        Ok(())
    }
    fn pickup(&mut self, slot: usize, cx: &impl Context, out: &mut Vec<OldPacket>) -> Result<()> {
        let result = self.simulate(slot as i16, 0, 0, cx)?;
        if result.resync {
            return Err(Error::new(
                "merchant input requires unsupported oversized stack",
            ));
        }
        self.script_click(slot as i16, 0, result.returned.as_ref(), cx, out)
    }
    fn fill_trade(
        &mut self,
        offer: &Offer,
        cx: &impl Context,
        out: &mut Vec<OldPacket>,
    ) -> Result<()> {
        for input in 0..2 {
            if self.active().slots[input].is_none() {
                continue;
            }
            self.pickup(input, cx, out)?;
            // Merge existing stacks first, then use empty player inventory.
            for empty in [false, true] {
                for slot in 3..39 {
                    let Some(cursor) = self.carried.as_ref() else {
                        break;
                    };
                    let target = self.active().slots[slot].as_ref();
                    let eligible = if empty {
                        target.is_none()
                    } else {
                        match target {
                            Some(target) => {
                                click::same(cursor, target, cx, true)?
                                    && target.count < click::props(cursor, cx)?.limit
                            }
                            _ => false,
                        }
                    };
                    if eligible {
                        self.pickup(slot, cx, out)?;
                    }
                }
            }
            if self.carried.is_some() {
                self.pickup(input, cx, out)?;
                return Ok(()); // Full inventory: leave remaining payments safely in inputs.
            }
        }
        for (input, cost) in [(0, Some(&offer.a)), (1, offer.b.as_ref())] {
            let Some(cost) = cost else {
                continue;
            };
            for source in 3..39 {
                let Some(stack) = self.active().slots[source].as_ref() else {
                    continue;
                };
                let raw = click::shape(stack, cx)?;
                if raw.id != cost.id || raw.damage != cost.damage {
                    continue;
                }
                if let Some(target) = self.active().slots[input].as_ref() {
                    if !click::same(stack, target, cx, false)? {
                        continue;
                    }
                    if target.count >= click::props(stack, cx)?.limit {
                        break;
                    }
                }
                self.pickup(source, cx, out)?;
                self.pickup(input, cx, out)?;
                if self.carried.is_some() {
                    self.pickup(source, cx, out)?;
                }
            }
        }
        Ok(())
    }
}
