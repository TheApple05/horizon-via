//! Native version picker, independently implemented from ViaFabricPlus's published UI screenshot
//! and user guide. No mod implementation source or assets are used.
use super::{ButtonSpec, Screen};
use crate::{
    hud::{HudAtlas, HudBatch, rgba},
    item_render::{self, ItemRenderer, NoHost},
    items::ItemStack,
    lang, ui,
};
use horizon_assets::item_model::{DisplayContext, ItemEnv};
use horizon_protocol::adapter::ServerVersion;
use std::time::Instant;

pub(super) const SEARCH_ID: crate::clipboard::FieldId = ("server.version_search", 0);
const TOP: f32 = 84.0;
const ROW: f32 = 32.0;
const BLUE: u32 = 0xFFFFAA55;

#[derive(Clone, Debug)]
pub struct Selector {
    pub(super) previous: Box<Screen>,
    pub(super) query: String,
    pub(super) search_focused: bool,
    pub(super) scroll: ui::Scroll,
}

fn tr(key: &str) -> String {
    lang::get(&format!("horizon.multiplayer.horizon_via.{key}"))
}

fn version_name(version: Option<ServerVersion>) -> String {
    match version {
        None => tr("auto"),
        Some(ServerVersion::V26_1) => "26.1–26.1.2".into(),
        Some(ServerVersion::V1_21_10) => "1.21.9–1.21.10".into(),
        Some(ServerVersion::V1_21_8) => "1.21.7–1.21.8".into(),
        Some(ServerVersion::V1_21_3) => "1.21.2–1.21.3".into(),
        Some(ServerVersion::V1_21_1) => "1.21–1.21.1".into(),
        Some(ServerVersion::V1_20_5) => "1.20.5–1.20.6".into(),
        Some(ServerVersion::V1_20_3) => "1.20.3–1.20.4".into(),
        Some(ServerVersion::V1_20_1) => "1.20–1.20.1".into(),
        Some(version) => version.name().into(),
    }
}

// Release names and dates follow the published selector and Mojang release announcements.
fn details(version: Option<ServerVersion>) -> (&'static str, &'static str, &'static str) {
    match version {
        None => ("", "", "spyglass"),
        Some(ServerVersion::Current) => ("native", "", "compass"),
        Some(ServerVersion::V26_2) => ("chaos_cubed", "Jun 16, 2026", "totem_of_undying"),
        Some(ServerVersion::V26_1) => ("tiny_takeover", "Mar 24, 2026", "golden_dandelion"),
        Some(ServerVersion::V1_21_11) => ("mounts_of_mayhem", "Dec 9, 2025", "copper_horse_armor"),
        Some(ServerVersion::V1_21_10) => ("copper_age", "Sep 30, 2025", "copper_lantern"),
        Some(ServerVersion::V1_21_8) => ("chase_the_skies", "Jun 30, 2025", "compass"),
        Some(ServerVersion::V1_21_6) => ("chase_the_skies", "Jun 17, 2025", "dried_ghast"),
        Some(ServerVersion::V1_21_5) => ("spring_to_life", "Mar 25, 2025", "wildflowers"),
        Some(ServerVersion::V1_21_4) => ("garden_awakens", "Dec 3, 2024", "resin_clump"),
        Some(ServerVersion::V1_21_3) => ("bundles_of_bravery", "Oct 22, 2024", "bundle"),
        Some(ServerVersion::V1_21_1) => ("tricky_trials", "Jun 13, 2024", "trial_key"),
        Some(ServerVersion::V1_20_5) => ("armored_paws", "Apr 23, 2024", "armadillo_scute"),
        Some(ServerVersion::V1_20_3) => ("trails_tales", "Dec 5, 2023", "decorated_pot"),
        Some(ServerVersion::V1_20_2) => ("trails_tales", "Sep 21, 2023", "brush"),
        Some(ServerVersion::V1_20_1) => ("trails_tales", "Jun 7, 2023", "cherry_sapling"),
        Some(ServerVersion::V1_8_9) => ("bountiful_update", "", "slime_block"),
    }
}

fn entries(query: &str) -> Vec<Option<ServerVersion>> {
    let query = query.trim().to_lowercase();
    std::iter::once(None)
        .chain(ServerVersion::ALL.into_iter().map(Some))
        .filter(|&version| {
            let (update, _, _) = details(version);
            version_name(version).to_lowercase().contains(&query)
                || (!update.is_empty() && tr(update).to_lowercase().contains(&query))
                || matches!(version, Some(ServerVersion::V26_1)) && query == "26.1.1"
        })
        .collect()
}

pub(super) fn search_box(w: f32) -> [f32; 4] {
    let width = 260.0_f32.min(w.ceil() - 20.0);
    [((w.ceil() - width) / 2.0).floor(), 36.0, width, 20.0]
}

fn area((w, h): (f32, f32)) -> [f32; 4] {
    let width = 280.0_f32.min(w.ceil() - 20.0);
    [
        ((w.ceil() - width) / 2.0).floor(),
        TOP,
        width,
        h.ceil() - 28.0,
    ]
}

impl Selector {
    pub(super) fn new(previous: Screen) -> Self {
        Self {
            previous: Box::new(previous),
            query: String::new(),
            search_focused: false,
            scroll: ui::Scroll::default(),
        }
    }

    pub(super) fn buttons(&self, _w: f32, _h: f32) -> Vec<ButtonSpec> {
        vec![(5.0, 5.0, 60.0, lang::get_or("gui.back", "Back"), true, 62)]
    }

    pub(super) fn search_changed(&mut self) {
        self.scroll = ui::Scroll::default();
    }

    pub(super) fn blur_field(&mut self) {
        self.search_focused = false;
    }

    pub(super) fn tab_dir(&mut self, reverse: bool, buttons: &[u8], button: &mut Option<u8>) {
        let stops = buttons.len() + 1;
        let at = button
            .and_then(|id| buttons.iter().position(|&b| b == id))
            .map_or(0, |i| i + 1);
        let next = if reverse {
            (at + stops - 1) % stops
        } else {
            (at + 1) % stops
        };
        self.search_focused = next == 0;
        *button = (next > 0).then(|| buttons[next - 1]);
    }

    pub(super) fn draw(
        &self,
        atlas: &HudAtlas,
        batch: &mut HudBatch,
        caret: &mut ui::Caret,
        focused_at: Instant,
        cursor: (f32, f32),
        items: Option<&ItemRenderer>,
    ) {
        let (w, h) = (batch.width.ceil(), batch.height.ceil());
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 150));
        let title = tr("title");
        let tw = HudBatch::text_width(atlas, &title);
        let first = batch.vertices.len();
        batch.text(atlas, &title, (w - tw) / 2.0, 14.0, BLUE, true);
        batch.scale_since(first, w / 2.0, 18.0, 1.35, 1.35);
        let search = search_box(w);
        caret.draw(
            batch,
            atlas,
            (SEARCH_ID, &self.query, 128),
            search,
            self.search_focused,
            focused_at,
        );
        crate::widgets::name_last_edit_box(|| tr("search"));
        if self.query.is_empty() && !self.search_focused {
            batch.text(
                atlas,
                &tr("search"),
                search[0] + 5.0,
                search[1] + 6.0,
                rgba(140, 140, 140, 255),
                false,
            );
        }
        let tab_width = 60.0_f32.min(((w - 10.0) / 6.0).floor());
        let tab_left = ((w - tab_width * 6.0) / 2.0).floor();
        for (i, name) in [
            "modern",
            "legacy",
            "beta",
            "alpha",
            "classic",
            "april_fools",
        ]
        .into_iter()
        .enumerate()
        {
            let label = tr(name);
            let x = tab_left + i as f32 * tab_width;
            let y = if i == 0 { 61.0 } else { 65.0 };
            let height = TOP - y;
            let rect = [x, y, tab_width - 2.0, height];
            batch.fill(atlas, x, y, rect[2], height, rgba(100, 100, 100, 170));
            batch.fill(
                atlas,
                x + 1.0,
                y + 1.0,
                rect[2] - 2.0,
                height - 1.0,
                rgba(0, 0, 0, 225),
            );
            let text_width = HudBatch::text_width(atlas, &label);
            batch.text(
                atlas,
                &label,
                x + (rect[2] - text_width) / 2.0,
                70.0,
                if i == 0 {
                    u32::MAX
                } else {
                    rgba(140, 140, 140, 255)
                },
                true,
            );
            crate::widgets::record(&label, rect, i == 0);
            if i == 0 {
                crate::widgets::target(rect);
            }
        }
        let [left, top, width, bottom] = area((w, h));
        let versions = entries(&self.query);
        let content = versions.len() as f32 * ROW;
        let mut scroll = self.scroll;
        scroll.clamp(top, bottom, content);
        let selected = crate::net::selected_server_version();
        crate::ui::separators(batch, atlas, top, bottom, false);
        batch.fill(
            atlas,
            tab_left + 12.0,
            top - 1.0,
            tab_width - 26.0,
            1.0,
            u32::MAX,
        );
        let first = batch.vertices.len();
        let hovered = self.row_at(cursor, (w, h));
        crate::widgets::narratable(|| {
            use crate::widgets::{ListKind, Narratable, Row};
            let row = |i| Row::new(i, version_name(versions[i]));
            Narratable::list(
                ListKind::Object,
                versions.len(),
                hovered.map(row),
                versions.iter().position(|&v| v == selected).map(row),
                [left, top, width, bottom - top],
            )
            .key(crate::widgets::key_of(&"multiplayer/versions"))
        });
        for (i, &version) in versions.iter().enumerate() {
            let y = top + i as f32 * ROW - scroll.amount.trunc();
            if y + ROW <= top || y >= bottom {
                continue;
            }
            let rect = [left, y, width - 8.0, ROW];
            let name = version_name(version);
            if version == selected {
                batch.fill(atlas, left, y, width - 8.0, ROW, rgba(29, 56, 83, 220));
            } else if hovered == Some(i) {
                batch.fill(atlas, left, y, width - 8.0, ROW, rgba(255, 255, 255, 24));
            } else if i % 2 == 0 {
                batch.fill(atlas, left, y, width - 8.0, ROW, rgba(0, 0, 0, 80));
            }
            if y >= top && y + ROW <= bottom {
                crate::widgets::record(&name, rect, true);
                crate::widgets::target(rect);
            }
            let (update, date, icon) = details(version);
            draw_icon(
                atlas,
                items,
                batch,
                icon,
                left + 4.0,
                y + 8.0,
                [left, top, left + width, bottom],
            );
            let color = if version == selected { BLUE } else { u32::MAX };
            batch.text(
                atlas,
                &name,
                left + 24.0,
                y + if update.is_empty() { 12.0 } else { 6.0 },
                color,
                true,
            );
            if !update.is_empty() {
                batch.text(
                    atlas,
                    &tr(update),
                    left + 24.0,
                    y + 17.0,
                    rgba(150, 150, 150, 255),
                    true,
                );
            }
            let dw = HudBatch::text_width(atlas, date);
            batch.text(
                atlas,
                date,
                left + width - 12.0 - dw,
                y + 12.0,
                rgba(135, 135, 135, 255),
                true,
            );
        }
        batch.clip_since(first, left, top, left + width, bottom);
        ui::scrollbar(
            batch,
            atlas,
            left + width - 6.0,
            top,
            bottom,
            content,
            scroll.amount,
        );
        if versions.is_empty() {
            let message = tr("no_results");
            batch.text(
                atlas,
                &message,
                (w - HudBatch::text_width(atlas, &message)) / 2.0,
                top + 16.0,
                rgba(160, 160, 160, 255),
                true,
            );
        }
        let note = tr("experimental");
        batch.text(
            atlas,
            &note,
            (w - HudBatch::text_width(atlas, &note)) / 2.0,
            h - 17.0,
            rgba(150, 150, 150, 255),
            true,
        );
    }

    fn row_at(&self, cursor: (f32, f32), size: (f32, f32)) -> Option<usize> {
        let [left, top, width, bottom] = area(size);
        if !ui::hovered(cursor, left, top, width - 8.0, bottom - top) {
            return None;
        }
        let versions = entries(&self.query);
        let mut scroll = self.scroll;
        scroll.clamp(top, bottom, versions.len() as f32 * ROW);
        let row = ((cursor.1 - top + scroll.amount.trunc()) / ROW).floor() as usize;
        (row < versions.len()).then_some(row)
    }

    pub(super) fn click(&mut self, id: Option<u8>, x: f32, y: f32, size: (f32, f32)) {
        if id.is_some() {
            self.search_focused = false;
            return;
        }
        let [sx, sy, sw, sh] = search_box(size.0);
        self.search_focused = ui::hovered((x, y), sx, sy, sw, sh);
        let [left, top, width, bottom] = area(size);
        if self.scroll.press(
            (x, y),
            left + width - 6.0,
            top,
            bottom,
            entries(&self.query).len() as f32 * ROW,
        ) {
            return;
        }
        if let Some(row) = self.row_at((x, y), size) {
            crate::net::set_server_version(entries(&self.query)[row]);
        }
    }

    pub(super) fn wheel(&mut self, delta: f32, cursor: (f32, f32), size: (f32, f32)) {
        let [left, top, width, bottom] = area(size);
        if ui::hovered(cursor, left, top, width, bottom - top) {
            self.scroll.wheel(
                delta,
                ROW / 2.0,
                top,
                bottom,
                entries(&self.query).len() as f32 * ROW,
            );
        }
    }

    pub(super) fn drag(&mut self, y: f32, size: (f32, f32)) {
        let [_, top, _, bottom] = area(size);
        self.scroll
            .drag(y, top, bottom, entries(&self.query).len() as f32 * ROW);
    }

    pub(super) fn release(&mut self) {
        self.scroll.release();
    }
}

struct IconEnv;
impl ItemEnv for IconEnv {}

fn draw_icon(
    atlas: &HudAtlas,
    renderer: Option<&ItemRenderer>,
    batch: &mut HudBatch,
    icon: &str,
    x: f32,
    y: f32,
    clip: [f32; 4],
) {
    if let Some(renderer) = renderer {
        let stack = ItemStack::new(&format!("minecraft:{icon}"), 1);
        let state = renderer.resolve(
            &item_render::stack_view(&stack),
            DisplayContext::Gui,
            None,
            None,
            0,
            &IconEnv,
        );
        item_render::gui::draw_scissored(
            &state,
            renderer,
            &atlas.sprites,
            &NoHost,
            batch,
            x,
            y,
            clip,
        );
    } else {
        batch.sprite_scaled(atlas, &format!("item/{icon}"), x, y, 16.0, 16.0, u32::MAX);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_search_finds_shared_protocol_aliases_and_excludes_other_releases() {
        assert_eq!(entries("1.21.7"), vec![Some(ServerVersion::V1_21_8)]);
        assert_eq!(entries("26.1.1"), vec![Some(ServerVersion::V26_1)]);
        assert_eq!(entries("1.21.2"), vec![Some(ServerVersion::V1_21_3)]);
        assert!(entries("1.21.1").contains(&Some(ServerVersion::V1_21_1)));
        assert_eq!(entries("1.20.5"), vec![Some(ServerVersion::V1_20_5)]);
        assert_eq!(entries("1.20.6"), vec![Some(ServerVersion::V1_20_5)]);
        assert_eq!(entries("1.20.3"), vec![Some(ServerVersion::V1_20_3)]);
        assert_eq!(entries("1.20.4"), vec![Some(ServerVersion::V1_20_3)]);
        assert_eq!(entries("1.20.2"), vec![Some(ServerVersion::V1_20_2)]);
        assert_eq!(entries("1.20.1"), vec![Some(ServerVersion::V1_20_1)]);
        assert_eq!(
            entries("1.20"),
            vec![
                Some(ServerVersion::V1_20_5),
                Some(ServerVersion::V1_20_3),
                Some(ServerVersion::V1_20_2),
                Some(ServerVersion::V1_20_1),
            ]
        );
        assert_eq!(version_name(Some(ServerVersion::V1_20_1)), "1.20–1.20.1");
        assert!(entries("1.19.4").is_empty());
        assert_eq!(entries("1.8.9"), vec![Some(ServerVersion::V1_8_9)]);
        assert!(entries("1.7.10").is_empty());
    }

    #[test]
    fn every_picker_update_label_has_an_english_translation() {
        assert_eq!(ServerVersion::ALL.len(), 16);
        let translations: serde_json::Value =
            serde_json::from_str(include_str!("../../assets/horizon/lang/en_us.json")).unwrap();
        for version in ServerVersion::ALL {
            let (update, _, _) = details(Some(version));
            let key = format!("horizon.multiplayer.horizon_via.{update}");
            assert!(
                translations
                    .get(&key)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "missing picker translation {key}"
            );
        }
    }

    #[test]
    fn filtered_rows_use_the_visible_scroll_position_and_ignore_header_and_footer() {
        let mut selector = Selector::new(Screen::Direct {
            address: "example.invalid".into(),
        });
        selector.query = "1.21.4".into();
        selector.scroll.amount = 500.0;
        assert_eq!(selector.row_at((120.0, 90.0), (427.0, 240.0)), Some(0));
        assert_eq!(selector.row_at((120.0, 70.0), (427.0, 240.0)), None);
        assert_eq!(selector.row_at((120.0, 220.0), (427.0, 240.0)), None);
    }
}
