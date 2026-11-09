use super::*;
use crate::item::{Component, Hashed, component};

pub(super) struct Cx;
impl Context for Cx {
    fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
        Ok(Tag::String(r.string(32767)?.as_str().into()))
    }
    fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>> {
        let id = r.i16()?;
        if id == -1 {
            return Ok(None);
        }
        let count = r.i8()?;
        let damage = r.i16()?;
        let tag = r.named_nbt()?;
        let mut item = Stack::new(i32::from(id) + 10000, i32::from(count));
        if damage != 0 {
            item.added
                .push((component::DAMAGE, Component::Int(i32::from(damage))));
        }
        if tag != Tag::End {
            item.added
                .push((component::CUSTOM_DATA, Component::Nbt(tag)));
        }
        Ok(Some(item))
    }
    fn try_write_old_stack(&self, item: Option<&Stack>, w: &mut Writer) -> Result<()> {
        let Some(item) = item else {
            w.i16(-1);
            return Ok(());
        };
        let id = i16::try_from(item.item - 10000).map_err(|_| Error::new("unknown test item"))?;
        if id < 0 || !(1..=127).contains(&item.count) {
            return Err(Error::new("invalid test item"));
        }
        w.i16(id);
        w.i8(item.count as i8);
        w.i16(item.damage() as i16);
        if let Some(Component::Nbt(tag)) = item.component(component::CUSTOM_DATA) {
            w.named_nbt(tag);
        } else {
            w.u8(0);
        }
        Ok(())
    }
}
pub(super) fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn fixture() -> serde_json::Value {
    // Normal vanilla packet writers and actual Container.slotClick outputs. Sparse storage here
    // removes only known empty slots; it does not derive expected click outcomes from this bridge.
    let mut value:serde_json::Value=serde_json::from_str(r##"{"packets":{"open_chest":"010f6d696e6563726166743a63686573740f2246697874757265206368657374221b","property":"01000204d2","reject":"01800000","ack":"01800001","cursor":"ffffff00010d000000","open_brewing":"02176d696e6563726166743a62726577696e675f7374616e641122466978747572652062726577696e672204","close_inventory":"00","creative_hotbar":"0024000140000000"},"cases":[{"kind":"chest","creative":false,"drops":[],"name":"pickup_whole","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"00010d000000","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"0100000000010000010d000000"}]},{"kind":"chest","creative":false,"drops":[],"name":"pickup_half","steps":[{"slot":0,"button":1,"mode":0,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"00010d000000","after":{"count":63,"nonempty":[[0,"000106000000"]]},"carried_after":"000107000000","click_wire":"0100000100010000010d000000"}]},{"kind":"chest","creative":false,"drops":[],"name":"deposit_then_one","steps":[{"slot":0,"button":1,"mode":0,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000101000000"]]},"carried_after":"00010c000000","click_wire":"01000001000100ffff"},{"slot":1,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"000101000000"]]},"carried_before":"00010c000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000101000000"],[1,"00010c000000"]]},"carried_after":"ffff","click_wire":"01000100000200ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"merge_to_limit","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010a000000","returned":"00013c000000","after":{"count":63,"nonempty":[[0,"000140000000"]]},"carried_after":"000106000000","click_wire":"0100000000010000013c000000"},{"slot":1,"button":1,"mode":0,"before":{"count":63,"nonempty":[[0,"000140000000"]]},"carried_before":"000106000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000140000000"],[1,"000101000000"]]},"carried_after":"000105000000","click_wire":"01000101000200ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"pickup_swap","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"000307000000"]]},"carried_before":"00010a000000","returned":"000307000000","after":{"count":63,"nonempty":[[0,"00010a000000"]]},"carried_after":"000307000000","click_wire":"01000000000100000307000000"}]},{"kind":"chest","creative":false,"drops":["000101000000","000109000000"],"name":"outside_drop","steps":[{"slot":-999,"button":1,"mode":0,"before":{"count":63,"nonempty":[]},"carried_before":"00010a000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"000109000000","click_wire":"01fc1901000100ffff"},{"slot":-999,"button":0,"mode":0,"before":{"count":63,"nonempty":[]},"carried_before":"000109000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"ffff","click_wire":"01fc1900000200ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"quick_move_chest_out","steps":[{"slot":0,"button":0,"mode":1,"before":{"count":63,"nonempty":[[0,"00010d000000"],[62,"00013c000000"]]},"carried_before":"ffff","returned":"00010d000000","after":{"count":63,"nonempty":[[61,"000109000000"],[62,"000140000000"]]},"carried_after":"ffff","click_wire":"0100000000010100010d000000"}]},{"kind":"chest","creative":false,"drops":[],"name":"quick_move_chest_in","steps":[{"slot":27,"button":0,"mode":1,"before":{"count":63,"nonempty":[[0,"00013c000000"],[27,"00010d000000"]]},"carried_before":"ffff","returned":"00010d000000","after":{"count":63,"nonempty":[[0,"000140000000"],[1,"000109000000"]]},"carried_after":"ffff","click_wire":"01001b0000010100010d000000"}]},{"kind":"chest","creative":false,"drops":[],"name":"quick_move_full","steps":[{"slot":0,"button":0,"mode":1,"before":{"count":63,"nonempty":[[0,"00010d000000"],[27,"000340000000"],[28,"000340000000"],[29,"000340000000"],[30,"000340000000"],[31,"000340000000"],[32,"000340000000"],[33,"000340000000"],[34,"000340000000"],[35,"000340000000"],[36,"000340000000"],[37,"000340000000"],[38,"000340000000"],[39,"000340000000"],[40,"000340000000"],[41,"000340000000"],[42,"000340000000"],[43,"000340000000"],[44,"000340000000"],[45,"000340000000"],[46,"000340000000"],[47,"000340000000"],[48,"000340000000"],[49,"000340000000"],[50,"000340000000"],[51,"000340000000"],[52,"000340000000"],[53,"000340000000"],[54,"000340000000"],[55,"000340000000"],[56,"000340000000"],[57,"000340000000"],[58,"000340000000"],[59,"000340000000"],[60,"000340000000"],[61,"000340000000"],[62,"000340000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010d000000"],[27,"000340000000"],[28,"000340000000"],[29,"000340000000"],[30,"000340000000"],[31,"000340000000"],[32,"000340000000"],[33,"000340000000"],[34,"000340000000"],[35,"000340000000"],[36,"000340000000"],[37,"000340000000"],[38,"000340000000"],[39,"000340000000"],[40,"000340000000"],[41,"000340000000"],[42,"000340000000"],[43,"000340000000"],[44,"000340000000"],[45,"000340000000"],[46,"000340000000"],[47,"000340000000"],[48,"000340000000"],[49,"000340000000"],[50,"000340000000"],[51,"000340000000"],[52,"000340000000"],[53,"000340000000"],[54,"000340000000"],[55,"000340000000"],[56,"000340000000"],[57,"000340000000"],[58,"000340000000"],[59,"000340000000"],[60,"000340000000"],[61,"000340000000"],[62,"000340000000"]]},"carried_after":"ffff","click_wire":"01000000000101ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"swap_chest_hotbar_relocates","steps":[{"slot":0,"button":0,"mode":2,"before":{"count":63,"nonempty":[[0,"00010d000000"],[54,"000305000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[54,"00010d000000"],[55,"000305000000"]]},"carried_after":"ffff","click_wire":"01000000000102ffff"}]},{"kind":"inventory","creative":false,"drops":[],"name":"swap_inventory_hotbar","steps":[{"slot":9,"button":0,"mode":2,"before":{"count":45,"nonempty":[[9,"00010d000000"],[36,"000305000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":45,"nonempty":[[9,"000305000000"],[36,"00010d000000"]]},"carried_after":"ffff","click_wire":"00000900000102ffff"}]},{"kind":"chest","creative":true,"drops":[],"name":"clone_creative","steps":[{"slot":0,"button":2,"mode":3,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_after":"000140000000","click_wire":"01000002000103ffff"}]},{"kind":"chest","creative":false,"drops":["000101000000","00010c000000"],"name":"throw_one_all","steps":[{"slot":0,"button":0,"mode":4,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010c000000"]]},"carried_after":"ffff","click_wire":"01000000000104ffff"},{"slot":0,"button":1,"mode":4,"before":{"count":63,"nonempty":[[0,"00010c000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"ffff","click_wire":"01000001000204ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"pickup_all_forward","steps":[{"slot":3,"button":0,"mode":6,"before":{"count":63,"nonempty":[[0,"000105000000"],[1,"000140000000"],[2,"000103000000"]]},"carried_before":"000101000000","returned":"ffff","after":{"count":63,"nonempty":[[1,"000109000000"]]},"carried_after":"000140000000","click_wire":"01000300000106ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"pickup_all_backward","steps":[{"slot":3,"button":1,"mode":6,"before":{"count":63,"nonempty":[[0,"000105000000"],[1,"000140000000"],[2,"000103000000"]]},"carried_before":"000101000000","returned":"ffff","after":{"count":63,"nonempty":[[1,"000109000000"]]},"carried_after":"000140000000","click_wire":"01000301000106ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"drag_even","steps":[{"slot":-999,"button":0,"mode":5,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_after":"00010d000000","click_wire":"01fc1900000105ffff"},{"slot":0,"button":1,"mode":5,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_after":"00010d000000","click_wire":"01000001000205ffff"},{"slot":1,"button":1,"mode":5,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_after":"00010d000000","click_wire":"01000101000305ffff"},{"slot":2,"button":1,"mode":5,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_after":"00010d000000","click_wire":"01000201000405ffff"},{"slot":-999,"button":2,"mode":5,"before":{"count":63,"nonempty":[[0,"00013c000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000140000000"],[1,"000104000000"],[2,"000104000000"]]},"carried_after":"000101000000","click_wire":"01fc1902000505ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"drag_one_each","steps":[{"slot":-999,"button":4,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01fc1904000105ffff"},{"slot":0,"button":5,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01000005000205ffff"},{"slot":1,"button":5,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01000105000305ffff"},{"slot":-999,"button":6,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000101000000"],[1,"000101000000"]]},"carried_after":"00010b000000","click_wire":"01fc1906000405ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"drag_interrupted","steps":[{"slot":-999,"button":0,"mode":5,"before":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_after":"00010d000000","click_wire":"01fc1900000105ffff"},{"slot":0,"button":1,"mode":5,"before":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_after":"00010d000000","click_wire":"01000001000205ffff"},{"slot":3,"button":0,"mode":0,"before":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_after":"00010d000000","click_wire":"01000300000300ffff"},{"slot":3,"button":0,"mode":0,"before":{"count":63,"nonempty":[[3,"000302000000"]]},"carried_before":"00010d000000","returned":"000302000000","after":{"count":63,"nonempty":[[3,"00010d000000"]]},"carried_after":"000302000000","click_wire":"01000300000400000302000000"}]},{"kind":"inventory","creative":false,"drops":[],"name":"inventory_quickmove_hotbar","steps":[{"slot":9,"button":0,"mode":1,"before":{"count":45,"nonempty":[[9,"00010d000000"],[44,"00013c000000"]]},"carried_before":"ffff","returned":"00010d000000","after":{"count":45,"nonempty":[[36,"000109000000"],[44,"000140000000"]]},"carried_after":"ffff","click_wire":"0000090000010100010d000000"}]},{"kind":"inventory","creative":false,"drops":[],"name":"inventory_equip_helmet","steps":[{"slot":9,"button":0,"mode":1,"before":{"count":45,"nonempty":[[9,"012a01000000"]]},"carried_before":"ffff","returned":"012a01000000","after":{"count":45,"nonempty":[[5,"012a01000000"]]},"carried_after":"ffff","click_wire":"00000900000101012a01000000"},{"slot":5,"button":0,"mode":1,"before":{"count":45,"nonempty":[[5,"012a01000000"]]},"carried_before":"ffff","returned":"012a01000000","after":{"count":45,"nonempty":[[9,"012a01000000"]]},"carried_after":"ffff","click_wire":"00000500000201012a01000000"}]},{"kind":"chest","creative":false,"drops":[],"name":"subtype_pickup_swap","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"002308000e00"]]},"carried_before":"002305000f00","returned":"002308000e00","after":{"count":63,"nonempty":[[0,"002305000f00"]]},"carried_after":"002308000e00","click_wire":"01000000000100002308000e00"}]},{"kind":"chest","creative":false,"drops":[],"name":"sixteen_stack_merge","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"01700e000000"]]},"carried_before":"017005000000","returned":"01700e000000","after":{"count":63,"nonempty":[[0,"017010000000"]]},"carried_after":"017003000000","click_wire":"0100000000010001700e000000"},{"slot":1,"button":0,"mode":0,"before":{"count":63,"nonempty":[[0,"017010000000"]]},"carried_before":"017003000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"017010000000"],[1,"017003000000"]]},"carried_after":"ffff","click_wire":"01000100000200ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"swap_empty_hotbar","steps":[{"slot":0,"button":0,"mode":2,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[54,"00010d000000"]]},"carried_after":"ffff","click_wire":"01000000000102ffff"}]},{"kind":"inventory","creative":false,"drops":[],"name":"swap_self","steps":[{"slot":36,"button":0,"mode":2,"before":{"count":45,"nonempty":[[36,"00010d000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":45,"nonempty":[[36,"00010d000000"]]},"carried_after":"ffff","click_wire":"00002400000102ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"clone_survival_noop","steps":[{"slot":0,"button":2,"mode":3,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"ffff","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_after":"ffff","click_wire":"01000002000103ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"throw_with_carried_noop","steps":[{"slot":0,"button":0,"mode":4,"before":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_before":"000302000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010d000000"]]},"carried_after":"000302000000","click_wire":"01000000000104ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"pickup_all_selected_nonempty_noop","steps":[{"slot":0,"button":0,"mode":6,"before":{"count":63,"nonempty":[[0,"00010d000000"],[1,"000104000000"]]},"carried_before":"000102000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"00010d000000"],[1,"000104000000"]]},"carried_after":"000102000000","click_wire":"01000000000106ffff"}]},{"kind":"chest","creative":true,"drops":[],"name":"drag_creative_fill","steps":[{"slot":-999,"button":8,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"000140000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"000140000000","click_wire":"01fc1908000105ffff"},{"slot":0,"button":9,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"000140000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"000140000000","click_wire":"01000009000205ffff"},{"slot":1,"button":9,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"000140000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"000140000000","click_wire":"01000109000305ffff"},{"slot":-999,"button":10,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"000140000000","returned":"ffff","after":{"count":63,"nonempty":[[0,"000140000000"],[1,"000140000000"]]},"carried_after":"ffff","click_wire":"01fc190a000405ffff"}]},{"kind":"chest","creative":false,"drops":[],"name":"drag_survival_creative_denied","steps":[{"slot":-999,"button":8,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01fc1908000105ffff"},{"slot":0,"button":9,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01000009000205ffff"},{"slot":-999,"button":10,"mode":5,"before":{"count":63,"nonempty":[]},"carried_before":"00010d000000","returned":"ffff","after":{"count":63,"nonempty":[]},"carried_after":"00010d000000","click_wire":"01fc190a000305ffff"}]},{"kind":"inventory","creative":false,"drops":[],"name":"inventory_armor_reject_wrong_item","steps":[{"slot":5,"button":0,"mode":0,"before":{"count":45,"nonempty":[[5,"012a01000000"]]},"carried_before":"000103000000","returned":"012a01000000","after":{"count":45,"nonempty":[[5,"012a01000000"]]},"carried_after":"000103000000","click_wire":"00000500000100012a01000000"}]},{"kind":"inventory","creative":false,"drops":[],"name":"inventory_craft_table","steps":[{"slot":0,"button":0,"mode":0,"before":{"count":45,"nonempty":[[0,"003a01000000"],[1,"000501000000"],[2,"000501000000"],[3,"000501000000"],[4,"000501000000"]]},"carried_before":"ffff","returned":"003a01000000","after":{"count":45,"nonempty":[]},"carried_after":"003a01000000","click_wire":"00000000000100003a01000000"}]}],"properties":{"1":{"limit":64,"armor":-1},"100":{"limit":64,"armor":-1},"101":{"limit":64,"armor":-1},"102":{"limit":64,"armor":-1},"103":{"limit":64,"armor":-1},"106":{"limit":64,"armor":-1},"107":{"limit":64,"armor":-1},"108":{"limit":64,"armor":-1},"109":{"limit":64,"armor":-1},"110":{"limit":64,"armor":-1},"111":{"limit":64,"armor":-1},"112":{"limit":64,"armor":-1},"113":{"limit":64,"armor":-1},"114":{"limit":64,"armor":-1},"116":{"limit":64,"armor":-1},"12":{"limit":64,"armor":-1},"120":{"limit":64,"armor":-1},"121":{"limit":64,"armor":-1},"122":{"limit":64,"armor":-1},"123":{"limit":64,"armor":-1},"126":{"limit":64,"armor":-1},"128":{"limit":64,"armor":-1},"129":{"limit":64,"armor":-1},"13":{"limit":64,"armor":-1},"130":{"limit":64,"armor":-1},"131":{"limit":64,"armor":-1},"133":{"limit":64,"armor":-1},"134":{"limit":64,"armor":-1},"135":{"limit":64,"armor":-1},"136":{"limit":64,"armor":-1},"137":{"limit":64,"armor":-1},"138":{"limit":64,"armor":-1},"139":{"limit":64,"armor":-1},"14":{"limit":64,"armor":-1},"143":{"limit":64,"armor":-1},"145":{"limit":64,"armor":-1},"146":{"limit":64,"armor":-1},"147":{"limit":64,"armor":-1},"148":{"limit":64,"armor":-1},"15":{"limit":64,"armor":-1},"151":{"limit":64,"armor":-1},"152":{"limit":64,"armor":-1},"153":{"limit":64,"armor":-1},"154":{"limit":64,"armor":-1},"155":{"limit":64,"armor":-1},"156":{"limit":64,"armor":-1},"157":{"limit":64,"armor":-1},"158":{"limit":64,"armor":-1},"159":{"limit":64,"armor":-1},"16":{"limit":64,"armor":-1},"160":{"limit":64,"armor":-1},"161":{"limit":64,"armor":-1},"162":{"limit":64,"armor":-1},"163":{"limit":64,"armor":-1},"164":{"limit":64,"armor":-1},"165":{"limit":64,"armor":-1},"166":{"limit":64,"armor":-1},"167":{"limit":64,"armor":-1},"168":{"limit":64,"armor":-1},"169":{"limit":64,"armor":-1},"17":{"limit":64,"armor":-1},"170":{"limit":64,"armor":-1},"171":{"limit":64,"armor":-1},"172":{"limit":64,"armor":-1},"173":{"limit":64,"armor":-1},"174":{"limit":64,"armor":-1},"175":{"limit":64,"armor":-1},"179":{"limit":64,"armor":-1},"18":{"limit":64,"armor":-1},"180":{"limit":64,"armor":-1},"182":{"limit":64,"armor":-1},"183":{"limit":64,"armor":-1},"184":{"limit":64,"armor":-1},"185":{"limit":64,"armor":-1},"186":{"limit":64,"armor":-1},"187":{"limit":64,"armor":-1},"188":{"limit":64,"armor":-1},"189":{"limit":64,"armor":-1},"19":{"limit":64,"armor":-1},"190":{"limit":64,"armor":-1},"191":{"limit":64,"armor":-1},"192":{"limit":64,"armor":-1},"2":{"limit":64,"armor":-1},"20":{"limit":64,"armor":-1},"21":{"limit":64,"armor":-1},"22":{"limit":64,"armor":-1},"2256":{"limit":1,"armor":-1},"2257":{"limit":1,"armor":-1},"2258":{"limit":1,"armor":-1},"2259":{"limit":1,"armor":-1},"2260":{"limit":1,"armor":-1},"2261":{"limit":1,"armor":-1},"2262":{"limit":1,"armor":-1},"2263":{"limit":1,"armor":-1},"2264":{"limit":1,"armor":-1},"2265":{"limit":1,"armor":-1},"2266":{"limit":1,"armor":-1},"2267":{"limit":1,"armor":-1},"23":{"limit":64,"armor":-1},"24":{"limit":64,"armor":-1},"25":{"limit":64,"armor":-1},"256":{"limit":1,"armor":-1},"257":{"limit":1,"armor":-1},"258":{"limit":1,"armor":-1},"259":{"limit":1,"armor":-1},"260":{"limit":64,"armor":-1},"261":{"limit":1,"armor":-1},"262":{"limit":64,"armor":-1},"263":{"limit":64,"armor":-1},"264":{"limit":64,"armor":-1},"265":{"limit":64,"armor":-1},"266":{"limit":64,"armor":-1},"267":{"limit":1,"armor":-1},"268":{"limit":1,"armor":-1},"269":{"limit":1,"armor":-1},"27":{"limit":64,"armor":-1},"270":{"limit":1,"armor":-1},"271":{"limit":1,"armor":-1},"272":{"limit":1,"armor":-1},"273":{"limit":1,"armor":-1},"274":{"limit":1,"armor":-1},"275":{"limit":1,"armor":-1},"276":{"limit":1,"armor":-1},"277":{"limit":1,"armor":-1},"278":{"limit":1,"armor":-1},"279":{"limit":1,"armor":-1},"28":{"limit":64,"armor":-1},"280":{"limit":64,"armor":-1},"281":{"limit":64,"armor":-1},"282":{"limit":1,"armor":-1},"283":{"limit":1,"armor":-1},"284":{"limit":1,"armor":-1},"285":{"limit":1,"armor":-1},"286":{"limit":1,"armor":-1},"287":{"limit":64,"armor":-1},"288":{"limit":64,"armor":-1},"289":{"limit":64,"armor":-1},"29":{"limit":64,"armor":-1},"290":{"limit":1,"armor":-1},"291":{"limit":1,"armor":-1},"292":{"limit":1,"armor":-1},"293":{"limit":1,"armor":-1},"294":{"limit":1,"armor":-1},"295":{"limit":64,"armor":-1},"296":{"limit":64,"armor":-1},"297":{"limit":64,"armor":-1},"298":{"limit":1,"armor":0},"299":{"limit":1,"armor":1},"3":{"limit":64,"armor":-1},"30":{"limit":64,"armor":-1},"300":{"limit":1,"armor":2},"301":{"limit":1,"armor":3},"302":{"limit":1,"armor":0},"303":{"limit":1,"armor":1},"304":{"limit":1,"armor":2},"305":{"limit":1,"armor":3},"306":{"limit":1,"armor":0},"307":{"limit":1,"armor":1},"308":{"limit":1,"armor":2},"309":{"limit":1,"armor":3},"31":{"limit":64,"armor":-1},"310":{"limit":1,"armor":0},"311":{"limit":1,"armor":1},"312":{"limit":1,"armor":2},"313":{"limit":1,"armor":3},"314":{"limit":1,"armor":0},"315":{"limit":1,"armor":1},"316":{"limit":1,"armor":2},"317":{"limit":1,"armor":3},"318":{"limit":64,"armor":-1},"319":{"limit":64,"armor":-1},"32":{"limit":64,"armor":-1},"320":{"limit":64,"armor":-1},"321":{"limit":64,"armor":-1},"322":{"limit":64,"armor":-1},"323":{"limit":16,"armor":-1},"324":{"limit":64,"armor":-1},"325":{"limit":16,"armor":-1},"326":{"limit":1,"armor":-1},"327":{"limit":1,"armor":-1},"328":{"limit":1,"armor":-1},"329":{"limit":1,"armor":-1},"33":{"limit":64,"armor":-1},"330":{"limit":64,"armor":-1},"331":{"limit":64,"armor":-1},"332":{"limit":16,"armor":-1},"333":{"limit":1,"armor":-1},"334":{"limit":64,"armor":-1},"335":{"limit":1,"armor":-1},"336":{"limit":64,"armor":-1},"337":{"limit":64,"armor":-1},"338":{"limit":64,"armor":-1},"339":{"limit":64,"armor":-1},"340":{"limit":64,"armor":-1},"341":{"limit":64,"armor":-1},"342":{"limit":1,"armor":-1},"343":{"limit":1,"armor":-1},"344":{"limit":16,"armor":-1},"345":{"limit":64,"armor":-1},"346":{"limit":1,"armor":-1},"347":{"limit":64,"armor":-1},"348":{"limit":64,"armor":-1},"349":{"limit":64,"armor":-1},"35":{"limit":64,"armor":-1},"350":{"limit":64,"armor":-1},"351":{"limit":64,"armor":-1},"352":{"limit":64,"armor":-1},"353":{"limit":64,"armor":-1},"354":{"limit":1,"armor":-1},"355":{"limit":1,"armor":-1},"356":{"limit":64,"armor":-1},"357":{"limit":64,"armor":-1},"358":{"limit":64,"armor":-1},"359":{"limit":1,"armor":-1},"360":{"limit":64,"armor":-1},"361":{"limit":64,"armor":-1},"362":{"limit":64,"armor":-1},"363":{"limit":64,"armor":-1},"364":{"limit":64,"armor":-1},"365":{"limit":64,"armor":-1},"366":{"limit":64,"armor":-1},"367":{"limit":64,"armor":-1},"368":{"limit":16,"armor":-1},"369":{"limit":64,"armor":-1},"37":{"limit":64,"armor":-1},"370":{"limit":64,"armor":-1},"371":{"limit":64,"armor":-1},"372":{"limit":64,"armor":-1},"373":{"limit":1,"armor":-1},"374":{"limit":64,"armor":-1},"375":{"limit":64,"armor":-1},"376":{"limit":64,"armor":-1},"377":{"limit":64,"armor":-1},"378":{"limit":64,"armor":-1},"379":{"limit":64,"armor":-1},"38":{"limit":64,"armor":-1},"380":{"limit":64,"armor":-1},"381":{"limit":64,"armor":-1},"382":{"limit":64,"armor":-1},"383":{"limit":64,"armor":-1},"384":{"limit":64,"armor":-1},"385":{"limit":64,"armor":-1},"386":{"limit":1,"armor":-1},"387":{"limit":16,"armor":-1},"388":{"limit":64,"armor":-1},"389":{"limit":64,"armor":-1},"39":{"limit":64,"armor":-1},"390":{"limit":64,"armor":-1},"391":{"limit":64,"armor":-1},"392":{"limit":64,"armor":-1},"393":{"limit":64,"armor":-1},"394":{"limit":64,"armor":-1},"395":{"limit":64,"armor":-1},"396":{"limit":64,"armor":-1},"397":{"limit":64,"armor":-1},"398":{"limit":1,"armor":-1},"399":{"limit":64,"armor":-1},"4":{"limit":64,"armor":-1},"40":{"limit":64,"armor":-1},"400":{"limit":64,"armor":-1},"401":{"limit":64,"armor":-1},"402":{"limit":64,"armor":-1},"403":{"limit":1,"armor":-1},"404":{"limit":64,"armor":-1},"405":{"limit":64,"armor":-1},"406":{"limit":64,"armor":-1},"407":{"limit":1,"armor":-1},"408":{"limit":1,"armor":-1},"409":{"limit":64,"armor":-1},"41":{"limit":64,"armor":-1},"410":{"limit":64,"armor":-1},"411":{"limit":64,"armor":-1},"412":{"limit":64,"armor":-1},"413":{"limit":1,"armor":-1},"414":{"limit":64,"armor":-1},"415":{"limit":64,"armor":-1},"416":{"limit":16,"armor":-1},"417":{"limit":1,"armor":-1},"418":{"limit":1,"armor":-1},"419":{"limit":1,"armor":-1},"42":{"limit":64,"armor":-1},"420":{"limit":64,"armor":-1},"421":{"limit":64,"armor":-1},"422":{"limit":1,"armor":-1},"423":{"limit":64,"armor":-1},"424":{"limit":64,"armor":-1},"425":{"limit":16,"armor":-1},"427":{"limit":64,"armor":-1},"428":{"limit":64,"armor":-1},"429":{"limit":64,"armor":-1},"430":{"limit":64,"armor":-1},"431":{"limit":64,"armor":-1},"44":{"limit":64,"armor":-1},"45":{"limit":64,"armor":-1},"46":{"limit":64,"armor":-1},"47":{"limit":64,"armor":-1},"48":{"limit":64,"armor":-1},"49":{"limit":64,"armor":-1},"5":{"limit":64,"armor":-1},"50":{"limit":64,"armor":-1},"52":{"limit":64,"armor":-1},"53":{"limit":64,"armor":-1},"54":{"limit":64,"armor":-1},"56":{"limit":64,"armor":-1},"57":{"limit":64,"armor":-1},"58":{"limit":64,"armor":-1},"6":{"limit":64,"armor":-1},"60":{"limit":64,"armor":-1},"61":{"limit":64,"armor":-1},"62":{"limit":64,"armor":-1},"65":{"limit":64,"armor":-1},"66":{"limit":64,"armor":-1},"67":{"limit":64,"armor":-1},"69":{"limit":64,"armor":-1},"7":{"limit":64,"armor":-1},"70":{"limit":64,"armor":-1},"72":{"limit":64,"armor":-1},"73":{"limit":64,"armor":-1},"76":{"limit":64,"armor":-1},"77":{"limit":64,"armor":-1},"78":{"limit":64,"armor":-1},"79":{"limit":64,"armor":-1},"80":{"limit":64,"armor":-1},"81":{"limit":64,"armor":-1},"82":{"limit":64,"armor":-1},"84":{"limit":64,"armor":-1},"85":{"limit":64,"armor":-1},"86":{"limit":64,"armor":-1},"87":{"limit":64,"armor":-1},"88":{"limit":64,"armor":-1},"89":{"limit":64,"armor":-1},"91":{"limit":64,"armor":-1},"95":{"limit":64,"armor":-1},"96":{"limit":64,"armor":-1},"97":{"limit":64,"armor":-1},"98":{"limit":64,"armor":-1},"99":{"limit":64,"armor":-1}}}"##).unwrap();
    for case in value["cases"].as_array_mut().unwrap() {
        for step in case["steps"].as_array_mut().unwrap() {
            for key in ["before", "after"] {
                let sparse = &step[key];
                let mut slots = vec![
                    serde_json::Value::String("ffff".into());
                    sparse["count"].as_u64().unwrap() as usize
                ];
                for pair in sparse["nonempty"].as_array().unwrap() {
                    slots[pair[0].as_u64().unwrap() as usize] = pair[1].clone();
                }
                step[key] = serde_json::Value::Array(slots);
            }
        }
    }
    value
}
pub(super) fn read(text: &str) -> Option<Stack> {
    Cx.read_old_slot(&mut Reader::new(&hex(text))).unwrap()
}
pub(super) fn write(item: Option<&Stack>) -> String {
    let mut w = Writer::new();
    Cx.try_write_old_stack(item, &mut w).unwrap();
    w.data.iter().map(|b| format!("{b:02x}")).collect()
}
fn contents(state: &State) -> Vec<String> {
    state
        .active()
        .slots
        .iter()
        .map(|s| write(s.as_ref()))
        .collect()
}
fn initialize(case: &serde_json::Value) -> State {
    let first = &case["steps"][0];
    let mut state = State::new();
    state.set_creative(case["creative"].as_bool().unwrap());
    if case["kind"] == "chest" {
        let mut w = Writer::new();
        w.u8(1);
        w.string("minecraft:chest");
        w.string("Fixture chest");
        w.u8(27);
        state.clientbound(0x2d, &w.data, &Cx).unwrap();
    }
    let mut w = Writer::new();
    w.u8(state.active().id);
    let before = first["before"].as_array().unwrap();
    w.i16(before.len() as i16);
    for slot in before {
        w.bytes(&hex(slot.as_str().unwrap()));
    }
    state.clientbound(0x30, &w.data, &Cx).unwrap();
    let mut w = Writer::new();
    w.i8(-1);
    w.i16(-1);
    w.bytes(&hex(first["carried_before"].as_str().unwrap()));
    state.clientbound(0x2f, &w.data, &Cx).unwrap();
    state
}
fn click(state: &State, step: &serde_json::Value) -> play::Serverbound {
    play::Serverbound::ContainerClick(play::ContainerClick {
        container: i32::from(state.active().id),
        state: state.active().state,
        slot: step["slot"].as_i64().unwrap() as i16,
        button: step["button"].as_i64().unwrap() as i8,
        input: step["mode"].as_i64().unwrap() as i32,
        // Deliberately unhelpful hashed predictions: the adapter must use cached complete old values.
        changed: vec![],
        carried: None,
    })
}

#[test]
fn every_actual_vanilla_click_result_wire_and_predicted_state_matches() {
    let fixtures = fixture();
    for case in fixtures["cases"].as_array().unwrap() {
        if case["name"] == "inventory_craft_table" {
            continue;
        } // Its recipe callback is tested through authoritative resync below.
        let mut state = initialize(case);
        for (step_index, step) in case["steps"].as_array().unwrap().iter().enumerate() {
            let packet = click(&state, step);
            let out = state.serverbound(&packet, &Cx).unwrap().unwrap();
            assert_eq!(out.len(), 1);
            assert_eq!(out[0].id, 0x0e);
            assert_eq!(
                out[0].payload,
                hex(step["click_wire"].as_str().unwrap()),
                "{} step{step_index}",
                case["name"]
            );
            assert_eq!(
                contents(&state),
                step["after"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| s.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
                "{} step{step_index}",
                case["name"]
            );
            assert_eq!(
                write(state.carried.as_ref()),
                step["carried_after"].as_str().unwrap(),
                "{} step{step_index}",
                case["name"]
            );
            assert!(
                state.resync.is_none(),
                "plain oracle case should not request resync"
            );
            state.take_incoming();
        }
    }
}

#[test]
fn rejected_transaction_ack_and_both_authoritative_resync_halves_are_required() {
    let f = fixture();
    let case = &f["cases"][0];
    let mut state = initialize(case);
    state.active_mut().action = 32767;
    let step = &case["steps"][0];
    state.serverbound(&click(&state, step), &Cx).unwrap();
    assert_eq!(state.active().action, -32768);
    state
        .clientbound(0x32, &hex(f["packets"]["reject"].as_str().unwrap()), &Cx)
        .unwrap();
    assert_eq!(
        state.take_outgoing(),
        vec![OldPacket {
            id: 0x0f,
            payload: hex(f["packets"]["ack"].as_str().unwrap())
        }]
    );
    assert!(state.main_hand(0).is_err());
    assert!(
        state
            .serverbound(&click(&state, step), &Cx)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    let mut w = Writer::new();
    w.u8(1);
    w.i16(63);
    for text in step["before"].as_array().unwrap() {
        w.bytes(&hex(text.as_str().unwrap()));
    }
    state.clientbound(0x30, &w.data, &Cx).unwrap();
    assert!(state.resync.is_some());
    state
        .clientbound(0x2f, &hex(f["packets"]["cursor"].as_str().unwrap()), &Cx)
        .unwrap();
    assert!(state.resync.is_none());
    assert_eq!(state.carried.as_ref().unwrap().count, 13);
    assert!(state.main_hand(0).is_ok());
}

#[test]
fn offhand_is_virtual_and_mainhand_mirrors_active_container_updates() {
    let f = fixture();
    let case = &f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "swap_chest_hotbar_relocates")
        .unwrap();
    let mut state = initialize(case);
    assert_eq!(state.main_hand(0).unwrap().unwrap().item, 10003);
    state
        .serverbound(&click(&state, &case["steps"][0]), &Cx)
        .unwrap();
    assert_eq!(state.main_hand(0).unwrap().unwrap().item, 10001);
    let mut w = Writer::new();
    w.i8(-2);
    w.i16(0);
    w.bytes(&hex("000105000000"));
    state.clientbound(0x2f, &w.data, &Cx).unwrap();
    assert_eq!(state.main_hand(0).unwrap().unwrap().count, 5);
    let mut plain = State::new();
    plain.inventory.known.fill(true);
    let packet = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 0,
        state: 0,
        slot: 45,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    assert!(plain.serverbound(&packet, &Cx).is_err());
    assert_eq!(plain.active().action, 0);
}

#[test]
fn malformed_packets_and_failed_reverse_item_writes_do_not_mutate_state() {
    let f = fixture();
    let mut state = initialize(&f["cases"][0]);
    let before = contents(&state);
    let action = state.active().action;
    let mut packet = hex(f["packets"]["cursor"].as_str().unwrap());
    packet.push(99);
    assert!(state.clientbound(0x2f, &packet, &Cx).is_err());
    assert_eq!(contents(&state), before);
    struct Fail;
    impl Context for Fail {
        fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
            Cx.text(r)
        }
        fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>> {
            Cx.read_old_slot(r)
        }
        fn try_write_old_stack(&self, _: Option<&Stack>, w: &mut Writer) -> Result<()> {
            w.u8(99);
            Err(Error::new("explicit unsupported reverse mapping"))
        }
    }
    assert!(
        state
            .serverbound(&click(&state, &f["cases"][0]["steps"][0]), &Fail)
            .is_err()
    );
    assert_eq!(contents(&state), before);
    assert_eq!(state.active().action, action);
    assert!(state.take_incoming().is_empty());
}

#[test]
fn actual_recipe_slot_click_requests_and_consumes_authoritative_server_state() {
    let f = fixture();
    let case = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "inventory_craft_table")
        .unwrap();
    let step = &case["steps"][0];
    let mut state = initialize(case);
    let out = state
        .serverbound(&click(&state, step), &Cx)
        .unwrap()
        .unwrap();
    let mut r = Reader::new(&out[0].payload);
    assert_eq!(
        (
            r.u8().unwrap(),
            r.i16().unwrap(),
            r.i8().unwrap(),
            r.i16().unwrap(),
            r.u8().unwrap()
        ),
        (0, 0, 0, 1, 0)
    );
    let checksum = Cx.read_old_slot(&mut r).unwrap();
    assert_ne!(checksum, read(step["returned"].as_str().unwrap()));
    // Real1.8 ServerNetHandler compares the returned checksum after applying the click and sends
    // rejection + all slots + carried. Do not reproduce modern recipes or trust hash predictions.
    assert!(state.resync.is_some());
    let mut reject = Writer::new();
    reject.u8(0);
    reject.i16(1);
    reject.bool(false);
    state.clientbound(0x32, &reject.data, &Cx).unwrap();
    assert_eq!(
        state.take_outgoing()[0],
        OldPacket {
            id: 0x0f,
            payload: vec![0, 0, 1, 1]
        }
    );
    let mut content = Writer::new();
    content.u8(0);
    let after = step["after"].as_array().unwrap();
    content.i16(after.len() as i16);
    for item in after {
        content.bytes(&hex(item.as_str().unwrap()));
    }
    state.clientbound(0x30, &content.data, &Cx).unwrap();
    let mut cursor = Writer::new();
    cursor.i8(-1);
    cursor.i16(-1);
    cursor.bytes(&hex(step["carried_after"].as_str().unwrap()));
    state.clientbound(0x2f, &cursor.data, &Cx).unwrap();
    assert!(state.resync.is_none());
    assert_eq!(
        contents(&state),
        after
            .iter()
            .map(|s| s.as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        write(state.carried.as_ref()),
        step["carried_after"].as_str().unwrap()
    );
}

#[test]
fn modern_hash_predictions_cannot_replace_cached_old_item_values() {
    let f = fixture();
    let mut state = initialize(&f["cases"][0]);
    let mut p = click(&state, &f["cases"][0]["steps"][0]);
    if let play::Serverbound::ContainerClick(p) = &mut p {
        let invented = Some(Hashed::of(&Stack::new(987654, 99)));
        p.changed = vec![(0, invented.clone())];
        p.carried = invented;
    }
    let out = state.serverbound(&p, &Cx).unwrap().unwrap();
    assert_eq!(
        out[0].payload,
        hex(f["cases"][0]["steps"][0]["click_wire"].as_str().unwrap())
    );
    assert_eq!(state.carried.as_ref().unwrap().item, 10001);
    let corrections = state.take_incoming();
    assert!(
        matches!(&corrections[0],play::Clientbound::ContainerSetContent(p) if p.carried.as_ref().unwrap().item==10001)
    );
}

#[test]
fn brewing_inserts_virtual_fuel_without_shifting_old_player_slots_incorrectly() {
    let f = fixture();
    let mut state = State::new();
    let open = state
        .clientbound(
            0x2d,
            &hex(f["packets"]["open_brewing"].as_str().unwrap()),
            &Cx,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(&open[0],play::Clientbound::OpenScreen(p) if p.menu==11&&p.container==2));
    let mut w = Writer::new();
    w.u8(2);
    w.i16(40);
    for i in 0..40 {
        if i == 31 {
            w.bytes(&hex("000105000000"));
        } else {
            w.i16(-1);
        }
    }
    let content = state.clientbound(0x30, &w.data, &Cx).unwrap().unwrap();
    assert!(
        matches!(&content[0],play::Clientbound::ContainerSetContent(p)if p.items.len()==41&&p.items[4].is_none()&&p.items[32].as_ref().unwrap().count==5)
    );
    assert_eq!(state.main_hand(0).unwrap().unwrap().count, 5);
    let click = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 2,
        state: 0,
        slot: 4,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    assert!(state.serverbound(&click, &Cx).is_err());
}

#[test]
fn creative_reverse_callback_is_scoped_to_creative_slot_packets() {
    use std::cell::Cell;
    struct CreativeCx {
        creative: Cell<u32>,
        strict: Cell<u32>,
    }
    impl Context for CreativeCx {
        fn text(&self, r: &mut Reader<'_>) -> Result<Tag> {
            Cx.text(r)
        }
        fn read_old_slot(&self, r: &mut Reader<'_>) -> Result<Option<Stack>> {
            Cx.read_old_slot(r)
        }
        fn try_write_old_stack(&self, _: Option<&Stack>, _: &mut Writer) -> Result<()> {
            self.strict.set(self.strict.get() + 1);
            Err(Error::new("strict original deliberately unavailable"))
        }
        fn try_write_creative_stack(&self, s: Option<&Stack>, w: &mut Writer) -> Result<()> {
            self.creative.set(self.creative.get() + 1);
            Cx.try_write_old_stack(s, w)
        }
    }
    let cx = CreativeCx {
        creative: Cell::new(0),
        strict: Cell::new(0),
    };
    let f = fixture();
    let mut state = initialize(&f["cases"][0]);
    state.set_creative(true);
    let item = read("000140000000");
    let out = state
        .serverbound(
            &play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot { slot: 36, item }),
            &cx,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        out[0].payload,
        hex(f["packets"]["creative_hotbar"].as_str().unwrap())
    );
    assert_eq!(state.main_hand(0).unwrap().unwrap().count, 64);
    assert_eq!(cx.creative.get(), 1);
    assert_eq!(cx.strict.get(), 0);
    let p = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 1,
        state: 0,
        slot: 54,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    assert!(state.serverbound(&p, &cx).is_err());
    assert_eq!(cx.creative.get(), 1);
    assert!(cx.strict.get() > 0);
}

#[test]
fn drag_start_and_add_do_not_interrupt_native_drag_with_redundant_contents() {
    let f = fixture();
    let case = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "drag_even")
        .unwrap();
    let mut state = initialize(case);
    for (i, step) in case["steps"].as_array().unwrap().iter().enumerate() {
        let mut p = click(&state, step);
        if let play::Serverbound::ContainerClick(p) = &mut p {
            p.carried = state.carried.as_ref().map(Hashed::of);
        }
        state.serverbound(&p, &Cx).unwrap();
        let updates = state.take_incoming();
        if i + 1 < case["steps"].as_array().unwrap().len() {
            assert!(updates.is_empty());
        } else {
            assert_eq!(updates.len(), 1);
        }
    }
}

#[test]
fn portal_reset_preserves_inventory_and_drops_only_remote_menu_lifecycle() {
    let f = fixture();
    let case = f["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "inventory_quickmove_hotbar")
        .unwrap();
    let mut state = initialize(case);
    state.set_creative(true);
    let held = state.main_hand(8).unwrap().cloned();
    state
        .clientbound(
            0x2d,
            &hex(f["packets"]["open_chest"].as_str().unwrap()),
            &Cx,
        )
        .unwrap();
    let mut contents = Writer::new();
    contents.u8(1);
    contents.i16(63);
    for _ in 0..27 {
        contents.i16(-1);
    }
    for item in &state.inventory.slots[9..45] {
        Cx.try_write_old_stack(item.as_ref(), &mut contents)
            .unwrap();
    }
    state.clientbound(0x30, &contents.data, &Cx).unwrap();
    let click = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 1,
        state: 0,
        slot: 0,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    state.serverbound(&click, &Cx).unwrap();
    let mut reject = Writer::new();
    reject.u8(1);
    reject.i16(1);
    reject.bool(false);
    state.clientbound(0x32, &reject.data, &Cx).unwrap();
    assert!(state.main_hand(8).is_err());
    state.on_respawn();
    assert_eq!(state.main_hand(8).unwrap().cloned(), held);
    assert!(state.take_outgoing().is_empty());
    assert!(state.take_incoming().is_empty());
    let click = play::Serverbound::ContainerClick(play::ContainerClick {
        container: 0,
        state: 0,
        slot: 44,
        button: 0,
        input: 0,
        changed: vec![],
        carried: None,
    });
    assert!(!state.serverbound(&click, &Cx).unwrap().unwrap().is_empty());
    let creative = play::Serverbound::SetCreativeModeSlot(play::SetCreativeModeSlot {
        slot: 36,
        item: read("000101000000"),
    });
    assert!(state.serverbound(&creative, &Cx).is_ok());
}

#[test]
fn owned_packet_truncations_are_atomic_and_unknown_false_confirmations_are_acknowledged() {
    let f = fixture();
    for (name, id) in [
        ("open_chest", 0x2d),
        ("property", 0x31),
        ("cursor", 0x2f),
        ("reject", 0x32),
    ] {
        let bytes = hex(f["packets"][name].as_str().unwrap());
        for length in 0..bytes.len() {
            let mut state = initialize(&f["cases"][0]);
            let before = contents(&state);
            assert!(
                state.clientbound(id, &bytes[..length], &Cx).is_err(),
                "{name} {length}"
            );
            assert_eq!(state.active().id, 1);
            assert_eq!(contents(&state), before);
            assert!(state.take_outgoing().is_empty());
        }
    }
    let mut state = initialize(&f["cases"][0]);
    state
        .serverbound(&click(&state, &f["cases"][0]["steps"][0]), &Cx)
        .unwrap();
    let pending = state.pending.len();
    state
        .clientbound(0x32, &hex(f["packets"]["reject"].as_str().unwrap()), &Cx)
        .unwrap();
    assert_eq!(state.pending.len(), pending);
    assert_eq!(
        state.take_outgoing()[0].payload,
        hex(f["packets"]["ack"].as_str().unwrap())
    );
    assert!(state.resync.is_none());
    let property = state
        .clientbound(0x31, &hex(f["packets"]["property"].as_str().unwrap()), &Cx)
        .unwrap()
        .unwrap();
    assert!(
        matches!(&property[0],play::Clientbound::ContainerSetData(p) if p.id==2&&p.value==1234)
    );
    assert!(state.correction().is_some());
    let mut inventory = State::new();
    let close = inventory
        .clientbound(
            0x2e,
            &hex(f["packets"]["close_inventory"].as_str().unwrap()),
            &Cx,
        )
        .unwrap()
        .unwrap();
    assert!(matches!(&close[0],play::Clientbound::ContainerClose(p) if p.container==0));
}
