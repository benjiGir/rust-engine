//! Le commerce avec les habitants des villages : des échanges décrits en données
//! (`assets/core/trades.ron`), un nombre d'utilisations par habitant.

use std::collections::HashMap;

use anyhow::{Context as _, Result, bail};
use engine_voxel::{ItemId, ItemRegistry};
use serde::Deserialize;

use crate::player::inventory::{Inventory, ItemStack};

const BUILTIN_TRADES: &str = include_str!("../../../../assets/core/trades.ron");

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeEntry {
    give: (String, u32),
    get: (String, u32),
    uses: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradesFile {
    trades: HashMap<String, Vec<TradeEntry>>,
}

/// Un échange : le joueur donne `give`, reçoit `get`, au plus `uses` fois.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trade {
    pub give: (ItemId, u32),
    pub get: (ItemId, u32),
    pub uses: u32,
}

/// Les échanges de chaque sorte d'habitant (`core:farmer`...).
#[derive(Debug, Clone, Default)]
pub struct TradeBook {
    by_mob: HashMap<String, Vec<Trade>>,
}

impl TradeBook {
    pub fn builtin(items: &ItemRegistry) -> Result<Self> {
        Self::parse(BUILTIN_TRADES, "trades.ron intégré", items)
    }

    pub fn parse(text: &str, origin: &str, items: &ItemRegistry) -> Result<Self> {
        let file: TradesFile = ron::Options::default()
            .from_str(text)
            .with_context(|| format!("{origin} : RON invalide"))?;
        let mut by_mob = HashMap::new();
        for (mob, entries) in file.trades {
            let mut trades = Vec::new();
            for entry in entries {
                let resolve = |(key, count): &(String, u32)| -> Result<(ItemId, u32)> {
                    if *count == 0 {
                        bail!("{origin} : « {mob} » : un nombre nul");
                    }
                    let id = items.id(key).with_context(|| {
                        format!("{origin} : « {mob} » : objet « {key} » inconnu")
                    })?;
                    Ok((id, *count))
                };
                trades.push(Trade {
                    give: resolve(&entry.give)?,
                    get: resolve(&entry.get)?,
                    uses: entry.uses,
                });
            }
            by_mob.insert(mob, trades);
        }
        Ok(Self { by_mob })
    }

    /// Les échanges d'une créature (vide si elle ne commerce pas).
    pub fn for_mob(&self, key: &str) -> &[Trade] {
        self.by_mob.get(key).map_or(&[], Vec::as_slice)
    }

    pub fn trades(&self, key: &str) -> bool {
        self.by_mob.contains_key(key)
    }
}

/// Le joueur peut-il faire cet échange ? Il faut ce qu'il donne, et des utilisations
/// restantes.
pub fn can_trade(inventory: &Inventory, trade: &Trade, uses_left: u32) -> bool {
    uses_left > 0 && inventory.count(trade.give.0) >= trade.give.1
}

/// Fait l'échange. Renvoie ce qui n'a pas tenu dans l'inventaire (à jeter par terre), ou
/// `None` si l'échange est impossible.
pub fn execute(
    inventory: &mut Inventory,
    items: &ItemRegistry,
    trade: &Trade,
    uses_left: u32,
) -> Option<u32> {
    if !can_trade(inventory, trade, uses_left) {
        return None;
    }
    inventory.remove(trade.give.0, trade.give.1);
    Some(inventory.add_stack(ItemStack::new(trade.get.0, trade.get.1), items))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    #[test]
    fn the_builtin_trades_only_name_known_items_and_fair_prices() {
        let items = items();
        let book = TradeBook::builtin(&items).unwrap();
        for mob in ["core:farmer", "core:smith", "core:merchant"] {
            assert!(book.for_mob(mob).len() >= 5, "{mob}");
        }
        assert!(book.for_mob("core:pig").is_empty());
        // La monnaie : chaque habitant achète quelque chose contre de l'or, et en vend.
        let gold = items.id("core:gold_ingot").unwrap();
        for mob in ["core:farmer", "core:smith", "core:merchant"] {
            let trades = book.for_mob(mob);
            assert!(trades.iter().any(|t| t.get.0 == gold), "{mob} achète");
            assert!(trades.iter().any(|t| t.give.0 == gold), "{mob} vend");
        }
    }

    #[test]
    fn a_trade_swaps_items_and_runs_out() {
        let items = items();
        let book = TradeBook::builtin(&items).unwrap();
        let wheat = items.id("core:wheat").unwrap();
        let gold = items.id("core:gold_ingot").unwrap();
        let trade = book.for_mob("core:farmer")[0].clone();
        assert_eq!(trade.give, (wheat, 20));
        let mut inventory = Inventory::default();
        inventory.add(wheat, 45, &items);
        assert!(can_trade(&inventory, &trade, 2));
        assert_eq!(execute(&mut inventory, &items, &trade, 2), Some(0));
        assert_eq!(inventory.count(wheat), 25);
        assert_eq!(inventory.count(gold), 1);
        assert_eq!(execute(&mut inventory, &items, &trade, 1), Some(0));
        assert_eq!(inventory.count(wheat), 5);
        // Plus assez de blé, ou plus d'utilisations : rien ne change.
        assert_eq!(execute(&mut inventory, &items, &trade, 5), None);
        inventory.add(wheat, 30, &items);
        assert_eq!(execute(&mut inventory, &items, &trade, 0), None);
        assert_eq!(inventory.count(wheat), 35);
    }

    #[test]
    fn a_full_inventory_returns_the_overflow() {
        let items = items();
        let book = TradeBook::builtin(&items).unwrap();
        let stone = items.id("core:stone").unwrap();
        let wheat = items.id("core:wheat").unwrap();
        let mut inventory = Inventory::default();
        // Toutes les cases pleines de pierre, sauf du blé qu'on va donner.
        for slot in inventory.slots.iter_mut() {
            *slot = Some(ItemStack::new(stone, 64));
        }
        inventory.slots[0] = Some(ItemStack::new(wheat, 20));
        let trade = book.for_mob("core:farmer")[0].clone();
        // La case du blé se libère : l'or y tient.
        assert_eq!(execute(&mut inventory, &items, &trade, 1), Some(0));
        // Plus de place pour une seconde récompense : elle est rendue.
        inventory.slots[1] = Some(ItemStack::new(wheat, 20));
        inventory.slots[0] = Some(ItemStack::new(stone, 64));
        let left = execute(&mut inventory, &items, &trade, 1);
        assert_eq!(left, Some(0), "la case libérée accueille la récompense");
    }

    #[test]
    fn bad_trade_files_are_refused() {
        let items = items();
        let bad = r#"(trades: {"core:x": [(give: ("core:nothing", 1), get: ("core:wheat", 1), uses: 1)]})"#;
        assert!(TradeBook::parse(bad, "t", &items).is_err());
        let zero =
            r#"(trades: {"core:x": [(give: ("core:wheat", 0), get: ("core:wheat", 1), uses: 1)]})"#;
        assert!(TradeBook::parse(zero, "t", &items).is_err());
    }
}
