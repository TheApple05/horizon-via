use super::*;

pub(super) struct ClickResult {
    pub returned: Option<Stack>,
    pub resync: bool,
}
#[derive(PartialEq)]
pub(super) struct Shape {
    pub id: i16,
    pub damage: i16,
    pub tag: Tag,
}
pub(super) fn shape(stack: &Stack, cx: &impl Context) -> Result<Shape> {
    let mut one = stack.clone();
    one.count = 1;
    let mut w = Writer::new();
    cx.try_write_old_stack(Some(&one), &mut w)?;
    let mut r = Reader::new(&w.data);
    let id = r.i16()?;
    let count = r.i8()?;
    let damage = r.i16()?;
    let tag = r.named_nbt()?;
    end(&r)?;
    if id < 0 || count != 1 {
        return Err(Error::new(
            "item callback did not write a nonempty old stack",
        ));
    }
    Ok(Shape { id, damage, tag })
}
pub(super) fn props(stack: &Stack, cx: &impl Context) -> Result<item_properties::Properties> {
    item_properties::get(shape(stack, cx)?.id)
        .ok_or_else(|| Error::new("old item has no verified inventory properties"))
}
pub(super) fn same(a: &Stack, b: &Stack, cx: &impl Context, merge: bool) -> Result<bool> {
    let a = shape(a, cx)?;
    let b = shape(b, cx)?;
    let damage_matches = if !merge || item_properties::get(a.id).is_some_and(|p| p.subtypes) {
        a.damage == b.damage
    } else {
        true
    };
    Ok(a.id == b.id && a.tag == b.tag && damage_matches)
}
fn count(stack: &Option<Stack>) -> i32 {
    stack.as_ref().map_or(0, |s| s.count)
}
fn take(stack: &mut Option<Stack>, amount: i32) -> Option<Stack> {
    let value = stack.as_mut()?;
    let mut out = value.clone();
    out.count = amount.min(value.count);
    value.count -= out.count;
    if value.count <= 0 {
        *stack = None;
    }
    Some(out)
}

impl State {
    fn limit(&self, slot: usize) -> i32 {
        if self.active().kind == Kind::Inventory && (5..9).contains(&slot) {
            1
        } else {
            64
        }
    }
    fn may_place(&self, slot: usize, stack: &Stack, cx: &impl Context) -> Result<bool> {
        if self.active().kind != Kind::Inventory {
            return Ok(true);
        }
        match slot {
            0 => Ok(false),
            5..=8 => {
                let raw = shape(stack, cx)?;
                let armor = props(stack, cx)?.armor;
                Ok(armor == slot as i8 - 5 || slot == 5 && matches!(raw.id, 86 | 397))
            }
            _ => Ok(true),
        }
    }
    fn eligible(&self, slot: usize, cursor: &Stack, cx: &impl Context) -> Result<bool> {
        match self.active().slots[slot].as_ref() {
            None => Ok(true),
            Some(item) => {
                Ok(same(item, cursor, cx, false)? && item.count <= props(cursor, cx)?.limit)
            }
        }
    }
    fn pickup_all_allowed(&self, slot: usize) -> bool {
        self.active().kind != Kind::Inventory || slot != 0
    }
    fn merge(
        &mut self,
        source: usize,
        start: usize,
        end: usize,
        reverse: bool,
        cx: &impl Context,
    ) -> Result<bool> {
        let mut moving = self.active().slots[source].clone().unwrap();
        let original = moving.count;
        let order: Vec<usize> = if reverse {
            (start..end).rev().collect()
        } else {
            (start..end).collect()
        };
        let maximum = props(&moving, cx)?.limit;
        if maximum > 1 {
            for &slot in &order {
                if moving.count <= 0 {
                    break;
                }
                let Some(other) = self.active().slots[slot].as_ref() else {
                    continue;
                };
                if same(&moving, other, cx, true)? {
                    let amount = moving.count.min((maximum - other.count).max(0));
                    self.active_mut().slots[slot].as_mut().unwrap().count += amount;
                    moving.count -= amount;
                }
            }
        }
        if moving.count > 0 {
            for &slot in &order {
                if self.active().slots[slot].is_none() {
                    self.active_mut().slots[slot] = Some(moving.clone());
                    moving.count = 0;
                    break;
                }
            }
        }
        self.active_mut().slots[source] = if moving.count > 0 { Some(moving) } else { None };
        Ok(self.active().slots[source].as_ref().map_or(0, |s| s.count) != original)
    }
    fn quickmove(&mut self, slot: usize, cx: &impl Context) -> Result<Option<Stack>> {
        let Some(original) = self.active().slots[slot].clone() else {
            return Ok(None);
        };
        let kind = self.active().kind;
        let (start, end, reverse) = match kind {
            Kind::Storage { top, .. } => {
                if slot < top {
                    (top, top + 36, true)
                } else {
                    (0, top, false)
                }
            }
            Kind::Inventory => {
                if slot < 9 {
                    (9, 45, slot == 0)
                } else {
                    let armor = props(&original, cx)?.armor;
                    if armor >= 0 && self.active().slots[5 + armor as usize].is_none() {
                        (5 + armor as usize, 6 + armor as usize, false)
                    } else if slot < 36 {
                        (36, 45, false)
                    } else {
                        (9, 36, false)
                    }
                }
            }
            Kind::Special { .. } | Kind::Merchant | Kind::Horse { .. } => unreachable!(),
        };
        Ok(self
            .merge(slot, start, end, reverse, cx)?
            .then_some(original))
    }
    fn player_index(&self, inventory_index: usize) -> usize {
        if inventory_index < 9 {
            self.active().kind.top() + 27 + inventory_index
        } else {
            self.active().kind.top() + inventory_index - 9
        }
    }
    fn swap(&mut self, slot: usize, button: usize, cx: &impl Context) -> Result<()> {
        let hotbar = self.player_index(button);
        let hot = self.active().slots[hotbar].clone();
        let clicked = self.active().slots[slot].clone();
        let player_slot = self.active().kind == Kind::Inventory || slot >= self.active().kind.top();
        let may_place = if let Some(hot) = &hot {
            self.may_place(slot, hot, cx)?
        } else {
            true
        };
        let directly = hot.is_none() || player_slot && may_place;
        let empty = (0..36)
            .map(|i| self.player_index(i))
            .find(|i| self.active().slots[*i].is_none());
        if directly || empty.is_some() {
            if let Some(clicked) = clicked {
                self.active_mut().slots[hotbar] = Some(clicked);
                if (!player_slot || !may_place)
                    && let Some(mut moving) = hot.clone()
                {
                    // Vanilla InventoryPlayer.addItemStackToInventory: merge then first empty in
                    // inventory order(hotbar0..8,main9..35), not the chest's clicked slot.
                    self.active_mut().slots[slot] = None;
                    let max = props(&moving, cx)?.limit;
                    for i in 0..36 {
                        let dest = self.player_index(i);
                        if moving.count <= 0 {
                            break;
                        }
                        if let Some(existing) = self.active().slots[dest].as_ref()
                            && same(existing, &moving, cx, true)?
                        {
                            let amount = moving.count.min((max - existing.count).max(0));
                            self.active_mut().slots[dest].as_mut().unwrap().count += amount;
                            moving.count -= amount;
                        }
                    }
                    if moving.count > 0 {
                        let dest = (0..36)
                            .map(|i| self.player_index(i))
                            .find(|i| self.active().slots[*i].is_none())
                            .ok_or_else(|| {
                                Error::new("hotbar relocation lost its empty inventory slot")
                            })?;
                        self.active_mut().slots[dest] = Some(moving);
                    }
                } else {
                    self.active_mut().slots[slot] = hot;
                }
            } else if let Some(hot) = hot
                && self.may_place(slot, &hot, cx)?
            {
                self.active_mut().slots[hotbar] = None;
                self.active_mut().slots[slot] = Some(hot);
            }
        }
        Ok(())
    }

    pub(super) fn simulate(
        &mut self,
        slot: i16,
        button: i8,
        input: i32,
        cx: &impl Context,
    ) -> Result<ClickResult> {
        if !(0..=6).contains(&input) {
            return Err(Error::new("unknown protocol47 click mode"));
        }
        let index = if slot >= 0 { Some(slot as usize) } else { None };
        if index.is_some_and(|i| i >= self.active().slots.len()) {
            return Err(Error::new("click exceeds old window slot count"));
        }
        if slot < 0 && !matches!(slot, -1 | -999) {
            return Err(Error::new("invalid negative protocol47 click slot"));
        }
        let original = index.and_then(|i| self.active().slots[i].clone());
        let mut result = ClickResult {
            returned: None,
            resync: false,
        };
        if input == 5 {
            if !(0..=10).contains(&button) {
                return Err(Error::new("invalid protocol47 drag button"));
            }
            let previous = self.drag.phase;
            self.drag.phase = (button as u8) & 3;
            if (previous != 1 || self.drag.phase != 2) && previous != self.drag.phase
                || self.carried.is_none()
            {
                self.drag = Drag::default();
                return Ok(result);
            }
            match self.drag.phase {
                0 => {
                    let mode = (button as u8 >> 2) & 3;
                    if mode <= 1 || mode == 2 && self.creative {
                        self.drag = Drag {
                            phase: 1,
                            mode,
                            slots: vec![],
                        };
                    } else {
                        self.drag = Drag::default();
                    }
                }
                1 => {
                    let i = index.ok_or_else(|| Error::new("drag add needs a real slot"))?;
                    let cursor = self.carried.as_ref().unwrap();
                    if self.eligible(i, cursor, cx)?
                        && self.may_place(i, cursor, cx)?
                        && cursor.count > self.drag.slots.len() as i32
                        && !self.drag.slots.contains(&i)
                    {
                        self.drag.slots.push(i);
                    }
                }
                2 => {
                    let drag = std::mem::take(&mut self.drag);
                    let special = matches!(
                        self.active().kind,
                        Kind::Special { .. } | Kind::Merchant | Kind::Horse { .. }
                    ) || self.active().kind == Kind::Inventory
                        && drag.slots.iter().any(|s| (1..=4).contains(s))
                        || self.active().slots.iter().flatten().any(|s| s.count > 64)
                        || self.carried.as_ref().is_some_and(|s| s.count > 64);
                    if special {
                        result.resync = true;
                        return Ok(result);
                    }
                    if !drag.slots.is_empty() {
                        let cursor = self.carried.clone().unwrap();
                        let mut left = cursor.count;
                        let maximum = props(&cursor, cx)?.limit;
                        for i in drag.slots.iter().copied() {
                            if self.eligible(i, &cursor, cx)?
                                && self.may_place(i, &cursor, cx)?
                                && cursor.count >= drag.slots.len() as i32
                            {
                                let old = count(&self.active().slots[i]);
                                let base = match drag.mode {
                                    0 => cursor.count / drag.slots.len() as i32,
                                    1 => 1,
                                    2 => maximum,
                                    _ => unreachable!(),
                                };
                                let new = (base + old).min(maximum).min(self.limit(i));
                                left -= new - old;
                                let mut item = cursor.clone();
                                item.count = new;
                                self.active_mut().slots[i] = Some(item);
                            }
                        }
                        self.carried = if left > 0 {
                            let mut remaining = cursor;
                            remaining.count = left;
                            Some(remaining)
                        } else {
                            None
                        };
                    }
                }
                _ => self.drag = Drag::default(),
            }
            return Ok(result);
        }
        if self.drag.phase != 0 {
            self.drag = Drag::default();
            return Ok(result);
        }
        if input == 2 && !(0..=8).contains(&button) {
            return Err(Error::new(
                "hotbar/offhand swap has no protocol47 counterpart",
            ));
        }
        if matches!(input, 0 | 1 | 4 | 6) && !(0..=1).contains(&button) {
            return Err(Error::new("invalid protocol47 click button"));
        }
        let special = matches!(
            self.active().kind,
            Kind::Special { .. } | Kind::Merchant | Kind::Horse { .. }
        ) || self.active().slots.iter().flatten().any(|s| s.count > 64)
            || self.carried.as_ref().is_some_and(|s| s.count > 64)
            || self.active().kind == Kind::Inventory
                && (index.is_some_and(|i| i <= 4) && input != 3
                    || input == 6 && self.active().slots[1..5].iter().any(Option::is_some));
        if special {
            result.returned = if matches!(input, 0 | 1) {
                original
            } else {
                None
            };
            result.resync = true;
            return Ok(result);
        }
        match input {
            0 | 1 if slot == -999 => {
                if let Some(cursor) = self.carried.as_ref() {
                    let amount = if button == 0 { cursor.count } else { 1 };
                    take(&mut self.carried, amount);
                }
            }
            1 => {
                if let Some(i) = index {
                    result.returned = self.quickmove(i, cx)?;
                }
            }
            0 => {
                if let Some(i) = index {
                    result.returned = original.clone();
                    let cursor = self.carried.clone();
                    match (original, cursor) {
                        (None, Some(cursor)) => {
                            if self.may_place(i, &cursor, cx)? {
                                let amount =
                                    (if button == 0 { cursor.count } else { 1 }).min(self.limit(i));
                                self.active_mut().slots[i] = take(&mut self.carried, amount);
                            }
                        }
                        (Some(item), None) => {
                            let amount = if button == 0 {
                                item.count
                            } else {
                                (item.count + 1) / 2
                            };
                            let picked = take(&mut self.active_mut().slots[i], amount);
                            self.carried = picked;
                        }
                        (Some(item), Some(cursor)) => {
                            if self.may_place(i, &cursor, cx)? {
                                if same(&item, &cursor, cx, false)? {
                                    let amount = (if button == 0 { cursor.count } else { 1 })
                                        .min((self.limit(i) - item.count).max(0))
                                        .min((props(&cursor, cx)?.limit - item.count).max(0));
                                    take(&mut self.carried, amount);
                                    self.active_mut().slots[i].as_mut().unwrap().count += amount;
                                } else if cursor.count <= self.limit(i) {
                                    self.active_mut().slots[i] = Some(cursor);
                                    self.carried = Some(item);
                                }
                            } else if props(&cursor, cx)?.limit > 1
                                && same(&item, &cursor, cx, true)?
                                && item.count + cursor.count <= props(&cursor, cx)?.limit
                            {
                                self.active_mut().slots[i] = None;
                                self.carried.as_mut().unwrap().count += item.count;
                            }
                        }
                        (None, None) => {}
                    }
                }
            }
            2 => {
                if let Some(i) = index {
                    self.swap(i, button as usize, cx)?;
                }
            }
            3 => {
                if self.creative
                    && self.carried.is_none()
                    && let Some(mut item) = original
                {
                    item.count = props(&item, cx)?.limit;
                    self.carried = Some(item);
                }
            }
            4 => {
                if self.carried.is_none()
                    && let Some(i) = index
                {
                    let amount = if button == 0 {
                        1
                    } else {
                        count(&self.active().slots[i])
                    };
                    take(&mut self.active_mut().slots[i], amount);
                }
            }
            6 => {
                if let (Some(i), Some(mut cursor)) = (index, self.carried.clone())
                    && self.active().slots[i].is_none()
                {
                    let order: Vec<_> = if button == 0 {
                        (0..self.active().slots.len()).collect()
                    } else {
                        (0..self.active().slots.len()).rev().collect()
                    };
                    let max = props(&cursor, cx)?.limit;
                    for pass in 0..2 {
                        for &slot in &order {
                            if cursor.count >= max {
                                break;
                            }
                            if self.pickup_all_allowed(slot)
                                && let Some(other) = self.active().slots[slot].as_ref()
                                && same(other, &cursor, cx, true)?
                                && (pass != 0 || other.count != props(other, cx)?.limit)
                            {
                                let amount = (max - cursor.count).min(other.count);
                                take(&mut self.active_mut().slots[slot], amount);
                                cursor.count += amount;
                            }
                        }
                    }
                    self.carried = Some(cursor);
                }
            }
            _ => {}
        }
        Ok(result)
    }
}
