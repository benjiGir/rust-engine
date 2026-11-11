//! Les « capacités » d'un bloc actif : ce qu'il stocke et échange avec ses voisins.
//!
//! Les mods techniques (Create, Mekanism...) échangent tous les mêmes choses : de
//! l'**énergie**, des **fluides**, des **objets**, et de la **rotation** (les engrenages de
//! Create). Plutôt que chaque mod invente son format (et que deux mods ne puissent pas
//! se brancher l'un sur l'autre), le moteur fournit des conteneurs génériques, comme les
//! « capabilities » de Forge. Un câble d'un mod peut alors alimenter la machine d'un autre.
//!
//! Chaque conteneur dit **par quelle face** il accepte d'échanger ([`Sides`]) : un
//! réservoir peut se remplir par le haut et se vider par le bas.
//!
//! Ici, seulement les données et les opérations élémentaires (insérer, extraire, avec
//! un mode « simulation » pour demander « combien accepterais-tu ? » sans rien changer).
//! La simulation des réseaux (câbles, tuyaux, arbres de transmission) viendra en phase 9c.

use serde::{Deserialize, Serialize};

/// Une face d'un bloc. L'ordre est celui des six voisins partout dans le moteur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Face {
    PosX = 0,
    NegX = 1,
    PosY = 2,
    NegY = 3,
    PosZ = 4,
    NegZ = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    /// Depuis un numéro (0 à 5) ; `None` au-delà. Pratique pour l'interface des mods.
    pub fn from_index(i: i32) -> Option<Face> {
        Self::ALL.get(usize::try_from(i).ok()?).copied()
    }

    pub fn opposite(self) -> Face {
        Self::ALL[self as usize ^ 1]
    }
}

/// Le rôle d'un conteneur dans un réseau (câbles, tuyaux) : il reçoit seulement, il fournit
/// seulement, ou les deux (une batterie, un réservoir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Io {
    #[default]
    Both,
    /// Un consommateur : il reçoit, il ne rend rien (une lampe, un remplisseur).
    In,
    /// Un producteur : il fournit, il ne reçoit rien (un panneau solaire, une pompe).
    Out,
}

/// Un ensemble de faces, en 6 bits : bit `i` = [`Face`] numéro `i`. Les deux bits restants
/// codent le rôle ([`Io`]) : les ranger ici évite d'ajouter un champ aux conteneurs, donc de
/// changer le format des sauvegardes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Sides(pub u8);

impl Sides {
    pub const NONE: Sides = Sides(0);
    pub const ALL: Sides = Sides(0b11_1111);

    pub fn contains(self, face: Face) -> bool {
        self.0 & (1 << face as u8) != 0
    }

    const INPUT_ONLY: u8 = 1 << 6;
    const OUTPUT_ONLY: u8 = 1 << 7;

    /// Le rôle dans un réseau.
    pub fn io(self) -> Io {
        if self.0 & Self::INPUT_ONLY != 0 {
            Io::In
        } else if self.0 & Self::OUTPUT_ONLY != 0 {
            Io::Out
        } else {
            Io::Both
        }
    }

    /// Les mêmes faces, avec ce rôle.
    pub fn with_io(self, io: Io) -> Self {
        let faces = self.0 & 0b11_1111;
        Sides(match io {
            Io::Both => faces,
            Io::In => faces | Self::INPUT_ONLY,
            Io::Out => faces | Self::OUTPUT_ONLY,
        })
    }

    /// `None` veut dire « de l'intérieur » (le bloc lui-même ou le jeu) : toujours permis.
    fn allows(self, face: Option<Face>) -> bool {
        face.is_none_or(|f| self.contains(f))
    }
}

impl Default for Sides {
    fn default() -> Self {
        Self::ALL
    }
}

/// Un stock d'énergie (unité libre, comme les FE de Forge).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnergyStorage {
    pub amount: u64,
    pub capacity: u64,
    /// Débit maximal par opération, en entrée et en sortie.
    pub max_insert: u64,
    pub max_extract: u64,
    pub sides: Sides,
}

impl EnergyStorage {
    pub fn new(capacity: u64, max_rate: u64) -> Self {
        Self {
            amount: 0,
            capacity,
            max_insert: max_rate,
            max_extract: max_rate,
            sides: Sides::ALL,
        }
    }

    /// Ajoute jusqu'à `amount` ; renvoie ce qui a été (ou aurait été, si `simulate`) accepté.
    pub fn insert(&mut self, face: Option<Face>, amount: u64, simulate: bool) -> u64 {
        if !self.sides.allows(face) {
            return 0;
        }
        let accepted = amount
            .min(self.max_insert)
            .min(self.capacity - self.amount.min(self.capacity));
        if !simulate {
            self.amount += accepted;
        }
        accepted
    }

    pub fn extract(&mut self, face: Option<Face>, amount: u64, simulate: bool) -> u64 {
        if !self.sides.allows(face) {
            return 0;
        }
        let given = amount.min(self.max_extract).min(self.amount);
        if !simulate {
            self.amount -= given;
        }
        given
    }
}

/// Un réservoir de fluide. Les quantités sont en millibuckets (1000 mB = un bloc de fluide).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FluidTank {
    /// L'identifiant du fluide (`core:water`), `None` si vide. Un texte et pas un numéro :
    /// les numéros changent quand on ajoute un mod, la sauvegarde doit rester valable.
    pub fluid: Option<String>,
    pub amount: u32,
    pub capacity: u32,
    pub sides: Sides,
}

impl FluidTank {
    pub fn new(capacity: u32) -> Self {
        Self {
            fluid: None,
            amount: 0,
            capacity,
            sides: Sides::ALL,
        }
    }

    /// Remplit avec `fluid` ; un réservoir ne contient qu'un fluide à la fois.
    pub fn fill(&mut self, face: Option<Face>, fluid: &str, amount: u32, simulate: bool) -> u32 {
        if !self.sides.allows(face) || self.fluid.as_deref().is_some_and(|f| f != fluid) {
            return 0;
        }
        let accepted = amount.min(self.capacity.saturating_sub(self.amount));
        if !simulate && accepted > 0 {
            self.amount += accepted;
            self.fluid = Some(fluid.to_string());
        }
        accepted
    }

    /// Vide jusqu'à `amount` de `fluid` (ou de n'importe quel fluide si `None`).
    pub fn drain(
        &mut self,
        face: Option<Face>,
        fluid: Option<&str>,
        amount: u32,
        simulate: bool,
    ) -> u32 {
        if !self.sides.allows(face) || fluid.is_some_and(|f| self.fluid.as_deref() != Some(f)) {
            return 0;
        }
        let given = amount.min(self.amount);
        if !simulate {
            self.amount -= given;
            if self.amount == 0 {
                self.fluid = None;
            }
        }
        given
    }
}

/// Une pile d'objets. L'objet est un identifiant texte (`core:stone`, `example:ruby`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemStack {
    pub item: String,
    pub count: u32,
    /// L'usure d'un outil ou d'une armure (0 : neuf). Toujours 0 pour ce qui s'empile.
    pub damage: u32,
}

pub const MAX_STACK: u32 = 64;

/// Un inventaire de cases (un coffre, l'entrée d'un four...).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub slots: Vec<Option<ItemStack>>,
    pub sides: Sides,
}

impl Inventory {
    pub fn new(slots: usize) -> Self {
        Self {
            slots: vec![None; slots],
            sides: Sides::ALL,
        }
    }

    pub fn count(&self, item: &str) -> u32 {
        self.slots
            .iter()
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count)
            .sum()
    }

    /// Range jusqu'à `count` objets : d'abord sur les piles existantes, puis dans les cases vides.
    pub fn insert(&mut self, face: Option<Face>, item: &str, count: u32, simulate: bool) -> u32 {
        if !self.sides.allows(face) {
            return 0;
        }
        let mut left = count;
        let mut slots = self.slots.clone();
        for slot in slots
            .iter_mut()
            .filter(|s| s.as_ref().is_some_and(|s| s.item == item))
        {
            let stack = slot.as_mut().expect("filtré");
            let n = left.min(MAX_STACK - stack.count.min(MAX_STACK));
            stack.count += n;
            left -= n;
        }
        for slot in slots.iter_mut().filter(|s| s.is_none()) {
            if left == 0 {
                break;
            }
            let n = left.min(MAX_STACK);
            *slot = Some(ItemStack {
                item: item.to_string(),
                count: n,
                damage: 0,
            });
            left -= n;
        }
        if !simulate {
            self.slots = slots;
        }
        count - left
    }

    /// Retire jusqu'à `count` objets de `item` (ou du premier objet trouvé si `None`).
    pub fn extract(
        &mut self,
        face: Option<Face>,
        item: Option<&str>,
        count: u32,
        simulate: bool,
    ) -> Option<ItemStack> {
        if !self.sides.allows(face) {
            return None;
        }
        let wanted = item
            .map(str::to_string)
            .or_else(|| self.slots.iter().flatten().next().map(|s| s.item.clone()))?;
        let mut left = count;
        let mut slots = self.slots.clone();
        for slot in slots.iter_mut() {
            if let Some(stack) = slot.as_mut().filter(|s| s.item == wanted) {
                let n = left.min(stack.count);
                stack.count -= n;
                left -= n;
                if stack.count == 0 {
                    *slot = None;
                }
            }
        }
        let taken = count - left;
        if !simulate {
            self.slots = slots;
        }
        (taken > 0).then_some(ItemStack {
            item: wanted,
            count: taken,
            damage: 0,
        })
    }
}

/// La rotation, façon Create : une vitesse (tours par minute, signée pour le sens) et
/// une « contrainte » (stress) : ce qu'une source peut fournir et ce qu'une machine consomme.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Kinetic {
    pub speed: f32,
    /// Ce qu'une source (moulin, moteur) peut entraîner.
    pub stress_capacity: f32,
    /// Ce que ce bloc consomme quand il tourne.
    pub stress_impact: f32,
    pub sides: Sides,
}

/// Toutes les capacités d'un bloc. Boîte à part dans [`crate::BlockEntity`] : un câble
/// n'a qu'un stock d'énergie, une lampe rien du tout ; elle ne paye que ce qu'elle utilise.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Capabilities {
    pub energy: Option<EnergyStorage>,
    pub tanks: Vec<FluidTank>,
    pub inventory: Option<Inventory>,
    pub kinetic: Option<Kinetic>,
}

impl Capabilities {
    /// Remplit les réservoirs dans l'ordre ; renvoie la quantité acceptée.
    pub fn fill(&mut self, face: Option<Face>, fluid: &str, amount: u32, simulate: bool) -> u32 {
        let mut left = amount;
        for tank in &mut self.tanks {
            left -= tank.fill(face, fluid, left, simulate);
        }
        amount - left
    }

    pub fn drain(
        &mut self,
        face: Option<Face>,
        fluid: Option<&str>,
        amount: u32,
        simulate: bool,
    ) -> u32 {
        let mut left = amount;
        for tank in &mut self.tanks {
            left -= tank.drain(face, fluid, left, simulate);
        }
        amount - left
    }

    pub fn fluid_amount(&self, fluid: &str) -> u32 {
        self.tanks
            .iter()
            .filter(|t| t.fluid.as_deref() == Some(fluid))
            .map(|t| t.amount)
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn energy_respects_capacity_rate_and_sides() {
        let mut battery = EnergyStorage::new(1000, 300);
        assert_eq!(battery.insert(None, 500, false), 300, "débit limité");
        assert_eq!(battery.insert(None, 300, true), 300);
        assert_eq!(battery.amount, 300, "une simulation ne change rien");
        battery.sides = Sides(1 << Face::PosY as u8);
        assert_eq!(
            battery.insert(Some(Face::NegY), 10, false),
            0,
            "face fermée"
        );
        assert_eq!(battery.extract(Some(Face::PosY), 1000, false), 300);
    }

    #[test]
    fn a_tank_holds_one_fluid_at_a_time() {
        let mut caps = Capabilities {
            tanks: vec![FluidTank::new(1000), FluidTank::new(500)],
            ..Default::default()
        };
        assert_eq!(caps.fill(None, "core:water", 1200, false), 1200);
        assert_eq!(
            caps.fill(None, "core:lava", 1000, false),
            0,
            "les deux contiennent de l'eau"
        );
        assert_eq!(caps.fluid_amount("core:water"), 1200);
        // Vider le premier réservoir le libère pour un autre fluide.
        assert_eq!(caps.drain(None, Some("core:water"), 1100, false), 1100);
        assert_eq!(caps.tanks[0].fluid, None);
        assert_eq!(caps.fill(None, "core:lava", 2000, false), 1000);
        assert_eq!(caps.fluid_amount("core:water"), 100);
    }

    #[test]
    fn inventories_stack_then_fill_empty_slots() {
        let mut chest = Inventory::new(2);
        assert_eq!(chest.insert(None, "core:stone", 100, false), 100);
        assert_eq!(chest.insert(None, "core:dirt", 10, false), 0, "plein");
        assert_eq!(chest.insert(None, "core:stone", 50, false), 28);
        let taken = chest.extract(None, None, 70, false).unwrap();
        assert_eq!((taken.item.as_str(), taken.count), ("core:stone", 70));
        assert_eq!(chest.count("core:stone"), 58);
    }

    #[test]
    fn the_role_rides_in_the_spare_bits_of_the_sides() {
        let sides = Sides(1 << Face::PosY as u8).with_io(Io::Out);
        assert_eq!(sides.io(), Io::Out);
        assert!(sides.contains(Face::PosY) && !sides.contains(Face::NegY));
        // Le rôle ne change pas les faces autorisées : un stockage `Out` accepte les mêmes
        // échanges directs qu'avant.
        let mut battery = EnergyStorage::new(100, 100);
        battery.sides = Sides::ALL.with_io(Io::In);
        assert_eq!(battery.insert(Some(Face::NegX), 10, false), 10);
        assert_eq!(Sides::ALL.io(), Io::Both);
        assert_eq!(Sides::ALL.with_io(Io::In).with_io(Io::Both), Sides::ALL);
    }

    #[test]
    fn faces_have_opposites() {
        for face in Face::ALL {
            assert_eq!(face.opposite().opposite(), face);
            assert_ne!(face.opposite(), face);
        }
    }
}
