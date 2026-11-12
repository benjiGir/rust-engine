//! Les objets : ce qu'on tient en main et range dans un inventaire.
//!
//! Un bloc n'est pas un objet : le bloc vit dans le monde, l'objet dans un inventaire.
//! Chaque bloc « placeable » a son objet, du même identifiant (`core:stone`), qui pose ce
//! bloc. Les autres objets (outils, lingots, bâtons...) sont décrits dans `items.ron`.
//!
//! Comme les blocs, un objet a un identifiant texte (stable, utilisé dans les sauvegardes)
//! et un numéro [`ItemId`] attribué au chargement (rapide, utilisé en mémoire).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::block::BlockId;
use crate::registry::BlockRegistry;

const BUILTIN_ITEMS: &str = include_str!("../../../assets/core/items.ron");

/// Le numéro d'un objet dans le registre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ItemId(pub u16);

/// Les sortes d'outils. Chaque bloc a son outil préféré (`tool` dans `blocks.ron`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum ToolKind {
    Pickaxe,
    Axe,
    Shovel,
    Hoe,
    Sword,
    Shears,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct ToolDef {
    pub kind: ToolKind,
    /// Le niveau : 1 bois et or, 2 pierre et cuivre, 3 fer, 4 saphir.
    pub tier: u8,
    /// Le multiplicateur de vitesse de minage sur les blocs de son type.
    pub speed: f32,
    /// Les dégâts d'un coup.
    pub damage: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum ArmorSlot {
    Head,
    Chest,
    Legs,
    Feet,
}

impl ArmorSlot {
    pub const ALL: [ArmorSlot; 4] = [
        ArmorSlot::Head,
        ArmorSlot::Chest,
        ArmorSlot::Legs,
        ArmorSlot::Feet,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Head => "Tête",
            Self::Chest => "Torse",
            Self::Legs => "Jambes",
            Self::Feet => "Pieds",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ArmorDef {
    pub slot: ArmorSlot,
    /// Des points d'armure : chacun retire une part des dégâts (voir la phase O4).
    pub protection: u32,
}

/// Ce qu'apporte un aliment (phase O4) : des points de faim, et de la saturation (une
/// réserve qui s'épuise avant la faim).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct FoodDef {
    pub nutrition: f32,
    pub saturation: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
pub enum Category {
    Block,
    Material,
    Tool,
    Armor,
    Food,
}

/// L'image d'un objet : la texture de son bloc, ou une icône à lui (`items/<nom>.png`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Block(BlockId),
    /// Identifiant de l'icône, avec espace de noms (`core:stick`).
    Texture(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemDef {
    pub key: String,
    pub name: String,
    pub category: Category,
    pub max_stack: u32,
    /// Utilisations avant de casser (outils, armures).
    pub durability: Option<u32>,
    /// Le bloc posé par cet objet.
    pub block: Option<BlockId>,
    /// La clé du bloc planté en posant cet objet sur de la terre labourée (une graine).
    pub plants: Option<String>,
    pub tool: Option<ToolDef>,
    pub armor: Option<ArmorDef>,
    /// Un aliment : ce qu'il nourrit.
    pub food: Option<FoodDef>,
    /// Un clic droit avec cet objet en main (hors d'une action du jeu) prévient les scripts des mods
    /// (`Event::ItemUsed`) : une baguette, un grimoire, un outil de mod.
    pub usable: bool,
    /// Sa couleur, pour l'objet posé au sol.
    pub color: [f32; 3],
    pub icon: Icon,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemEntry {
    id: String,
    name: String,
    category: Category,
    color: (f32, f32, f32),
    icon: String,
    #[serde(default)]
    max_stack: Option<u32>,
    #[serde(default)]
    durability: Option<u32>,
    #[serde(default)]
    plants: Option<String>,
    #[serde(default)]
    tool: Option<ToolDef>,
    #[serde(default)]
    armor: Option<ArmorDef>,
    #[serde(default)]
    food: Option<FoodDef>,
    #[serde(default)]
    usable: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsFile {
    items: Vec<ItemEntry>,
    #[serde(default)]
    tags: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct ItemRegistry {
    defs: Vec<ItemDef>,
    by_key: HashMap<String, ItemId>,
    /// L'objet de chaque bloc (par numéro de bloc).
    by_block: HashMap<BlockId, ItemId>,
    /// Les groupes : `planks` → les planches de toutes les essences.
    tags: HashMap<String, Vec<ItemId>>,
}

fn qualify(name: &str, namespace: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("{namespace}:{name}")
    }
}

impl ItemRegistry {
    /// Les objets des blocs, puis ceux de `items.ron` intégré.
    pub fn builtin(blocks: &BlockRegistry) -> Result<Self> {
        let mut registry = Self::from_blocks(blocks);
        registry.add_source("core", BUILTIN_ITEMS, "items.ron intégré")?;
        Ok(registry)
    }

    /// `assets/core/items.ron`, puis le `items.ron` de chaque mod, dans l'ordre.
    pub fn load_mods(
        blocks: &BlockRegistry,
        assets_dir: &Path,
        mods: &[(String, PathBuf)],
    ) -> Result<Self> {
        let mut registry = Self::from_blocks(blocks);
        let mut files = vec![(
            "core".to_string(),
            assets_dir.join("core").join("items.ron"),
        )];
        files.extend(
            mods.iter()
                .map(|(ns, dir)| (ns.clone(), dir.join("items.ron"))),
        );
        for (namespace, path) in files {
            if path.exists() {
                let text = std::fs::read_to_string(&path)
                    .with_context(|| format!("lecture de {}", path.display()))?;
                registry.add_source(&namespace, &text, &path.display().to_string())?;
            }
        }
        Ok(registry)
    }

    /// Un objet par bloc « placeable ».
    fn from_blocks(blocks: &BlockRegistry) -> Self {
        let mut registry = Self::default();
        for (id, def) in blocks.iter() {
            if !def.placeable {
                continue;
            }
            let item = ItemDef {
                key: def.key.clone(),
                name: def.name.clone(),
                category: Category::Block,
                max_stack: 64,
                durability: None,
                block: Some(id),
                plants: None,
                tool: None,
                armor: None,
                food: None,
                usable: false,
                color: def.color,
                icon: Icon::Block(id),
            };
            let item_id = registry.push(item);
            registry.by_block.insert(id, item_id);
        }
        // Une variante (orientation, moitié...) lâche et se choisit comme son bloc de base.
        for (id, def) in blocks.iter() {
            if let Some(item) = def.base.and_then(|b| registry.by_block.get(&b).copied()) {
                registry.by_block.insert(id, item);
            }
        }
        registry
    }

    fn push(&mut self, def: ItemDef) -> ItemId {
        let id = ItemId(self.defs.len() as u16);
        self.by_key.insert(def.key.clone(), id);
        self.defs.push(def);
        id
    }

    pub fn add_source(&mut self, namespace: &str, source: &str, origin: &str) -> Result<()> {
        let options = ron::Options::default()
            .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let file: ItemsFile = options
            .from_str(source)
            .with_context(|| format!("{origin} : RON invalide"))?;
        for e in file.items {
            if e.id.split_once(':').map(|(ns, _)| ns) != Some(namespace) {
                bail!(
                    "{origin} : l'objet « {} » doit commencer par « {namespace}: »",
                    e.id
                );
            }
            if self.by_key.contains_key(&e.id) {
                bail!("{origin} : l'objet « {} » est défini deux fois", e.id);
            }
            if self.defs.len() >= u16::MAX as usize {
                bail!("trop d'objets");
            }
            let (r, g, b) = e.color;
            let stackable = e.durability.is_none();
            self.push(ItemDef {
                name: e.name,
                category: e.category,
                max_stack: e.max_stack.unwrap_or(if stackable { 64 } else { 1 }),
                durability: e.durability,
                block: None,
                plants: e.plants,
                tool: e.tool,
                armor: e.armor,
                food: e.food,
                usable: e.usable,
                color: [r, g, b],
                icon: Icon::Texture(qualify(&e.icon, namespace)),
                key: e.id,
            });
        }
        for (tag, members) in file.tags {
            let ids = members
                .iter()
                .map(|m| {
                    let key = qualify(m, namespace);
                    self.id(&key).with_context(|| {
                        format!("{origin} : groupe « {tag} » : objet inconnu « {key} »")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            self.tags.entry(tag).or_default().extend(ids);
        }
        Ok(())
    }

    pub fn id(&self, key: &str) -> Option<ItemId> {
        self.by_key.get(key).copied()
    }

    pub fn get(&self, id: ItemId) -> &ItemDef {
        &self.defs[id.0 as usize]
    }

    /// L'objet qui pose ce bloc.
    pub fn of_block(&self, block: BlockId) -> Option<ItemId> {
        self.by_block.get(&block).copied()
    }

    /// Les membres d'un groupe (`planks`).
    pub fn tag(&self, name: &str) -> &[ItemId] {
        self.tags.get(name).map_or(&[], Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = (ItemId, &ItemDef)> {
        self.defs
            .iter()
            .enumerate()
            .map(|(i, d)| (ItemId(i as u16), d))
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }
}

/// Le temps pour casser un bloc, en secondes, avec cet outil en main (`None` : à la main).
///
/// La règle, simple et proche du genre :
/// - la dureté du bloc est le temps de base (à la main) ;
/// - si l'outil n'est pas assez fort pour lui (niveau trop bas, ou mauvaise sorte pour un
///   bloc qui en exige une), c'est 3 fois plus long, et il ne lâchera rien ;
/// - avec la bonne sorte d'outil, on divise par la vitesse de l'outil.
///
/// `None` : incassable.
pub fn break_time(block: &crate::registry::BlockDef, tool: Option<&ToolDef>) -> Option<f32> {
    if block.hardness < 0.0 {
        return None;
    }
    let right_kind = tool.is_some_and(|t| Some(t.kind) == block.tool);
    let mut time = block.hardness;
    if !can_harvest(block, tool) {
        time *= 3.0;
    }
    if right_kind && let Some(t) = tool {
        time /= t.speed;
    }
    Some(time)
}

/// Le bloc lâche-t-il son butin avec cet outil ? Un bloc de niveau 0 lâche tout à la main ;
/// sinon il faut la bonne sorte d'outil, d'un niveau suffisant.
pub fn can_harvest(block: &crate::registry::BlockDef, tool: Option<&ToolDef>) -> bool {
    block.tier == 0 || tool.is_some_and(|t| Some(t.kind) == block.tool && t.tier >= block.tier)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registries() -> (std::sync::Arc<BlockRegistry>, ItemRegistry) {
        let blocks = BlockRegistry::builtin();
        let items = ItemRegistry::builtin(&blocks).unwrap();
        (blocks, items)
    }

    #[test]
    fn blocks_have_items_and_tools_are_unstackable() {
        let (blocks, items) = registries();
        let stone = blocks.id("core:stone").unwrap();
        let item = items.of_block(stone).unwrap();
        assert_eq!(items.get(item).key, "core:stone");
        assert_eq!(items.get(item).max_stack, 64);
        let pickaxe = items.get(items.id("core:iron_pickaxe").unwrap());
        assert_eq!(pickaxe.max_stack, 1);
        assert_eq!(pickaxe.tool.unwrap().kind, ToolKind::Pickaxe);
        assert_eq!(items.tag("planks").len(), 5);
        // Ni l'eau ni le socle ne sont des objets.
        assert!(items.of_block(blocks.id("core:water").unwrap()).is_none());
    }

    #[test]
    fn break_time_depends_on_the_tool() {
        let (blocks, items) = registries();
        let stone = blocks.def(blocks.id("core:stone").unwrap());
        let tool = |key: &str| items.get(items.id(key).unwrap()).tool;
        let hand = break_time(stone, None).unwrap();
        let wood = break_time(stone, tool("core:wooden_pickaxe").as_ref()).unwrap();
        let iron = break_time(stone, tool("core:iron_pickaxe").as_ref()).unwrap();
        let axe = break_time(stone, tool("core:iron_axe").as_ref()).unwrap();
        assert!(iron < wood && wood < hand, "{iron} < {wood} < {hand}");
        assert_eq!(axe, hand, "une hache ne mine pas la pierre plus vite");
        assert!(!can_harvest(stone, None));
        assert!(can_harvest(stone, tool("core:wooden_pickaxe").as_ref()));
        let gold = blocks.def(blocks.id("core:gold_ore").unwrap());
        assert!(!can_harvest(gold, tool("core:stone_pickaxe").as_ref()));
        assert!(can_harvest(gold, tool("core:iron_pickaxe").as_ref()));
        assert!(
            break_time(
                blocks.def(blocks.id("core:bedrock").unwrap()),
                tool("core:sapphire_pickaxe").as_ref()
            )
            .is_none()
        );
    }
}
