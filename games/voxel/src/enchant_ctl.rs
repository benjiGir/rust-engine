//! Les commandes `ctl` de l'enchantement avancé, pour les essais sans clavier.
//!
//! - `table x y z` : les stats de la table posée en x y z (Eterna, Quanta, Arcana...) en JSON ;
//! - `shelves x y z [étagère] [nombre]` : pose des étagères (`shelf_volcanic` par défaut ; `air` en
//!   enlève) sur les positions lues par la table en x y z (les 32, ou les `nombre` premières), et
//!   vide les cases d'air qui leur sont nécessaires ;
//! - `offers x y z` : les trois lots que cette table proposerait pour l'objet en main, en JSON ;
//! - `screen enchant x y z | anvil | library x y z` : ouvre l'écran (sans viser le bloc) ;
//! - `book sorte niveau [sorte niveau...]` : donne un livre enchanté (`tome armes|outils|armures`
//!   donne un tome enchanté de ce type : un livre dont les enchantements sont filtrés par type) ;
//! - `libadd x y z sorte niveau` : range directement un enchantement dans la bibliothèque en x y z ;
//! - `library x y z` : le contenu de la bibliothèque en JSON ;
//! - `points n` : fixe l'expérience du joueur à `n` points ;
//! - `charms` : les charmes de l'inventaire (actif, charge) en JSON ;
//! - `fortune`, `silk`, `magnet` (valeurs d'attributs) : voir `attributes`.

use anyhow::{Context as _, Result, bail};
use glam::IVec3;

use crate::VoxelGame;
use crate::enchant_books::{self, ENCHANTED_BOOK};
use crate::enchant_library::{self, Library};
use crate::enchant_table::{self, Subject};
use crate::enchanting::{self, Enchantment};
use crate::inventory::ItemStack;
use crate::screens::Screen;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Table(IVec3),
    Shelves(IVec3, String, Option<usize>),
    Offers(IVec3),
    Screen(String, Option<IVec3>),
    Book(Vec<(Enchantment, u8)>),
    LibAdd(IVec3, Enchantment, u8),
    Library(IVec3),
    Points(u32),
    Charms,
}

fn pos(args: &[&str], name: &str) -> Result<IVec3> {
    anyhow::ensure!(args.len() >= 3, "{name} : x y z");
    let n = |i: usize| -> Result<i32> {
        args[i]
            .parse()
            .with_context(|| format!("{name} : « {} » n'est pas un entier", args[i]))
    };
    Ok(IVec3::new(n(0)?, n(1)?, n(2)?))
}

fn kind(name: &str) -> Result<Enchantment> {
    Enchantment::from_key(name).with_context(|| format!("sorte d'enchantement inconnue « {name} »"))
}

fn level(kind: Enchantment, text: &str) -> Result<u8> {
    text.parse()
        .ok()
        .filter(|l| (1..=kind.hard_max()).contains(l))
        .with_context(|| format!("le niveau de {} va de 1 à {}", kind.name(), kind.hard_max()))
}

/// Lit ces commandes ; `None` si ce n'est pas l'une d'elles.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    Some(match name {
        "table" => pos(args, "table").map(Command::Table),
        "offers" => pos(args, "offers").map(Command::Offers),
        "shelves" => (|| {
            let at = pos(args, "shelves")?;
            let block = args.get(3).map_or("shelf_volcanic".to_string(), |b| {
                if b.contains(':') || *b == "air" {
                    (*b).to_string()
                } else {
                    format!("core:{b}")
                }
            });
            let count = args.get(4).map(|n| n.parse()).transpose()?;
            Ok(Command::Shelves(at, block, count))
        })(),
        "screen" => (|| match args {
            [name @ ("anvil" | "guide")] => Ok(Command::Screen((*name).into(), None)),
            [screen @ ("enchant" | "library"), rest @ ..] => Ok(Command::Screen(
                (*screen).into(),
                Some(pos(rest, "screen")?),
            )),
            _ => bail!("screen : enchant x y z | anvil | library x y z | guide"),
        })(),
        "book" => (|| {
            anyhow::ensure!(
                !args.is_empty() && args.len().is_multiple_of(2),
                "book : sorte niveau [sorte niveau...]"
            );
            args.chunks(2)
                .map(|pair| {
                    let k = kind(pair[0])?;
                    Ok((k, level(k, pair[1])?))
                })
                .collect::<Result<Vec<_>>>()
                .map(Command::Book)
        })(),
        "libadd" => (|| {
            anyhow::ensure!(args.len() == 5, "libadd : x y z sorte niveau");
            let k = kind(args[3])?;
            Ok(Command::LibAdd(pos(args, "libadd")?, k, level(k, args[4])?))
        })(),
        "library" => pos(args, "library").map(Command::Library),
        "points" => args
            .first()
            .and_then(|n| n.parse().ok())
            .map(Command::Points)
            .context("points : un nombre"),
        "charms" if args.is_empty() => Ok(Command::Charms),
        _ => return None,
    })
}

fn json_pos(p: IVec3) -> String {
    format!("[{},{},{}]", p.x, p.y, p.z)
}

impl VoxelGame {
    /// Exécute une commande d'enchantement avancé ; renvoie la réponse JSON.
    pub(crate) fn enchant_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Table(at) => {
                let s = self.server.table_stats(at);
                Ok(format!(
                    "{{\"ok\":true,\"eterna\":{:.2},\"raw_eterna\":{:.2},\"max_eterna\":{:.1},\"quanta\":{:.1},\"arcana\":{:.1},\"clues\":{},\"stability\":{:.1},\"treasure\":{},\"shelves\":{}}}",
                    s.eterna,
                    s.raw_eterna,
                    s.max_eterna,
                    s.quanta,
                    s.arcana,
                    s.clues,
                    s.stability,
                    s.treasure,
                    s.shelves
                ))
            }
            Command::Shelves(at, key, count) => {
                let block = if key == "air" {
                    engine_voxel::BlockId::AIR
                } else {
                    self.client
                        .registry
                        .id(&key)
                        .with_context(|| format!("shelves : bloc inconnu « {key} »"))?
                };
                let mut placed = 0;
                for (offset, middle) in enchant_table::shelf_positions()
                    .into_iter()
                    .take(count.unwrap_or(32))
                {
                    // L'air entre la table et l'étagère, puis l'étagère.
                    if !block.is_air() {
                        self.server.edit(at + middle, engine_voxel::BlockId::AIR);
                    }
                    self.server.edit(at + offset, block);
                    placed += 1;
                }
                Ok(format!("{{\"ok\":true,\"placed\":{placed}}}"))
            }
            Command::Offers(at) => {
                let stats = self.server.table_stats(at);
                let items = self.client.content.items.clone();
                let stack = self
                    .server
                    .players
                    .me_mut()
                    .inventory
                    .selected_stack()
                    .context("offers : rien en main")?;
                let def = items.get(stack.item);
                let subject = match enchant_books::BookKind::of_key(&def.key) {
                    Some(kind) => Subject::Book(kind),
                    None => Subject::Item(def),
                };
                let data = crate::adventure_data::current();
                let Some(lots) = enchant_table::offers(
                    &data,
                    subject,
                    &def.key,
                    stack.damage,
                    self.server.enchant_seed,
                    &stats,
                ) else {
                    return Ok("{\"ok\":true,\"offers\":[]}".to_string());
                };
                let lots: Vec<String> = lots
                    .iter()
                    .map(|o| {
                        format!(
                            "{{\"level\":{},\"points\":{},\"gems\":{},\"clues\":{},\"preview\":{},\"label\":{}}}",
                            o.level,
                            o.points,
                            o.gems,
                            o.clues,
                            crate::json_text::json_str(&o.preview()),
                            crate::json_text::json_str(&o.label())
                        )
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"offers\":[{}]}}", lots.join(",")))
            }
            Command::Screen(screen, at) => {
                let screen = match (screen.as_str(), at) {
                    ("enchant", Some(p)) => Screen::Enchant(p),
                    ("library", Some(p)) => Screen::Library(p),
                    ("guide", _) => Screen::Guide,
                    _ => Screen::Anvil,
                };
                // Par le serveur, comme un clic droit : ses gestes y seront acceptés.
                self.server.open_screen(screen);
                self.apply_replies();
                self.client.mouse_captured = false;
                Ok("{\"ok\":true}".to_string())
            }
            Command::Book(list) => {
                let book = self
                    .client
                    .content
                    .items
                    .id(ENCHANTED_BOOK)
                    .context("book : pas de livre enchanté")?;
                self.server.give_or_drop(ItemStack {
                    item: book,
                    count: 1,
                    damage: enchant_books::book_damage(&list),
                });
                Ok("{\"ok\":true}".to_string())
            }
            Command::LibAdd(at, kind, level) => {
                let max = {
                    let streamer = self.server.streamer();
                    enchant_library::max_level_of(
                        self.client
                            .registry
                            .def(streamer.world().block(at))
                            .enchanting
                            .as_ref(),
                    )
                };
                anyhow::ensure!(max > 0, "libadd : pas de bibliothèque en {}", json_pos(at));
                let mut streamer = self.server.streamer_mut();
                let entity = streamer
                    .block_entity_mut(at)
                    .context("libadd : la bibliothèque n'a pas d'état")?;
                let mut library = Library::read(&entity.data);
                library
                    .deposit(&[(kind, level)], max)
                    .map_err(|r| anyhow::anyhow!(r.text()))?;
                entity.data = library.write();
                Ok(format!(
                    "{{\"ok\":true,\"points\":{}}}",
                    library.points(kind)
                ))
            }
            Command::Library(at) => {
                let streamer = self.server.streamer();
                let entity = streamer
                    .block_entity(at)
                    .context("library : pas de bibliothèque en ce point")?;
                let library = Library::read(&entity.data);
                let rows: Vec<String> = library
                    .entries()
                    .iter()
                    .map(|(kind, points, best)| {
                        format!(
                            "{{\"enchantment\":{},\"points\":{points},\"best\":{best}}}",
                            crate::json_text::json_str(&kind.name())
                        )
                    })
                    .collect();
                Ok(format!("{{\"ok\":true,\"entries\":[{}]}}", rows.join(",")))
            }
            Command::Points(points) => {
                self.server.players.me_mut().experience.points = points;
                Ok(format!(
                    "{{\"ok\":true,\"points\":{},\"level\":{}}}",
                    points,
                    self.server.players.me_mut().experience.level().0
                ))
            }
            Command::Charms => {
                let items = self.client.content.items.clone();
                let data = crate::adventure_data::current();
                let rows: Vec<String> = self
                    .server
                    .players
                    .me_mut()
                    .inventory
                    .slots
                    .iter()
                    .flatten()
                    .filter_map(|s| {
                        let def = items.get(s.item);
                        data.charm_of(&def.key)?;
                        Some(format!(
                            "{{\"item\":{},\"active\":{},\"charge\":{:.3}}}",
                            crate::json_text::json_str(&def.key),
                            crate::charms::is_active(s.damage),
                            crate::charms::charge(s.damage, def.durability.unwrap_or(1))
                        ))
                    })
                    .collect();
                let _ = enchanting::WEAR_MASK;
                Ok(format!("{{\"ok\":true,\"charms\":[{}]}}", rows.join(",")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_parse_and_check_their_arguments() {
        assert_eq!(
            parse("table", &["1", "2", "3"]).unwrap().unwrap(),
            Command::Table(IVec3::new(1, 2, 3))
        );
        assert!(parse("table", &["1", "2"]).unwrap().is_err());
        assert_eq!(
            parse("book", &["sharpness", "5", "fortune", "3"])
                .unwrap()
                .unwrap(),
            Command::Book(vec![(Enchantment::SHARPNESS, 5), (Enchantment::FORTUNE, 3)])
        );
        // Le niveau dépasse le maximum de la sorte.
        assert!(parse("book", &["silk_touch", "2"]).unwrap().is_err());
        assert!(parse("book", &["inconnu", "1"]).unwrap().is_err());
        assert_eq!(
            parse("shelves", &["0", "5", "0", "jade", "8"])
                .unwrap()
                .unwrap(),
            Command::Shelves(IVec3::new(0, 5, 0), "core:jade".into(), Some(8))
        );
        assert!(parse("screen", &["anvil"]).unwrap().is_ok());
        assert!(parse("screen", &["library"]).unwrap().is_err());
        assert!(parse("points", &["x"]).unwrap().is_err());
        assert!(parse("autre", &[]).is_none());
    }
}
