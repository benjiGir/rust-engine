//! Informations lisibles des objets et des blocs ; aucun identifiant technique affiché.
use crate::skin;
use engine_render::egui::{self, Color32};
use engine_voxel::{BlockDef, Capabilities, Category, FluidRegistry, ItemDef, ToolDef, ToolKind};

#[derive(Debug, Clone)]
pub struct Card {
    pub name: String,
    /// La couleur du nom (la rareté d'un objet à affixes).
    pub name_color: Option<Color32>,
    pub origin: String,
    pub lines: Vec<(String, Color32)>,
}

pub fn origin(key: &str) -> String {
    match key.split(':').next().unwrap_or("core") {
        "core" => "Jeu de base".into(),
        "example" => "Exemple".into(),
        "mecanique" => "Mécanique".into(),
        "magie" => "Magie".into(),
        "donjons" => "Donjons".into(),
        "entrepot" => "Entrepôt".into(),
        "forge" => "Forge".into(),
        other => other.replace('_', " "),
    }
}

pub fn item(def: &ItemDef, count: u32, damage: u32) -> Card {
    let category = match def.category {
        Category::Block => "Bloc",
        Category::Material => "Matériau",
        Category::Tool => "Outil",
        Category::Armor => "Équipement",
        Category::Food => "Aliment",
    };
    let mut lines = vec![(
        if count > 1 {
            format!("{category} · {count} objets")
        } else {
            category.into()
        },
        skin::MUTED,
    )];
    if let Some(max) = def.durability {
        lines.push((
            format!(
                "Durabilité : {} / {max}",
                max.saturating_sub(crate::enchanting::wear(damage))
            ),
            skin::TEXT,
        ));
        // Un charme de potion : son état et ce qui le recharge (voir `charms.rs`).
        lines.extend(
            crate::charms::card_lines(
                &crate::adventure_data::current(),
                &def.key,
                damage,
                Some(max),
            )
            .into_iter()
            .map(|(text, tone)| (text, skin::tone(tone))),
        );
    }
    // Tous les enchantements, un par ligne (outils, armures, livres enchantés).
    for label in crate::enchanting::labels(damage) {
        lines.push((label, skin::MAGIC));
    }
    // La rareté, les affixes et les gemmes de la fiche (objets à affixes, voir `affixes.rs`).
    let sheet = crate::item_data::sheet(damage);
    let data = crate::adventure_data::current();
    let rarity = sheet
        .as_ref()
        .and_then(crate::affixes::display_rarity)
        .and_then(|r| data.rarity(r));
    let rarity_color = rarity.map(|r| Color32::from_rgb(r.color.0, r.color.1, r.color.2));
    if let (Some(rarity), Some(color)) = (rarity, rarity_color) {
        lines.insert(0, (rarity.name.clone(), color));
    }
    for affix in sheet.iter().flat_map(|s| &s.affixes) {
        if let Some(text) = crate::affixes::describe(affix) {
            lines.push((text, skin::GOLD));
        }
    }
    for socket in sheet.iter().flat_map(|s| &s.sockets) {
        lines.push(match socket {
            None => ("Emplacement de gemme : vide".into(), skin::MUTED),
            Some(key) => match crate::gems::gem_label(&data, key) {
                Some((name, tone)) => (format!("Gemme : {name}"), skin::tone(tone)),
                None => ("Gemme inconnue".into(), skin::MUTED),
            },
        });
    }
    // Une gemme dit ce qu'elle apporte selon l'objet qui la porte ; un sceau, ce qu'il fait.
    if let Some((_, _)) = data.gem_of(&def.key) {
        for text in crate::gems::gem_effects(&data, &def.key) {
            lines.push((text, skin::GOLD));
        }
        lines.push((
            "Tenue au curseur, un clic sur un objet à emplacement libre la sertit".into(),
            skin::MUTED,
        ));
    }
    match def.key.as_str() {
        crate::enchant_books::ENCHANTED_BOOK => lines.push((
            "Dans l'écran d'une table d'enchantement ou d'une enclume, tenu au curseur : un clic sur un objet lui apporte ces enchantements".into(),
            skin::MUTED,
        )),
        crate::enchant_books::TOME_EXTRACTION => lines.push((
            "Dans l'écran d'une table d'enchantement ou d'une enclume, tenu au curseur : un clic sur un objet enchanté en retire les enchantements sur un livre vierge".into(),
            skin::MUTED,
        )),
        key if crate::enchant_books::BookKind::of_key(key).is_some() => lines.push((
            "En main devant une table d'enchantement : devient un livre enchanté (un tome ne reçoit que les enchantements de son type)".into(),
            skin::MUTED,
        )),
        crate::gems::SEAL_UNSOCKET => lines.push((
            "Au curseur, un clic sur un objet : retire ses gemmes".into(),
            skin::MUTED,
        )),
        crate::gems::SEAL_SOCKET => lines.push((
            "Au curseur, un clic sur un objet : un emplacement de plus".into(),
            skin::MUTED,
        )),
        _ => {}
    }
    if let Some(tool) = def.tool {
        lines.push((format!("Dégâts : {:.0}", tool.damage), skin::MUTED));
    }
    // Le stockage : l'effet d'une amélioration, l'usage d'un kit (le numéro d'un sac est ajouté
    // par `backpacks::card_lines`).
    if let Some(upgrade) = engine_voxel::Upgrade::from_key(&def.key) {
        lines.push((upgrade.help().into(), skin::TEXT));
        lines.push((
            match (upgrade.in_chest(), upgrade.in_backpack()) {
                (true, true) => "Coffre ou sac à dos",
                (true, false) => "Coffre seulement",
                _ => "Sac à dos seulement",
            }
            .into(),
            skin::MUTED,
        ));
    }
    if let Some(kit) = engine_voxel::kit_of(&def.key) {
        lines.push((
            format!(
                "Clic droit sur un coffre : {}",
                if kit.direct {
                    format!(
                        "monte à {} depuis n'importe quel palier inférieur",
                        engine_voxel::CHEST_TIERS[kit.to].name
                    )
                } else {
                    format!(
                        "{} → {}",
                        engine_voxel::CHEST_TIERS[kit.to - 1].name,
                        engine_voxel::CHEST_TIERS[kit.to].name
                    )
                }
            ),
            skin::TEXT,
        ));
    }
    if let Some(armor) = def.armor {
        lines.push((format!("Protection : {}", armor.protection), skin::TEXT));
    }
    if let Some(food) = def.food {
        lines.push((format!("Nourriture : {:.0}", food.nutrition), skin::TEXT));
    }
    if def.key.ends_with("_bucket") && damage > 0 {
        lines.push((
            format!(
                "Contient {:.0} % d’un bloc",
                damage.min(engine_voxel::FLUID_FULL as u32) as f32 * 100.0
                    / engine_voxel::FLUID_FULL as f32
            ),
            skin::WATER,
        ));
    }
    Card {
        name: sheet
            .and_then(|s| s.name)
            .unwrap_or_else(|| def.name.clone()),
        name_color: rarity_color,
        origin: origin(&def.key),
        lines,
    }
}

pub fn block(def: &BlockDef, held: Option<&ToolDef>) -> Card {
    let mut lines = Vec::new();
    if engine_voxel::break_time(def, held).is_none() {
        lines.push(("Incassable".into(), skin::MUTED));
    } else if let Some(tool) = def.tool {
        let name = match tool {
            ToolKind::Pickaxe => "Pioche",
            ToolKind::Axe => "Hache",
            ToolKind::Shovel => "Pelle",
            ToolKind::Hoe => "Houe",
            ToolKind::Sword => "Épée",
            ToolKind::Shears => "Cisailles",
        };
        let harvest = engine_voxel::can_harvest(def, held);
        lines.push((
            if def.tier > 0 {
                format!("{name} · niveau {} requis", def.tier)
            } else {
                format!("{name} conseillée")
            },
            if harvest { skin::MUTED } else { skin::GOLD },
        ));
        if !harvest {
            lines.push((
                "Outil insuffisant pour récupérer ce bloc".into(),
                skin::GOLD,
            ));
        }
    }
    Card {
        name: def.name.clone(),
        name_color: None,
        origin: origin(&def.key),
        lines,
    }
}

/// Les capacités d'un bloc actif. `main` : le nombre de cases de contenu d'un coffre à paliers
/// (les emplacements d'amélioration et les filtres ne comptent pas), `None` pour les autres.
pub fn capabilities(
    caps: &Capabilities,
    fluids: Option<&FluidRegistry>,
    main: Option<usize>,
) -> Vec<(String, Color32)> {
    let mut lines = Vec::new();
    if let Some(e) = &caps.energy {
        lines.push((
            format!("Énergie : {} / {}", e.amount, e.capacity),
            skin::GOLD,
        ));
    }
    for t in &caps.tanks {
        let name = t
            .fluid
            .as_deref()
            .and_then(|id| fluids.and_then(|f| f.get(id)))
            .map_or(if t.fluid.is_some() { "Fluide" } else { "Vide" }, |f| {
                f.name.as_str()
            });
        lines.push((
            format!("{name} : {} / {} mB", t.amount, t.capacity),
            skin::WATER,
        ));
    }
    if let Some(k) = &caps.kinetic {
        lines.push((format!("Rotation : {:.0} tr/min", k.speed), skin::COPPER));
    }
    if let Some(i) = &caps.inventory {
        lines.push((
            format!(
                "Stockage : {} / {} cases",
                i.slots
                    .iter()
                    .take(main.unwrap_or(usize::MAX))
                    .filter(|s| s.is_some())
                    .count(),
                main.unwrap_or(i.slots.len()).min(i.slots.len())
            ),
            skin::MUTED,
        ));
    }
    lines
}

pub fn draw(ui: &mut egui::Ui, card: &Card, icon: Option<egui::TextureId>) {
    ui.horizontal(|ui| {
        if let Some(icon) = icon {
            ui.image(egui::load::SizedTexture::new(icon, egui::vec2(32.0, 32.0)));
        }
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(&card.name)
                    .strong()
                    .size(16.0)
                    .color(card.name_color.unwrap_or(skin::TEXT)),
            );
            ui.label(
                egui::RichText::new(&card.origin)
                    .size(12.0)
                    .color(skin::MUTED),
            );
        });
    });
    for (text, color) in card.lines.iter().take(16) {
        ui.label(egui::RichText::new(text).size(13.0).color(*color));
    }
}

pub fn hover(response: egui::Response, card: &Card) {
    let mut tooltip = egui::Tooltip::for_enabled(&response).at_pointer();
    tooltip.popup = tooltip.popup.frame(
        egui::Frame::new()
            .fill(egui::Color32::from_rgb(19, 17, 28))
            .stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(96, 82, 131)))
            .inner_margin(8),
    );
    tooltip.show(|ui| {
        let size = ui.ctx().content_rect().size();
        ui.set_max_width((size.x - 32.0).clamp(80.0, 280.0));
        egui::ScrollArea::vertical()
            .max_height((size.y - 48.0).clamp(40.0, 380.0))
            .show(ui, |ui| draw(ui, card, None));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durability_decodes_enchantment_bits_and_clamps_broken_items() {
        let blocks = engine_voxel::BlockRegistry::builtin();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let def = items.get(items.id("core:iron_pickaxe").unwrap());
        let damage =
            crate::enchanting::with_enchantment(3, crate::enchanting::Enchantment::EFFICIENCY, 2);
        let card = item(def, 1, damage);
        assert!(card.lines.iter().any(|(s, _)| s
            == &format!(
                "Durabilité : {} / {}",
                def.durability.unwrap() - 3,
                def.durability.unwrap()
            )));
        assert!(card.lines.iter().any(|(_, c)| *c == skin::MAGIC));
        assert!(
            item(def, 1, 65535)
                .lines
                .iter()
                .any(|(s, _)| s.starts_with("Durabilité : 0 /"))
        );
    }

    #[test]
    fn harvest_hint_uses_the_game_rule_and_unbreakable_blocks_do_not_claim_a_tool() {
        let blocks = engine_voxel::BlockRegistry::builtin();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let ore = blocks.def(blocks.id("core:gold_ore").unwrap());
        let tool = items.get(items.id("core:iron_pickaxe").unwrap()).tool;
        assert!(
            block(ore, None)
                .lines
                .iter()
                .any(|(s, _)| s.contains("insuffisant"))
        );
        assert!(
            !block(ore, tool.as_ref())
                .lines
                .iter()
                .any(|(s, _)| s.contains("insuffisant"))
        );
        assert_eq!(
            block(blocks.def(blocks.id("core:bedrock").unwrap()), None).lines[0].0,
            "Incassable"
        );
    }

    #[test]
    fn tooltips_near_the_bottom_right_remain_readable_inside_the_viewport() {
        let ctx = egui::Context::default();
        crate::skin::apply(&ctx);
        let card = Card {
            name: "Pioche en acier".into(),
            name_color: None,
            origin: "Forge".into(),
            lines: vec![("Durabilité : 697 / 700".into(), skin::TEXT)],
        };
        for [w, h] in [[320.0, 240.0], [640.0, 360.0], [1280.0, 720.0]] {
            let mut found = false;
            for frame in 0..12 {
                let pointer = egui::pos2(w - 30.0, h - 30.0);
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        time: Some((w as f64 + frame as f64) * 0.1),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(w, h),
                        )),
                        events: if frame == 0 {
                            vec![egui::Event::PointerMoved(pointer)]
                        } else {
                            vec![]
                        },
                        ..Default::default()
                    },
                    |ui| {
                        let rect = egui::Rect::from_min_size(
                            egui::pos2(w - 54.0, h - 54.0),
                            egui::vec2(44.0, 44.0),
                        );
                        let response = ui.allocate_rect(rect, egui::Sense::hover());
                        hover(response, &card);
                    },
                );
                for shape in &output.shapes {
                    if let egui::Shape::Text(text) = &shape.shape
                        && text.galley.text() == card.name
                    {
                        found = true;
                        let rect = text.visual_bounding_rect();
                        assert!(
                            rect.min.x >= 0.0
                                && rect.min.y >= 0.0
                                && rect.max.x <= w + 1.0
                                && rect.max.y <= h + 1.0,
                            "{w}x{h}: {rect:?}"
                        );
                    }
                }
                output.textures_delta.clear();
            }
            assert!(found, "l'infobulle doit effectivement apparaître à {w}x{h}");
        }
    }

    #[test]
    fn the_tooltip_lists_every_enchantment_of_an_item() {
        use crate::enchanting::{Enchantment, with_enchantment};
        let blocks = engine_voxel::BlockRegistry::builtin();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let def = items.get(items.id("core:iron_axe").unwrap());
        let damage = with_enchantment(
            with_enchantment(
                with_enchantment(2, Enchantment::EFFICIENCY, 3),
                Enchantment::SHARPNESS,
                2,
            ),
            Enchantment::UNBREAKING,
            1,
        );
        let card = item(def, 1, damage);
        let magic: Vec<&str> = card
            .lines
            .iter()
            .filter(|(_, c)| *c == skin::MAGIC)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(magic, ["Efficacité III", "Tranchant II", "Solidité I"]);
        // L'usure vient de la fiche.
        assert!(card.lines.iter().any(|(s, _)| s
            == &format!(
                "Durabilité : {} / {}",
                def.durability.unwrap() - 2,
                def.durability.unwrap()
            )));
        // Un objet simple n'a toujours qu'une ligne d'enchantement.
        let single = item(def, 1, with_enchantment(0, Enchantment::SHARPNESS, 1));
        assert_eq!(
            single
                .lines
                .iter()
                .filter(|(_, c)| *c == skin::MAGIC)
                .count(),
            1
        );
    }

    #[test]
    fn books_and_charms_have_their_own_tooltip_lines() {
        use crate::enchanting::{Enchantment, with_enchantment};
        let blocks = engine_voxel::BlockRegistry::builtin();
        let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
        let get = |key: &str| items.get(items.id(key).unwrap());
        // Un livre enchanté n'a pas de durabilité, mais il liste ses enchantements.
        let damage = with_enchantment(
            with_enchantment(0, Enchantment::FORTUNE, 4),
            Enchantment::SILK_TOUCH,
            1,
        );
        let card = item(get("core:enchanted_book"), 1, damage);
        let magic: Vec<&str> = card
            .lines
            .iter()
            .filter(|(_, c)| *c == skin::MAGIC)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(magic, ["Fortune IV", "Toucher de soie"]);
        assert!(
            card.lines
                .iter()
                .any(|(s, _)| s.contains("table d'enchantement"))
        );
        // Un tome et un livre vierge disent ce qu'ils font.
        for key in ["core:book", "core:tome_armor", "core:tome_extraction"] {
            let card = item(get(key), 1, 0);
            assert!(card.lines.len() >= 2, "{key}");
        }
        // Un charme dit son état et sa charge.
        let charm = get("core:charm_strength");
        let inactive = item(charm, 1, 0);
        assert!(inactive.lines.iter().any(|(s, _)| s.starts_with("Inactif")));
        let active = item(charm, 1, crate::charms::ACTIVE);
        assert!(active.lines.iter().any(|(s, _)| s.starts_with("Actif")));
        assert!(active.lines.iter().any(|(s, _)| s == "Charge : 100 %"));
        let empty = item(charm, 1, charm.durability.unwrap());
        assert!(empty.lines.iter().any(|(s, _)| s.starts_with("Vide")));
    }
}
