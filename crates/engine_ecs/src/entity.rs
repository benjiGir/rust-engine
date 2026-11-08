//! Les entités : de simples numéros, avec une « génération ».
//!
//! # Pourquoi pas des références ?
//!
//! En C#, on garderait une référence vers l'objet « cochon ». En Rust, une référence
//! `&Cochon` emprunte le monde : tant qu'on la garde, impossible de modifier le monde.
//! Et si le cochon meurt, que deviendrait la référence ? On utilise donc un **numéro** :
//! l'entité 42. On le stocke où l'on veut (la cible d'un monstre, une sauvegarde...).
//!
//! # Pourquoi une génération ?
//!
//! Quand l'entité 42 meurt, sa case est réutilisée pour la prochaine entité créée.
//! Un monstre qui gardait « 42 » comme cible viserait alors la nouvelle venue, par
//! erreur. Chaque case porte donc un compteur, la génération, augmenté à chaque
//! réutilisation : l'ancien numéro devient `(42, génération 0)`, le nouveau
//! `(42, génération 1)`. Le monde refuse un numéro dont la génération est périmée.

/// Une entité : une case (`index`) et sa génération.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    pub index: u32,
    pub generation: u32,
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}v{}", self.index, self.generation)
    }
}

/// Distribue les numéros d'entités et recycle ceux des entités détruites.
#[derive(Debug, Default, Clone)]
pub struct Entities {
    /// La génération actuelle de chaque case.
    generations: Vec<u32>,
    /// Vrai si la case est occupée par une entité vivante.
    alive: Vec<bool>,
    /// Les cases libres, à réutiliser avant d'en créer de nouvelles.
    free: Vec<u32>,
    count: usize,
}

impl Entities {
    pub fn allocate(&mut self) -> Entity {
        self.count += 1;
        if let Some(index) = self.free.pop() {
            self.alive[index as usize] = true;
            return Entity {
                index,
                generation: self.generations[index as usize],
            };
        }
        let index = self.generations.len() as u32;
        self.generations.push(0);
        self.alive.push(true);
        Entity {
            index,
            generation: 0,
        }
    }

    /// Libère la case. Renvoie `false` si l'entité était déjà morte (ou périmée).
    pub fn free(&mut self, entity: Entity) -> bool {
        if !self.is_alive(entity) {
            return false;
        }
        let i = entity.index as usize;
        self.alive[i] = false;
        // `wrapping_add` : après 4 milliards de réutilisations, on repart de 0 sans paniquer.
        self.generations[i] = self.generations[i].wrapping_add(1);
        self.free.push(entity.index);
        self.count -= 1;
        true
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        let i = entity.index as usize;
        i < self.alive.len() && self.alive[i] && self.generations[i] == entity.generation
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Toutes les entités vivantes.
    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.alive
            .iter()
            .enumerate()
            .filter(|(_, alive)| **alive)
            .map(|(i, _)| Entity {
                index: i as u32,
                generation: self.generations[i],
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_entity_does_not_find_the_new_one_in_its_slot() {
        let mut entities = Entities::default();
        let old = entities.allocate();
        assert!(entities.free(old));
        let new = entities.allocate();
        // Même case, mais génération différente.
        assert_eq!(new.index, old.index);
        assert_ne!(new.generation, old.generation);
        assert!(!entities.is_alive(old));
        assert!(entities.is_alive(new));
        // Libérer deux fois ne fait rien.
        assert!(!entities.free(old));
        assert_eq!(entities.len(), 1);
    }
}
