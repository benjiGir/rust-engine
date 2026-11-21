//! Les commandes `ctl` de l'aventure, pour les essais sans clavier.
//!
//! - `loot [rareté] [objet]` : donne un objet à affixes (rareté : son identifiant ou son numéro,
//!   `epic`, `3`... ; objet : `iron_sword` par défaut) ;
//! - `gem sorte qualité [nombre]` : donne des gemmes (`ruby 3`, qualité de 1 à 5) ;
//! - `tier n` : règle le palier (et débloque jusqu'à lui) ; `tiers` : l'état de la progression ;
//! - `progress kills n | bosses n | rarity n | depth n` : règle la progression (pour tester les
//!   déblocages) ;
//! - `elite [créature]` : fait apparaître une élite devant soi ; `invader` : un envahisseur comme celui
//!   du minuteur (sur l'anneau autour du joueur, annoncé par un son et un message) ;
//!   `invader créature` : le même, devant soi, de cette créature ; `elites` : la liste des élites vivantes ;
//! - `slay [all]` : tue les élites (ou toutes les hostiles) : leur butin tombe comme à la mort ;
//! - `cursor n | clear` : met la pile de la case `n` à la souris (ou la range) ;
//! - `open tiers|salvage|reforge` : ouvre l'écran sans le bloc ;
//! - `pending x y z objet [rareté_min]` : range dans le coffre en x y z un objet « à tirer » (comme ceux
//!   des coffres de structure) ; il est tiré à la première ouverture du coffre ;
//! - `clearinv` : vide l'inventaire (la barre et la réserve), pour refaire de la place aux essais ;
//! - `sockets n` : donne `n` emplacements vides à l'objet en main (sans sceau).

use anyhow::{Context as _, Result, bail};
use glam::Vec3;

use crate::VoxelGame;
use crate::adventure_data;
use crate::affixes;
use crate::inventory::ItemStack;
use crate::ui::Screen;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Loot(String, String),
    Gem(String, usize, u32),
    Tier(u8),
    Tiers,
    Progress(String, i64),
    Elite(Option<String>),
    Invader(Option<String>),
    Elites,
    Slay(bool),
    Cursor(Option<usize>),
    Open(String),
    Sockets(usize),
    ClearInventory,
    Pending(glam::IVec3, String, u8),
}

fn qualified(key: &str) -> String {
    if key.contains(':') {
        key.to_string()
    } else {
        format!("core:{key}")
    }
}

/// Lit les commandes de l'aventure ; `None` si ce n'est pas l'une d'elles.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    Some(match name {
        "loot" => Ok(Command::Loot(
            args.first().copied().unwrap_or("common").to_string(),
            qualified(args.get(1).copied().unwrap_or("iron_sword")),
        )),
        "gem" => (|| {
            let kind = args.first().context("gem : sorte qualité [nombre]")?;
            let quality: usize = args
                .get(1)
                .and_then(|q| q.parse().ok())
                .filter(|q| (1..=5).contains(q))
                .context("gem : la qualité va de 1 à 5")?;
            let count = args.get(2).map_or(Ok(1), |n| n.parse())?;
            Ok(Command::Gem(qualified(kind), quality, count))
        })(),
        "tier" => args
            .first()
            .and_then(|n| n.parse().ok())
            .map(Command::Tier)
            .context("tier : un numéro de palier (0 à 4)"),
        "tiers" => Ok(Command::Tiers),
        "clearinv" => Ok(Command::ClearInventory),
        "pending" => (|| {
            anyhow::ensure!(args.len() >= 4, "pending : x y z objet [rareté_min]");
            let n = |i: usize| -> Result<i32> { Ok(args[i].parse()?) };
            let min = args.get(4).map_or(Ok(0), |m| m.parse())?;
            Ok(Command::Pending(
                glam::IVec3::new(n(0)?, n(1)?, n(2)?),
                qualified(args[3]),
                min,
            ))
        })(),
        "progress" => (|| {
            if args.len() != 2 {
                bail!("progress : kills|bosses|rarity|depth nombre");
            }
            Ok(Command::Progress(args[0].to_string(), args[1].parse()?))
        })(),
        "elite" => Ok(Command::Elite(args.first().map(|a| qualified(a)))),
        "invader" => Ok(Command::Invader(args.first().map(|a| qualified(a)))),
        "elites" => Ok(Command::Elites),
        "slay" => Ok(Command::Slay(args.first() == Some(&"all"))),
        "cursor" => match args {
            ["clear"] => Ok(Command::Cursor(None)),
            [n] => n
                .parse::<usize>()
                .ok()
                .filter(|n| (1..=36).contains(n))
                .map(|n| Command::Cursor(Some(n - 1)))
                .context("cursor : une case de 1 à 36, ou clear"),
            _ => Err(anyhow::anyhow!("cursor : n | clear")),
        },
        "open" => match args {
            [screen @ ("tiers" | "salvage" | "reforge")] => Ok(Command::Open((*screen).into())),
            _ => Err(anyhow::anyhow!("open : tiers | salvage | reforge")),
        },
        "sockets" => args
            .first()
            .and_then(|n| n.parse().ok())
            .map(Command::Sockets)
            .context("sockets : un nombre"),
        _ => return None,
    })
}

impl VoxelGame {
    pub(crate) fn adventure_command(&mut self, command: Command) -> Result<String> {
        let data = adventure_data::current();
        match command {
            Command::Loot(rarity, key) => {
                let index = data
                    .rarity_index(&rarity)
                    .or_else(|| rarity.parse().ok())
                    .filter(|r| usize::from(*r) < data.rarities.len())
                    .with_context(|| format!("loot : rareté inconnue « {rarity} »"))?;
                let items = self.client.content.items.clone();
                let item = items
                    .id(&key)
                    .with_context(|| format!("loot : objet inconnu « {key} »"))?;
                let mut rng = engine_voxel::Rng64::new(self.server.next_random());
                let def = items.get(item);
                anyhow::ensure!(
                    affixes::can_carry(def),
                    "loot : {key} ne peut pas porter d'affixe"
                );
                let sheet = affixes::roll_sheet(&data, def, index, &mut rng);
                let mut damage = 0;
                anyhow::ensure!(
                    affixes::apply(&mut damage, sheet),
                    "loot : plus de numéro de fiche"
                );
                let left = self.server.players.me_mut().inventory.add_stack(
                    ItemStack {
                        item,
                        count: 1,
                        damage,
                    },
                    &items,
                );
                Ok(format!("{{\"ok\":true,\"full\":{left}}}"))
            }
            Command::Gem(key, quality, count) => {
                let kind = data
                    .gems
                    .iter()
                    .find(|g| g.id == key)
                    .with_context(|| format!("gem : sorte inconnue « {key} »"))?;
                let item = self
                    .client
                    .content
                    .items
                    .id(&kind.item_key(quality - 1))
                    .context("gem : objet absent")?;
                let left = self.server.players.me_mut().inventory.add(
                    item,
                    count,
                    &self.client.content.items,
                );
                Ok(format!("{{\"ok\":true,\"full\":{left}}}"))
            }
            Command::Tier(tier) => {
                anyhow::ensure!(
                    usize::from(tier) < data.tiers.len(),
                    "tier : 0 à {}",
                    data.tiers.len() - 1
                );
                let progress = &mut self.server.players.me_mut().adventure.progress;
                progress.unlocked = progress.unlocked.max(tier);
                progress.select(tier, &data.tiers);
                Ok(self.adventure_json())
            }
            Command::Tiers => Ok(self.adventure_json()),
            Command::Progress(what, n) => {
                let p = &mut self.server.players.me_mut().adventure.progress;
                match what.as_str() {
                    "kills" => p.kills = n.max(0) as u32,
                    "bosses" => p.bosses = n.max(0) as u32,
                    "rarity" => p.best_rarity = (n >= 0).then_some(n as u8),
                    "depth" => p.depth = n as i32,
                    other => bail!("progress : champ inconnu « {other} »"),
                }
                self.server.unlock_tiers(&data);
                Ok(self.adventure_json())
            }
            Command::Elite(kind) => self.spawn_test_elite(kind, false),
            Command::Invader(Some(kind)) => self.spawn_test_elite(Some(kind), true),
            Command::Invader(None) => {
                let site = self
                    .server
                    .spawn_invader(&data)
                    .context("invader : pas de terrain libre sur l'anneau, réessayer")?;
                Ok(format!(
                    "{{\"ok\":true,\"pos\":[{:.1},{:.1},{:.1}]}}",
                    site.x, site.y, site.z
                ))
            }
            Command::Elites => {
                let mut rows = Vec::new();
                self.server.ecs.query::<(
                    &crate::elites::Elite,
                    &crate::components::Health,
                    &crate::components::Transform,
                )>().for_each(|_, (elite, health, transform)| {
                    rows.push(format!(
                        "{{\"name\":{},\"invader\":{},\"health\":{:.1},\"max\":{:.1},\"damage\":{:.2},\"speed\":{:.2},\"armor\":{:.1},\"pos\":[{:.1},{:.1},{:.1}]}}",
                        crate::control::json_str(&elite.name),
                        elite.invader,
                        health.current,
                        health.max,
                        elite.damage,
                        elite.speed,
                        elite.armor,
                        transform.position.x,
                        transform.position.y,
                        transform.position.z
                    ));
                });
                Ok(format!("{{\"ok\":true,\"elites\":[{}]}}", rows.join(",")))
            }
            Command::Slay(all) => {
                let registry = self
                    .server
                    .ecs
                    .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
                    .clone();
                let mut count = 0;
                self.server
                    .ecs
                    .query::<(&crate::components::Mob, &mut crate::components::Health)>()
                    .for_each(|entity, (mob, health)| {
                        let hostile = registry.get(mob.kind).is_some_and(|d| d.hostile);
                        let elite = self.server.ecs.has::<crate::elites::Elite>(entity);
                        if (all && hostile) || elite {
                            health.current = 0.0;
                            count += 1;
                        }
                    });
                Ok(format!("{{\"ok\":true,\"slain\":{count}}}"))
            }
            Command::Cursor(slot) => {
                self.server.players.me_mut().inventory.cursor = match slot {
                    Some(i) => self.server.players.me_mut().inventory.slots[i].take(),
                    None => None,
                };
                Ok("{\"ok\":true}".to_string())
            }
            Command::Open(screen) => {
                let screen = match screen.as_str() {
                    "salvage" => Screen::Salvage,
                    "reforge" => Screen::Reforge,
                    _ => Screen::Tiers,
                };
                self.server.open_screen(screen);
                self.apply_replies();
                self.client.mouse_captured = false;
                Ok("{\"ok\":true}".to_string())
            }
            Command::Pending(pos, key, min) => {
                let block = self.server.streamer().world().block(pos);
                let slots = engine_voxel::container_slots(&self.client.registry, block)
                    .context("pending : pas un conteneur")?;
                let seed = (self.server.next_random() & 0xFFFF) as u16;
                let mut streamer = self.server.streamer_mut();
                let mut entity = streamer.block_entity(pos).cloned().unwrap_or_default();
                let inventory = entity
                    .caps_mut()
                    .inventory
                    .get_or_insert_with(|| engine_voxel::Inventory::new(slots));
                let free = inventory
                    .slots
                    .iter()
                    .position(Option::is_none)
                    .context("pending : conteneur plein")?;
                inventory.slots[free] = Some(engine_voxel::ItemStack {
                    item: key,
                    count: 1,
                    damage: engine_voxel::pending_loot(min, seed),
                });
                streamer.set_block_entity(pos, Some(entity));
                Ok("{\"ok\":true}".to_string())
            }
            Command::ClearInventory => {
                self.server.players.me_mut().inventory.slots = [None; crate::inventory::SLOTS];
                self.server.players.me_mut().inventory.cursor = None;
                Ok("{\"ok\":true}".to_string())
            }
            Command::Sockets(n) => {
                let selected = self.server.players.me_mut().inventory.selected;
                let stack = self.server.players.me_mut().inventory.slots[selected]
                    .as_mut()
                    .context("sockets : rien en main")?;
                crate::enchanting::edit_sheet(&mut stack.damage, |s| {
                    s.sockets.extend(std::iter::repeat_n(None, n));
                });
                Ok("{\"ok\":true}".to_string())
            }
        }
    }

    /// Une élite (ou un envahisseur) devant le joueur.
    fn spawn_test_elite(&mut self, kind: Option<String>, invader: bool) -> Result<String> {
        let data = adventure_data::current();
        let registry = self
            .server
            .ecs
            .resource::<std::sync::Arc<crate::mobs::MobRegistry>>()
            .clone();
        let key = kind.unwrap_or_else(|| "core:prowler".into());
        let id = registry
            .id(&key)
            .with_context(|| format!("créature inconnue « {key} »"))?;
        let feet = self.client.body.body.position;
        let distance = if invader { 18.0 } else { 5.0 };
        let mut at: Vec3 = feet + self.client.camera.forward_flat() * distance;
        // Au sol : le premier bloc plein sous le point, vers le bas.
        {
            let shared = crate::shared::StreamerExt::streamer(&self.server.ecs);
            let streamer = shared.borrow();
            let column = streamer
                .generator()
                .column(at.x.floor() as i32, at.z.floor() as i32);
            at.y = column.height as f32 + 1.0;
        }
        let entity = crate::mobs::spawn_mob(&mut self.server.ecs, id, at).context("impossible")?;
        let promoted = if invader {
            self.server.promote_invader(entity, &data)
        } else {
            let tier = self.server.players.me_mut().adventure.progress.tier;
            let mut rng = engine_voxel::Rng64::new(self.server.next_random());
            crate::elites::promote(&mut self.server.ecs, entity, &data, tier, false, &mut rng)
        };
        anyhow::ensure!(promoted, "{key} ne peut pas devenir élite");
        if invader && let Some(audio) = &mut self.client.audio {
            audio.play("core:cave_rumble");
            audio.play("core:enchant");
        }
        let name = crate::elites::of(&self.server.ecs, entity).map_or_else(String::new, |e| e.name);
        Ok(format!(
            "{{\"ok\":true,\"name\":{},\"pos\":[{:.1},{:.1},{:.1}]}}",
            crate::control::json_str(&name),
            at.x,
            at.y,
            at.z
        ))
    }

    /// La progression et le palier, en JSON.
    pub(crate) fn adventure_json(&self) -> String {
        let data = adventure_data::current();
        let p = &self.server.players.me().adventure.progress;
        let effects = self.server.players.me().adventure.effects(&data);
        format!(
            "{{\"ok\":true,\"tier\":{},\"unlocked\":{},\"kills\":{},\"bosses\":{},\"best_rarity\":{},\"depth\":{},\"health\":{:.2},\"damage\":{:.2},\"elite_chance\":{:.2}}}",
            p.tier,
            p.unlocked,
            p.kills,
            p.bosses,
            p.best_rarity.map_or("null".to_string(), |r| r.to_string()),
            p.depth,
            effects.health,
            effects.damage,
            effects.elite_chance
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adventure_commands_are_read() {
        assert_eq!(
            parse("loot", &["epic", "iron_axe"]).unwrap().unwrap(),
            Command::Loot("epic".into(), "core:iron_axe".into())
        );
        assert_eq!(
            parse("gem", &["ruby", "3", "2"]).unwrap().unwrap(),
            Command::Gem("core:ruby".into(), 3, 2)
        );
        assert!(parse("gem", &["ruby", "9"]).unwrap().is_err());
        assert_eq!(parse("tier", &["3"]).unwrap().unwrap(), Command::Tier(3));
        assert_eq!(
            parse("cursor", &["clear"]).unwrap().unwrap(),
            Command::Cursor(None)
        );
        assert_eq!(
            parse("cursor", &["4"]).unwrap().unwrap(),
            Command::Cursor(Some(3))
        );
        assert!(parse("open", &["bank"]).unwrap().is_err());
        assert_eq!(
            parse("slay", &["all"]).unwrap().unwrap(),
            Command::Slay(true)
        );
        assert!(parse("autre", &[]).is_none());
    }
}
