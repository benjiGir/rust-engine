//! Les données étendues des objets : une « fiche » par objet, rangée dans le monde.
//!
//! # Pourquoi
//!
//! Une pile ne porte qu'un entier, `ItemStack::damage`. Il suffit pour l'usure et un seul
//! enchantement (voir `enchanting.rs`), pas pour plusieurs enchantements, une rareté, des
//! affixes, des emplacements de gemmes ou un nom. Ces données vont donc dans un magasin, le
//! [`ItemStore`], sur le modèle de celui des sacs à dos (`backpacks.rs`) : la pile ne garde
//! qu'un **numéro de fiche**, et c'est la pile qui voyage (inventaire, coffre, sol, sac,
//! wagon, sauvegarde), jamais la fiche.
//!
//! # Le format
//!
//! - **Dans la pile** : une pile « étendue » a le **bit 31** de `damage` à 1 ; ses bits 0 à 30
//!   donnent le numéro de la fiche ([`NUMBER_MASK`]). L'enchantement simple n'a jamais ce bit
//!   (sa sorte, dans les bits 24 à 31, reste sous 128), et les sacs à dos ne le mettent pas non
//!   plus (leur numéro tient sur 16 bits) : des tests le vérifient. Les chunks ne changent pas :
//!   ils ne contiennent aucune pile.
//! - **Dans le monde** : `items.bin`, à la racine du monde, à côté de `backpacks.bin` (donc
//!   partagé par toutes les dimensions). C'est un [`SavedStore`] encodé en postcard, comme les
//!   sacs ; postcard n'étant pas auto-descriptif, ajouter un champ à [`ItemData`] exige de
//!   monter `SavedStore::version` et de garder l'ancienne forme pour la relire.
//! - **La fiche** ([`ItemData`]) : l'usure, les enchantements (plusieurs : sorte et niveau),
//!   la rareté, les affixes (un bonus d'attribut chacun), les emplacements de gemmes (vide ou
//!   la clé de la gemme) et un nom. Rareté, affixes, emplacements et nom sont remplis par la phase
//!   « aventure » (voir `affixes.rs`, `gems.rs`) ; une fiche d'enchantements seule les laisse vides.
//!   Cette phase n'a **pas** changé la forme de [`ItemData`] : `items.bin` reste en version 1, les
//!   fiches de la phase précédente se lisent telles quelles.
//! - **Un marqueur dans `damage`** : les deux bits hauts à 1 ([`is_pending`]) ne sont pas une fiche mais
//!   un objet à affixes « à tirer » rangé par un coffre de structure ; les numéros de fiche s'arrêtent
//!   donc à [`MAX_NUMBER`] (2^30 - 1).
//!
//! # La migration
//!
//! Une pile simple (usure et au plus un enchantement dans `damage`) le reste tant qu'elle n'a
//! pas besoin d'une donnée étendue. Dès qu'un deuxième enchantement ou un affixe arrive,
//! `enchanting::edit_sheet` crée la fiche, y recopie l'usure et l'enchantement existants, et
//! donne à la pile son numéro. Une fiche d'objet détruit (outil brisé) est libérée ; celle d'un
//! objet disparu autrement (vidé d'un coffre détruit, par exemple) reste : il n'y a pas de
//! ramasse-miettes à cette phase, une fiche pèse quelques dizaines d'octets.
//!
//! # Le magasin du jeu
//!
//! Les piles sont lues partout (barre d'objets, bulles, minage, armure) par des fonctions qui
//! n'ont que `damage` sous la main. Le magasin du monde ouvert est donc unique pour le
//! processus ([`replace_all`] le remplace au chargement d'un monde), derrière un verrou. Les
//! tests, eux, travaillent sur des [`ItemStore`] à eux.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

/// Le bit 31 de `damage` : la pile a une fiche.
pub const EXTENDED: u32 = 1 << 31;
/// Les bits 0 à 30 : le numéro de la fiche.
pub const NUMBER_MASK: u32 = EXTENDED - 1;
/// Le plus grand numéro de fiche : les numéros à partir de 2^30 sont réservés au marqueur « objet à
/// affixes à tirer » que les coffres des structures portent (voir [`is_pending`]).
pub const MAX_NUMBER: u32 = (1 << 30) - 1;
const FILE: &str = "items.bin";

/// Cette pile est-elle un objet à affixes pas encore tiré (un coffre de structure) ? Le moteur le
/// pose à la génération (`engine_voxel::pending_loot`) ; le jeu le remplace par une vraie fiche à la
/// première ouverture du coffre (voir `affixes::resolve_pending`).
pub fn is_pending(damage: u32) -> bool {
    damage & engine_voxel::PENDING_LOOT == engine_voxel::PENDING_LOOT
}

/// Cette pile a-t-elle une fiche ?
pub fn is_extended(damage: u32) -> bool {
    damage & EXTENDED != 0 && !is_pending(damage)
}

/// Le numéro de fiche d'un `damage` étendu.
pub fn number(damage: u32) -> u32 {
    damage & NUMBER_MASK
}

/// Un bonus d'attribut porté par un objet (un affixe).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Affix {
    /// La clé de l'affixe dans les données (`affixes.ron`, phase suivante).
    pub key: String,
    /// L'attribut touché : son numéro stable (`Attribute::id`).
    pub attribute: u8,
    /// Ce qui s'ajoute à l'attribut.
    pub flat: f32,
    /// La part qui le multiplie (0,1 = +10 %).
    pub percent: f32,
}

/// La fiche d'un objet.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemData {
    /// L'usure (le même sens que les bits bas de `damage` d'une pile simple).
    pub wear: u32,
    /// Les enchantements : (numéro de sorte, niveau), chaque sorte une seule fois.
    pub enchantments: Vec<(u8, u8)>,
    /// La rareté (0 = commune).
    pub rarity: u8,
    pub affixes: Vec<Affix>,
    /// Les emplacements de gemmes : vide, ou la clé de la gemme posée.
    pub sockets: Vec<Option<String>>,
    /// Un nom qui remplace celui de l'objet (objets à affixes).
    pub name: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SavedStore {
    version: u32,
    next: u32,
    items: Vec<(u32, ItemData)>,
}

/// Le magasin des fiches d'un monde.
#[derive(Debug, Default, PartialEq)]
pub struct ItemStore {
    sheets: BTreeMap<u32, ItemData>,
    next: u32,
    dirty: bool,
    /// Monte à chaque changement de fiche autre que l'usure : les attributs en dépendent.
    revision: u64,
}

impl ItemStore {
    pub fn len(&self) -> usize {
        self.sheets.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }

    pub fn get(&self, number: u32) -> Option<&ItemData> {
        self.sheets.get(&number)
    }

    /// Une fiche neuve : renvoie son numéro (`None` si les 2^31 - 1 numéros sont pris).
    pub fn create(&mut self, data: ItemData) -> Option<u32> {
        self.next = self.next.max(1);
        while self.sheets.contains_key(&self.next) {
            self.next += 1;
        }
        if self.next > MAX_NUMBER {
            return None;
        }
        let number = self.next;
        self.next += 1;
        self.sheets.insert(number, data);
        self.dirty = true;
        self.revision += 1;
        Some(number)
    }

    /// La fiche, à modifier. `structural` : le changement touche autre chose que l'usure
    /// (les attributs sont à recalculer).
    pub fn edit(&mut self, number: u32, structural: bool) -> Option<&mut ItemData> {
        let sheet = self.sheets.get_mut(&number)?;
        self.dirty = true;
        if structural {
            self.revision += 1;
        }
        Some(sheet)
    }

    /// Libère une fiche (l'objet n'existe plus).
    pub fn remove(&mut self, number: u32) {
        if self.sheets.remove(&number).is_some() {
            self.dirty = true;
            self.revision += 1;
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Écrit `items.bin` dans le dossier du monde (le fichier n'existe qu'une fois une fiche
    /// créée).
    pub fn save(&mut self, dir: &Path) -> Result<()> {
        if self.sheets.is_empty() {
            return Ok(());
        }
        let saved = SavedStore {
            version: 1,
            next: self.next,
            items: self.sheets.iter().map(|(n, d)| (*n, d.clone())).collect(),
        };
        let bytes = postcard::to_allocvec(&saved).context("encodage des fiches d'objets")?;
        engine_voxel::write_atomic(&dir.join(FILE), &bytes)?;
        self.dirty = false;
        Ok(())
    }

    /// Relit `items.bin` (un magasin vide s'il n'existe pas). Un fichier illisible est mis de
    /// côté (`items.bin.illisible`), jamais écrasé.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match postcard::from_bytes::<SavedStore>(&bytes) {
            Ok(saved) => {
                log::info!("{} fiche(s) d'objet chargée(s)", saved.items.len());
                Self {
                    sheets: saved.items.into_iter().collect(),
                    next: saved.next,
                    dirty: false,
                    revision: 0,
                }
            }
            Err(err) => {
                log::error!("{} illisible ({err}) : mis de côté", path.display());
                let _ = std::fs::copy(&path, dir.join("items.bin.illisible"));
                Self::default()
            }
        }
    }
}

static STORE: LazyLock<Mutex<ItemStore>> = LazyLock::new(Mutex::default);

fn lock() -> MutexGuard<'static, ItemStore> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Passe au monde ouvert : son magasin remplace celui du précédent.
pub fn replace_all(store: ItemStore) {
    let mut guard = lock();
    let revision = guard.revision + 1;
    *guard = store;
    guard.revision = revision;
}

/// Lit le magasin du monde ouvert.
pub fn read<R>(f: impl FnOnce(&ItemStore) -> R) -> R {
    f(&lock())
}

/// Modifie le magasin du monde ouvert.
pub fn write<R>(f: impl FnOnce(&mut ItemStore) -> R) -> R {
    f(&mut lock())
}

/// La fiche de cette pile (`None` pour une pile simple, ou si la fiche est perdue).
pub fn sheet(damage: u32) -> Option<ItemData> {
    is_extended(damage)
        .then(|| read(|s| s.get(number(damage)).cloned()))
        .flatten()
}

/// Le numéro de révision du magasin (voir [`ItemStore::revision`]).
pub fn revision() -> u64 {
    read(ItemStore::revision)
}

/// Sauvegarde le magasin du monde ouvert dans `dir` s'il a changé.
pub fn save_if_dirty(dir: &Path) -> Result<()> {
    let mut guard = lock();
    if guard.dirty() {
        guard.save(dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("voxel-item-data-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rich_sheet() -> ItemData {
        ItemData {
            wear: 12,
            enchantments: vec![(1, 2), (3, 1)],
            rarity: 3,
            affixes: vec![Affix {
                key: "acérée".into(),
                attribute: 0,
                flat: 1.5,
                percent: 0.1,
            }],
            sockets: vec![None, Some("core:ruby".into())],
            name: Some("Épée de la tempête".into()),
        }
    }

    #[test]
    fn the_extended_bit_and_the_number_do_not_overlap() {
        assert!(is_extended(EXTENDED | 7));
        assert!(!is_extended(7));
        assert_eq!(number(EXTENDED | 7), 7);
        assert_eq!(NUMBER_MASK, 0x7FFF_FFFF);
        // Le marqueur des coffres (bits 31 et 30) n'est jamais une fiche.
        let pending = engine_voxel::pending_loot(2, 0xBEEF);
        assert!(is_pending(pending) && !is_extended(pending));
        assert!(!is_pending(EXTENDED | MAX_NUMBER) && is_extended(EXTENDED | MAX_NUMBER));
        // Les sacs à dos, eux, restent sous les 16 bits.
        const { assert!(crate::backpacks::MAX_NUMBER < EXTENDED) };
    }

    #[test]
    fn sheets_round_trip_through_items_bin() {
        let dir = temp_dir("roundtrip");
        let mut store = ItemStore::default();
        let a = store.create(rich_sheet()).unwrap();
        let b = store.create(ItemData::default()).unwrap();
        assert_ne!(a, b);
        store.edit(b, false).unwrap().wear = 5;
        assert!(store.dirty());
        store.save(&dir).unwrap();
        assert!(!store.dirty());
        let back = ItemStore::load(&dir);
        assert_eq!(back.get(a), Some(&rich_sheet()));
        assert_eq!(back.get(b).unwrap().wear, 5);
        assert_eq!(back.len(), 2);
        // Les numéros ne sont pas réutilisés après le rechargement.
        let mut back = back;
        let c = back.create(ItemData::default()).unwrap();
        assert!(c > b);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_missing_file_is_an_empty_store_and_a_broken_one_is_kept_aside() {
        let dir = temp_dir("broken");
        assert!(ItemStore::load(&dir).is_empty());
        std::fs::write(dir.join("items.bin"), [0xFF; 9]).unwrap();
        assert!(ItemStore::load(&dir).is_empty());
        assert!(dir.join("items.bin.illisible").exists());
        assert!(dir.join("items.bin").exists(), "jamais écrasé");
        // Un monde sans fiche n'écrit rien.
        let empty = temp_dir("empty");
        ItemStore::default().save(&empty).unwrap();
        assert!(!empty.join("items.bin").exists());
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(empty);
    }

    #[test]
    fn only_structural_edits_move_the_revision() {
        let mut store = ItemStore::default();
        let n = store.create(ItemData::default()).unwrap();
        let first = store.revision();
        store.edit(n, false).unwrap().wear += 1;
        assert_eq!(
            store.revision(),
            first,
            "l'usure ne change pas les attributs"
        );
        store.edit(n, true).unwrap().rarity = 1;
        assert!(store.revision() > first);
        store.remove(n);
        assert!(store.get(n).is_none());
    }
}
