//! Le stockage d'un type de composant : un « sparse set ».
//!
//! Deux tableaux :
//! - `dense` : les composants les uns à la suite des autres, sans trou. Parcourir tous
//!   les `Position` revient à parcourir un tableau contigu : idéal pour le cache du
//!   processeur, c'est tout l'intérêt d'un ECS.
//! - `sparse` : pour chaque numéro d'entité, la place de son composant dans `dense`
//!   (ou rien). Retrouver le composant d'une entité ne coûte qu'une lecture de tableau.
//!
//! Retirer un composant : on met le dernier élément de `dense` à sa place
//! (`swap_remove`), et on corrige `sparse` pour cet élément déplacé. Pas de trou,
//! et c'est en temps constant.

use std::any::Any;

use crate::entity::Entity;

/// Ce que le monde sait faire avec n'importe quel stockage, sans connaître son type.
///
/// Le monde garde des `Box<dyn ComponentStorage>` : un « objet trait », c'est-à-dire
/// un pointeur vers une valeur de type inconnu à la compilation, plus une table de ses
/// méthodes. Pour retrouver le vrai type (`SparseSet<Position>`), on passe par `Any`
/// et `downcast_ref`, qui vérifie le type à l'exécution.
pub trait ComponentStorage: Any {
    fn remove_entity(&mut self, entity: Entity);
    fn contains(&self, entity: Entity) -> bool;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// Le nom du type de composant, pour le debug.
    fn type_name(&self) -> &'static str;
}

/// Les composants d'un type `T`.
#[derive(Debug, Clone)]
pub struct SparseSet<T> {
    /// Numéro d'entité → place dans `dense` (`u32::MAX` = absent).
    sparse: Vec<u32>,
    entities: Vec<Entity>,
    dense: Vec<T>,
}

const ABSENT: u32 = u32::MAX;

impl<T> Default for SparseSet<T> {
    fn default() -> Self {
        Self {
            sparse: Vec::new(),
            entities: Vec::new(),
            dense: Vec::new(),
        }
    }
}

impl<T> SparseSet<T> {
    fn slot(&self, entity: Entity) -> Option<usize> {
        let slot = *self.sparse.get(entity.index as usize)?;
        // La génération doit correspondre : une entité périmée ne trouve rien.
        (slot != ABSENT && self.entities[slot as usize] == entity).then_some(slot as usize)
    }

    /// Ajoute (ou remplace) le composant ; renvoie l'ancien s'il y en avait un.
    pub fn insert(&mut self, entity: Entity, value: T) -> Option<T> {
        let i = entity.index as usize;
        if i >= self.sparse.len() {
            self.sparse.resize(i + 1, ABSENT);
        }
        let slot = self.sparse[i];
        if slot != ABSENT {
            let slot = slot as usize;
            // La case appartenait peut-être à une ancienne génération : on l'écrase.
            self.entities[slot] = entity;
            return Some(std::mem::replace(&mut self.dense[slot], value));
        }
        self.sparse[i] = self.dense.len() as u32;
        self.entities.push(entity);
        self.dense.push(value);
        None
    }

    pub fn remove(&mut self, entity: Entity) -> Option<T> {
        let slot = self.slot(entity)?;
        self.sparse[entity.index as usize] = ABSENT;
        // `swap_remove` : le dernier élément prend la place du retiré.
        let value = self.dense.swap_remove(slot);
        self.entities.swap_remove(slot);
        if let Some(moved) = self.entities.get(slot) {
            self.sparse[moved.index as usize] = slot as u32;
        }
        Some(value)
    }

    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.slot(entity).map(|slot| &self.dense[slot])
    }

    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.slot(entity).map(|slot| &mut self.dense[slot])
    }

    /// Les entités qui ont ce composant, dans l'ordre de `dense`.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.entities.iter().copied().zip(&self.dense)
    }

    /// Itération mutable : possible ici, car on parcourt un seul tableau à la fois.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.entities.iter().copied().zip(&mut self.dense)
    }
}

impl<T: 'static> ComponentStorage for SparseSet<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn contains(&self, entity: Entity) -> bool {
        self.slot(entity).is_some()
    }

    fn len(&self) -> usize {
        self.dense.len()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn type_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(index: u32) -> Entity {
        Entity {
            index,
            generation: 0,
        }
    }

    #[test]
    fn insert_get_remove_keep_the_array_dense() {
        let mut set = SparseSet::default();
        for i in 0..5 {
            set.insert(entity(i), i * 10);
        }
        assert_eq!(set.remove(entity(1)), Some(10));
        // Le dernier (4) a pris la place du retiré : toujours sans trou.
        assert_eq!(
            set.entities(),
            &[entity(0), entity(4), entity(2), entity(3)]
        );
        assert_eq!(set.get(entity(4)), Some(&40));
        assert_eq!(set.get(entity(1)), None);
        *set.get_mut(entity(3)).unwrap() += 1;
        assert_eq!(set.get(entity(3)), Some(&31));
    }

    #[test]
    fn stale_generation_finds_nothing() {
        let mut set = SparseSet::default();
        set.insert(entity(2), "ancien");
        let newer = Entity {
            index: 2,
            generation: 1,
        };
        assert_eq!(set.get(newer), None);
        set.insert(newer, "nouveau");
        assert_eq!(set.get(newer), Some(&"nouveau"));
        assert_eq!(set.get(entity(2)), None);
    }
}
