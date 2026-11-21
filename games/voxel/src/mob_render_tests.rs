//! Les tests des créatures qui touchent au dessin (boîtes, icônes, peaux) : ils vivent côté fenêtre,
//! avec les mondes de test de la simulation (`voxel_sim::mobs::tests`, fonction `testing`).

use std::sync::Arc;

use engine_ecs::Events;
use glam::Vec3;

use crate::components::*;
use crate::events::EntityDied;
use crate::mobs::tests::{repo, sim};
use crate::mobs::*;

#[test]
fn sheep_colours_tint_only_fleece_and_survive_despawning_in_the_death_event() {
    use crate::sheep_colors::{SheepCoat, drop_key, palette};
    let mut world = sim();
    let registry = world.resource::<Arc<MobRegistry>>().clone();
    let kind = registry.id("core:sheep").unwrap();
    let mob = spawn_mob_with_coat(
        &mut world,
        kind,
        Vec3::new(5.5, 80.0, 5.5),
        Some(SheepCoat(0)),
    )
    .unwrap();
    let plain = crate::entity_render::support::boxes(&mut world, 1.0);
    world.insert(mob, SheepCoat(3));
    let dyed = crate::entity_render::support::boxes(&mut world, 1.0);
    for ((part, a), b) in registry
        .get(kind)
        .unwrap()
        .parts
        .iter()
        .zip(&plain)
        .zip(&dyed)
    {
        assert_eq!(a.center, b.center);
        assert_eq!(a.size, b.size);
        assert_eq!(a.textures, b.textures);
        if part.texture.as_deref() == Some("core:mob_sheep_wool") {
            assert_ne!(a.color, b.color);
        } else {
            assert_eq!(
                a.color, b.color,
                "peau, yeux et sabots gardent leur matière"
            );
        }
    }
    world.commands().despawn(mob);
    world.apply_commands();
    let items = engine_voxel::ItemRegistry::builtin(&crate::test_registry()).unwrap();
    let mut reader = world.resource::<Events<EntityDied>>().reader();
    for (i, color) in palette().iter().enumerate() {
        let coat = SheepCoat(i as u8);
        let e =
            spawn_mob_with_coat(&mut world, kind, Vec3::new(5.5, 80.0, 5.5), Some(coat)).unwrap();
        world.get_mut::<Health>(e).unwrap().current = 0.0;
        cleanup(&mut world);
        world.apply_commands();
        assert!(!world.is_alive(e));
        let died: Vec<_> = reader
            .read(&world.resource::<Events<EntityDied>>())
            .cloned()
            .collect();
        assert_eq!(died.len(), 1);
        assert_eq!(died[0].coat, Some(coat));
        assert_eq!(drop_key("core:wool", died[0].coat), color.item);
        assert!(items.id(drop_key("core:wool", died[0].coat)).is_some());
    }
}

#[test]
fn every_wool_colour_has_native_icons_and_can_make_a_bed_even_when_mixed() {
    use crate::sheep_colors::palette;
    let blocks = crate::test_registry();
    let items = engine_voxel::ItemRegistry::builtin(&blocks).unwrap();
    let recipes = engine_voxel::Recipes::builtin(&items).unwrap();
    let plank = items.id("core:oak_planks");
    for (i, color) in palette().iter().enumerate() {
        let wool = items.id(&color.item);
        assert!(wool.is_some());
        let other = items.id(&palette()[(i + 1) % palette().len()].item);
        let grid = [wool, other, wool, plank, plank, plank, None, None, None];
        assert_eq!(
            recipes.find(&grid, 3).unwrap().result(),
            (items.id("core:bed").unwrap(), 1)
        );
        for packed in [false, true] {
            let mut registry = blocks.clone();
            if packed {
                registry
                    .set_visual_pack(
                        Some(repo().join("texture_packs/earth-copper-32/assets")),
                        32,
                    )
                    .unwrap();
            }
            let pixels =
                crate::assets::load_png(&registry.asset_path(&color.item, "items", "png").unwrap())
                    .unwrap();
            assert_eq!(pixels.len(), if packed { 32 * 32 * 4 } else { 16 * 16 * 4 });
            let opaque: Vec<_> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[3] > 0)
                .collect();
            assert!(!opaque.is_empty());
            for c in 0..3 {
                let mean = opaque
                    .iter()
                    .map(|p| {
                        let s = p[c] as f32 / 255.0;
                        if s <= 0.04045 {
                            s / 12.92
                        } else {
                            ((s + 0.055) / 1.055).powf(2.4)
                        }
                    })
                    .sum::<f32>()
                    / opaque.len() as f32;
                assert!(
                    (mean - items.get(wool.unwrap()).color[c]).abs()
                        < if packed { 0.02 } else { 0.00001 }
                );
            }
        }
    }
}

#[test]
fn farm_loot_has_icons_food_and_cooking_and_wool_can_make_a_bed() {
    use engine_voxel::{ItemRegistry, Recipes, Smelting};
    let blocks = crate::test_registry();
    let items = ItemRegistry::builtin(&blocks).unwrap();
    let creatures = MobRegistry::load(&repo().join("assets"), &[], &blocks).unwrap();
    for key in ["core:sheep", "core:chicken", "core:fish"] {
        for drop in &creatures.by_key(key).unwrap().drops {
            assert!(
                items.id(&drop.item).is_some(),
                "butin inconnu : {}",
                drop.item
            );
        }
    }
    let cooking = Smelting::builtin(&items).unwrap();
    for name in ["mutton", "chicken", "fish"] {
        let raw = items.id(&format!("core:raw_{name}")).unwrap();
        let cooked = items.id(&format!("core:cooked_{name}")).unwrap();
        assert_eq!(cooking.recipe(raw).unwrap().output, cooked);
        assert!(items.get(cooked).food.unwrap().nutrition > items.get(raw).food.unwrap().nutrition);
        for packed in [false, true] {
            let base = if packed {
                repo().join("texture_packs/earth-copper-32/assets/core/items")
            } else {
                repo().join("assets/core/items")
            };
            for prefix in ["raw", "cooked"] {
                let image =
                    crate::assets::load_png(&base.join(format!("{prefix}_{name}.png"))).unwrap();
                assert_eq!(image.len(), if packed { 32 * 32 * 4 } else { 16 * 16 * 4 });
                assert!(image.as_chunks::<4>().0.iter().any(|p| p[3] == 255));
                assert!(image.as_chunks::<4>().0.iter().any(|p| p[3] == 0));
            }
        }
    }
    let wool = items.id("core:wool");
    let plank = items.id("core:oak_planks");
    let recipes = Recipes::builtin(&items).unwrap();
    let grid = [wool, wool, wool, plank, plank, plank, None, None, None];
    assert_eq!(
        recipes.find(&grid, 3).unwrap().result(),
        (items.id("core:bed").unwrap(), 1)
    );
}

#[test]
fn creature_skins_use_both_packs_without_changing_block_ids_and_keep_fallback_colours() {
    let root = repo();
    let mods = [
        ("example".into(), root.join("mods/example")),
        ("donjons".into(), root.join("mods/donjons")),
    ];
    for packed in [false, true] {
        let mut blocks = crate::test_registry();
        let ids: Vec<_> = blocks
            .iter()
            .map(|(id, def)| (id, def.key.clone()))
            .collect();
        let original_layers = blocks.textures().to_vec();
        if packed {
            blocks
                .set_visual_pack(Some(root.join("texture_packs/earth-copper-32/assets")), 32)
                .unwrap();
        }
        let mut creatures = MobRegistry::load(&root.join("assets"), &mods, &blocks).unwrap();
        creatures.bind_textures(&mut blocks);
        let keys = blocks.textures().to_vec();
        creatures.bind_textures(&mut blocks);
        assert_eq!(
            keys,
            blocks.textures(),
            "le rechargement réutilise les couches"
        );
        assert_eq!(
            ids,
            blocks
                .iter()
                .map(|(id, def)| (id, def.key.clone()))
                .collect::<Vec<_>>()
        );
        assert_eq!(&blocks.textures()[..original_layers.len()], original_layers);
        let textures = crate::assets::load_textures(&blocks);
        for key in [
            "core:pig",
            "core:cow",
            "core:sheep",
            "core:chicken",
            "core:fish",
            "core:prowler",
            "core:spore_spitter",
            "example:slime",
            "example:guard",
            "donjons:warden",
        ] {
            let def = creatures.by_key(key).unwrap();
            assert!(def.parts.iter().any(|p| p.texture.is_some()));
            for part in def.parts.iter().filter(|p| p.texture.is_some()) {
                assert!(part.texture_layer as usize > original_layers.len());
                let image = &textures[part.texture_layer as usize - 1];
                let side = if packed { 32 } else { 16 };
                assert_eq!(image.len(), side * side * 4);
                let fallback = [part.color.0, part.color.1, part.color.2];
                for channel in 0..3 {
                    let mean = image
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|p| {
                            let c = p[channel] as f32 / 255.0;
                            if c <= 0.04045 {
                                c / 12.92
                            } else {
                                ((c + 0.055) / 1.055).powf(2.4)
                            }
                        })
                        .sum::<f32>()
                        / (side * side) as f32;
                    assert!(
                        (mean - fallback[channel]).abs() < if packed { 0.02 } else { 0.00001 },
                        "{key} : moyenne de repli"
                    );
                }
            }
        }
    }
}

#[test]
fn legs_swing_with_the_walk_and_rest_when_still() {
    let mut world = sim();
    let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
    let mob = spawn_mob(&mut world, pig, Vec3::new(0.5, 150.0, 0.5)).unwrap();
    world.get_mut::<Velocity>(mob).unwrap().0 = Vec3::new(1.6, 0.0, 0.0);
    for _ in 0..60 {
        animate(&mut world);
    }
    let walking = *world.get::<MobAnim>(mob).unwrap();
    assert!(walking.stride > 0.8, "il marche : {}", walking.stride);
    // Les pattes ont un balancement, en diagonale (deux de chaque signe).
    let boxes = crate::entity_render::support::boxes(&mut world, 1.0);
    let swings: Vec<f32> = boxes
        .iter()
        .map(|b| b.swing[0])
        .filter(|a| *a != 0.0)
        .collect();
    assert!(
        swings.len() >= 8,
        "pattes et sabots balancent : {}",
        swings.len()
    );
    assert!(swings.iter().any(|a| *a > 0.05) && swings.iter().any(|a| *a < -0.05));
    // À l'arrêt, les pas s'éteignent et la phase ne bouge plus.
    world.get_mut::<Velocity>(mob).unwrap().0 = Vec3::ZERO;
    for _ in 0..120 {
        animate(&mut world);
    }
    let still = *world.get::<MobAnim>(mob).unwrap();
    assert!(still.stride < 0.02, "{}", still.stride);
    let phase = still.phase;
    animate(&mut world);
    assert_eq!(world.get::<MobAnim>(mob).unwrap().phase, phase);
}

#[test]
fn a_creature_turns_its_head_toward_a_player_nearby_and_looks_away_far_off() {
    let mut world = sim();
    let player = world.resource::<PlayerEntities>().0[0];
    let prowler = world
        .resource::<Arc<MobRegistry>>()
        .id("core:prowler")
        .unwrap();
    let mob = spawn_mob(&mut world, prowler, Vec3::new(0.5, 150.0, 0.5)).unwrap();
    world.get_mut::<Transform>(mob).unwrap().yaw = 0.0;
    // Le joueur à 5 blocs, à 45° sur sa gauche (côté +z) : une hostile le suit des yeux.
    world.get_mut::<Transform>(player).unwrap().position = Vec3::new(4.0, 150.0, 4.5);
    for _ in 0..240 {
        animate(&mut world);
    }
    let look = world.get::<MobAnim>(mob).unwrap().look;
    assert!((look - 0.78).abs() < 0.15, "tête tournée de {look}");
    // La tête pivote avec ses boîtes : leur lacet change, pas celui du corps.
    let boxes = crate::entity_render::support::boxes(&mut world, 1.0);
    assert!(
        boxes.iter().any(|b| (b.yaw - look).abs() < 0.01),
        "une boîte de tête tournée"
    );
    assert!(boxes.iter().any(|b| b.yaw == 0.0), "le corps reste droit");
    // Le joueur loin (60 blocs) : elle ne le regarde plus, la tête revient près du repos.
    world.get_mut::<Transform>(player).unwrap().position = Vec3::new(60.0, 150.0, 4.5);
    for _ in 0..600 {
        animate(&mut world);
    }
    assert!(world.get::<MobAnim>(mob).unwrap().look.abs() < 0.5);
}
