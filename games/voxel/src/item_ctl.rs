//! Les commandes `ctl` des objets enrichis et des attributs, pour les essais sans clavier.
//!
//! - `enchant sorte niveau [sorte niveau...]` : ajoute ces enchantements à l'objet en main
//!   (`efficiency`, `sharpness`, `unbreaking`, `protection`), comme la table, donc avec la
//!   migration vers une fiche au deuxième ;
//! - `affix attribut fixe [part]` : ajoute un affixe d'essai à l'objet en main (`damage`,
//!   `crit_chance`, `life_steal`, `armor_pierce`, `reach`... : voir `Attribute::key`) ;
//! - `hover x y` : le pointeur se pose en x y (points egui, comme `click`) : la bulle d'aide
//!   de la case s'affiche ;
//! - `equip n` : passe la pièce d'armure de la case `n` sur le corps ;
//! - `sheet` : la pile en main (son `damage`, sa fiche) en JSON ;
//! - `attributes` : les attributs du joueur en JSON ; `attributes open|close` ouvre (avec
//!   l'inventaire) ou ferme la fiche, `attributes modified on|off` règle son filtre ;
//! - `throw` : jette un exemplaire de l'objet en main (la touche Q).

use anyhow::{Context as _, Result, bail};

use crate::VoxelGame;
use crate::attributes::Attribute;
use crate::enchanting::{self, Enchantment};
use crate::item_data;

/// Une commande des objets, lue par `control.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Enchant(Vec<(Enchantment, u8)>),
    Sheet,
    Attributes(Option<Panel>),
    Throw,
    /// Le pointeur se pose en x y (une bulle d'aide s'affiche).
    Hover(f32, f32),
    /// Passe la pièce d'armure de cette case (0 à 35) sur le corps.
    Equip(usize),
    /// Un affixe d'essai sur l'objet en main : attribut, bonus fixe, part.
    Affix(Attribute, f32, f32),
}

/// Ce que `attributes` fait de la fiche de l'inventaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Open,
    Close,
    Modified(bool),
}

fn enchantment(name: &str) -> Result<Enchantment> {
    Ok(match name {
        "efficiency" | "efficacite" | "efficacité" => Enchantment::Efficiency,
        "sharpness" | "tranchant" => Enchantment::Sharpness,
        "unbreaking" | "solidite" | "solidité" => Enchantment::Unbreaking,
        "protection" => Enchantment::Protection,
        other => bail!("enchant : sorte inconnue « {other} »"),
    })
}

fn attribute(name: &str) -> Result<Attribute> {
    Attribute::ALL
        .into_iter()
        .find(|a| a.key() == name)
        .with_context(|| format!("affix : attribut inconnu « {name} »"))
}

/// Lit `enchant`, `sheet`, `attributes`, `throw` ; `None` si ce n'est pas une commande d'objet.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    Some(match name {
        "enchant" => (|| {
            if args.is_empty() || !args.len().is_multiple_of(2) {
                bail!("enchant : sorte niveau [sorte niveau...]");
            }
            args.chunks(2)
                .map(|pair| {
                    let level: u8 = pair[1]
                        .parse()
                        .ok()
                        .filter(|l| (1..=3).contains(l))
                        .context("enchant : le niveau va de 1 à 3")?;
                    Ok((enchantment(pair[0])?, level))
                })
                .collect::<Result<Vec<_>>>()
                .map(Command::Enchant)
        })(),
        "affix" => (|| {
            if !(2..=3).contains(&args.len()) {
                bail!("affix : attribut bonus_fixe [part]");
            }
            let number = |text: &str| {
                text.parse::<f32>()
                    .with_context(|| format!("affix : « {text} » n'est pas un nombre"))
            };
            let percent = args.get(2).map_or(Ok(0.0), |p| number(p))?;
            Ok(Command::Affix(
                attribute(args[0])?,
                number(args[1])?,
                percent,
            ))
        })(),
        "hover" => (|| {
            let n = |i: usize| -> Result<f32> {
                args.get(i)
                    .context("hover : x y")?
                    .parse()
                    .with_context(|| format!("hover : « {} » n'est pas un nombre", args[i]))
            };
            Ok(Command::Hover(n(0)?, n(1)?))
        })(),
        "equip" => args
            .first()
            .and_then(|a| a.parse::<usize>().ok())
            .filter(|n| args.len() == 1 && (1..=36).contains(n))
            .map(|n| Command::Equip(n - 1))
            .context("equip : une case de 1 à 36"),
        "sheet" if args.is_empty() => Ok(Command::Sheet),
        "throw" if args.is_empty() => Ok(Command::Throw),
        "attributes" => match args {
            [] => Ok(Command::Attributes(None)),
            ["open"] => Ok(Command::Attributes(Some(Panel::Open))),
            ["close"] => Ok(Command::Attributes(Some(Panel::Close))),
            ["modified", "on"] => Ok(Command::Attributes(Some(Panel::Modified(true)))),
            ["modified", "off"] => Ok(Command::Attributes(Some(Panel::Modified(false)))),
            _ => Err(anyhow::anyhow!(
                "attributes : [open | close | modified on|off]"
            )),
        },
        _ => return None,
    })
}

impl VoxelGame {
    /// Exécute une commande d'objet ; renvoie la réponse JSON.
    pub(crate) fn item_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Enchant(list) => {
                let selected = self.inventory.selected;
                let items = self.content.items.clone();
                let stack = self.inventory.slots[selected]
                    .as_mut()
                    .context("enchant : rien en main")?;
                let def = items.get(stack.item);
                for (kind, level) in list {
                    if !kind.applies_to(def) {
                        bail!("enchant : {} ne va pas sur {}", kind.name(), def.name);
                    }
                    stack.damage = enchanting::with_enchantment(stack.damage, kind, level);
                }
                Ok(self.sheet_json())
            }
            Command::Hover(x, y) => {
                self.ui.input.hover(engine_render::egui::pos2(x, y), 30);
                Ok("{\"ok\":true}".to_string())
            }
            Command::Equip(slot) => {
                let items = self.content.items.clone();
                let stack = self.inventory.slots[slot].context("equip : case vide")?;
                let index = items
                    .get(stack.item)
                    .armor
                    .and_then(|a| {
                        engine_voxel::ArmorSlot::ALL
                            .iter()
                            .position(|s| *s == a.slot)
                    })
                    .context("equip : ce n'est pas une pièce d'armure")?;
                // La pièce portée (s'il y en a une) prend la place de la nouvelle.
                self.inventory.slots[slot] = self.inventory.armor[index].replace(stack);
                Ok(format!(
                    "{{\"ok\":true,\"armor\":{:.1}}}",
                    self.inventory.protection(&items)
                ))
            }
            Command::Affix(attribute, flat, percent) => {
                let selected = self.inventory.selected;
                let stack = self.inventory.slots[selected]
                    .as_mut()
                    .context("affix : rien en main")?;
                let affix = item_data::Affix {
                    key: "essai".into(),
                    attribute: attribute.id(),
                    flat,
                    percent,
                };
                if !enchanting::edit_sheet(&mut stack.damage, |sheet| sheet.affixes.push(affix)) {
                    bail!("affix : plus de numéro de fiche libre");
                }
                Ok(self.sheet_json())
            }
            Command::Sheet => Ok(self.sheet_json()),
            Command::Attributes(panel) => {
                match panel {
                    Some(Panel::Open) => {
                        // On ouvre aussi l'inventaire, où est la fiche.
                        self.ui.attributes.open = true;
                        if !self.ui.inventory_open {
                            self.ui.inventory_open = true;
                            self.ui.screen = crate::ui::Screen::Inventory;
                            self.mouse_captured = false;
                        }
                    }
                    Some(Panel::Close) => self.ui.attributes.open = false,
                    Some(Panel::Modified(on)) => self.ui.attributes.modified_only = on,
                    None => {}
                }
                Ok(self.attributes_json())
            }
            Command::Throw => {
                self.throw_selected();
                Ok("{\"ok\":true}".to_string())
            }
        }
    }

    /// La pile en main : `damage`, usure, enchantements et fiche.
    fn sheet_json(&self) -> String {
        let Some(stack) = self.inventory.selected_stack() else {
            return "{\"ok\":true,\"item\":null}".to_string();
        };
        let key = &self.content.items.get(stack.item).key;
        let labels: Vec<String> = enchanting::labels(stack.damage)
            .iter()
            .map(|l| crate::control::json_str(l))
            .collect();
        let extended = item_data::is_extended(stack.damage);
        format!(
            "{{\"ok\":true,\"item\":{},\"damage\":{},\"extended\":{},\"number\":{},\"wear\":{},\"enchantments\":[{}],\"sheets\":{}}}",
            crate::control::json_str(key),
            stack.damage,
            extended,
            if extended {
                item_data::number(stack.damage)
            } else {
                0
            },
            enchanting::wear(stack.damage),
            labels.join(","),
            item_data::read(|s| s.len())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enchant_reads_pairs_of_kind_and_level() {
        let parsed = parse("enchant", &["sharpness", "2", "unbreaking", "1"])
            .unwrap()
            .unwrap();
        assert_eq!(
            parsed,
            Command::Enchant(vec![
                (Enchantment::Sharpness, 2),
                (Enchantment::Unbreaking, 1)
            ])
        );
        assert!(parse("enchant", &["sharpness"]).unwrap().is_err());
        assert!(parse("enchant", &["sharpness", "9"]).unwrap().is_err());
        assert!(parse("enchant", &["feu", "1"]).unwrap().is_err());
        assert!(parse("attributes", &["modified", "on"]).unwrap().is_ok());
        assert!(parse("attributes", &["nope"]).unwrap().is_err());
        assert!(parse("autre", &[]).is_none());
    }
}
