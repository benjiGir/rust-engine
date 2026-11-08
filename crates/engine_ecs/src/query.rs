//! Les requêtes : « toutes les entités qui ont une Position et une Velocity ».
//!
//! # Un trait générique pour chaque élément de la requête
//!
//! `&T` lit le stockage de `T`, `&mut T` l'écrit. Un tuple `(&A, &mut B)` combine les
//! deux. Chaque forme implémente [`QueryParam`], qui dit quoi emprunter (`borrow`) et
//! comment lire un élément (`fetch`). Les implémentations pour les tuples de 1 à 8
//! éléments sont écrites par une macro.
//!
//! Les types associés génériques (`type Item<'g>`) disent : « l'élément prêté vit aussi
//! longtemps que l'emprunt `'g` du stockage ». C'est ce qui empêche de garder un
//! `&mut Position` après la fin de la requête.
//!
//! # Pourquoi `for_each` et pas un itérateur ?
//!
//! Un `Iterator` doit pouvoir rendre plusieurs éléments vivants en même temps
//! (`let a = it.next(); let b = it.next();`). Avec des `&mut` tirés au hasard d'un
//! stockage (`get_mut(entité)`), le compilateur ne peut pas prouver que `a` et `b` ne
//! désignent pas la même donnée. `for_each` passe chaque élément à une closure, et
//! l'emprunt se termine avant l'élément suivant : c'est prouvable, donc sûr, sans
//! `unsafe`. (`hecs` y arrive avec un itérateur grâce à du code `unsafe` vérifié.)

use std::any::TypeId;
use std::cell::{Ref, RefMut};

use crate::entity::Entity;
use crate::storage::SparseSet;
use crate::world::World;

/// Un élément de requête (`&T`, `&mut T`) ou un tuple d'éléments.
pub trait QueryParam {
    /// Les emprunts tenus pendant la requête.
    type Guard<'w>;
    /// Ce que reçoit la closure pour une entité.
    type Item<'g>;
    /// Les types accédés et si c'est en écriture, pour refuser `(&mut A, &A)`.
    fn access(out: &mut Vec<(TypeId, &'static str, bool)>);
    /// `None` si un des stockages n'existe pas (aucune entité n'a ce composant).
    fn borrow(world: &World) -> Option<Self::Guard<'_>>;
    /// Les entités à parcourir : celles du plus petit stockage de la requête.
    fn driver(guard: &Self::Guard<'_>) -> (usize, Vec<Entity>);
    fn fetch<'g>(guard: &'g mut Self::Guard<'_>, entity: Entity) -> Option<Self::Item<'g>>;
}

impl<T: 'static> QueryParam for &T {
    type Guard<'w> = Ref<'w, SparseSet<T>>;
    type Item<'g> = &'g T;

    fn access(out: &mut Vec<(TypeId, &'static str, bool)>) {
        out.push((TypeId::of::<T>(), std::any::type_name::<T>(), false));
    }

    fn borrow(world: &World) -> Option<Self::Guard<'_>> {
        world.borrow_storage::<T>()
    }

    fn driver(guard: &Self::Guard<'_>) -> (usize, Vec<Entity>) {
        (guard.entities().len(), guard.entities().to_vec())
    }

    fn fetch<'g>(guard: &'g mut Self::Guard<'_>, entity: Entity) -> Option<Self::Item<'g>> {
        guard.get(entity)
    }
}

impl<T: 'static> QueryParam for &mut T {
    type Guard<'w> = RefMut<'w, SparseSet<T>>;
    type Item<'g> = &'g mut T;

    fn access(out: &mut Vec<(TypeId, &'static str, bool)>) {
        out.push((TypeId::of::<T>(), std::any::type_name::<T>(), true));
    }

    fn borrow(world: &World) -> Option<Self::Guard<'_>> {
        world.borrow_storage_mut::<T>()
    }

    fn driver(guard: &Self::Guard<'_>) -> (usize, Vec<Entity>) {
        (guard.entities().len(), guard.entities().to_vec())
    }

    fn fetch<'g>(guard: &'g mut Self::Guard<'_>, entity: Entity) -> Option<Self::Item<'g>> {
        guard.get_mut(entity)
    }
}

/// Les tuples : chaque élément emprunte son stockage ; `fetch` réussit si l'entité a
/// tous les composants. Les champs d'un tuple sont des emprunts distincts : on peut
/// prendre `&mut guard.0` et `&mut guard.1` en même temps.
macro_rules! impl_query_tuple {
    ($($name:ident $index:tt),+) => {
        impl<$($name: QueryParam),+> QueryParam for ($($name,)+) {
            type Guard<'w> = ($($name::Guard<'w>,)+);
            type Item<'g> = ($($name::Item<'g>,)+);

            fn access(out: &mut Vec<(TypeId, &'static str, bool)>) {
                $( $name::access(out); )+
            }

            fn borrow(world: &World) -> Option<Self::Guard<'_>> {
                Some(($($name::borrow(world)?,)+))
            }

            fn driver(guard: &Self::Guard<'_>) -> (usize, Vec<Entity>) {
                // On parcourt le plus petit stockage : inutile de tester 10 000 entités
                // s'il n'y en a que 3 qui ont le composant le plus rare.
                let mut best: Option<(usize, Vec<Entity>)> = None;
                $(
                    let candidate = $name::driver(&guard.$index);
                    if best.as_ref().is_none_or(|b| candidate.0 < b.0) {
                        best = Some(candidate);
                    }
                )+
                best.unwrap_or_default()
            }

            fn fetch<'g>(guard: &'g mut Self::Guard<'_>, entity: Entity) -> Option<Self::Item<'g>> {
                Some(($($name::fetch(&mut guard.$index, entity)?,)+))
            }
        }
    };
}

impl_query_tuple!(A 0);
impl_query_tuple!(A 0, B 1);
impl_query_tuple!(A 0, B 1, C 2);
impl_query_tuple!(A 0, B 1, C 2, D 3);
impl_query_tuple!(A 0, B 1, C 2, D 3, E 4);
impl_query_tuple!(A 0, B 1, C 2, D 3, E 4, F 5);
impl_query_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6);
impl_query_tuple!(A 0, B 1, C 2, D 3, E 4, F 5, G 6, H 7);

/// Une requête en cours : elle tient les emprunts des stockages jusqu'à sa destruction.
pub struct Query<'w, Q: QueryParam> {
    guard: Option<Q::Guard<'w>>,
}

impl<'w, Q: QueryParam> Query<'w, Q> {
    pub(crate) fn new(guard: Option<Q::Guard<'w>>) -> Self {
        Self { guard }
    }

    /// Appelle `f` pour chaque entité qui a tous les composants demandés.
    pub fn for_each(&mut self, mut f: impl FnMut(Entity, Q::Item<'_>)) {
        let Some(guard) = self.guard.as_mut() else {
            return;
        };
        let (_, entities) = Q::driver(guard);
        for entity in entities {
            if let Some(item) = Q::fetch(guard, entity) {
                f(entity, item);
            }
        }
    }

    /// Les composants d'une seule entité.
    pub fn get(&mut self, entity: Entity) -> Option<Q::Item<'_>> {
        Q::fetch(self.guard.as_mut()?, entity)
    }

    /// Les entités qui correspondent.
    pub fn entities(&mut self) -> Vec<Entity> {
        let mut found = Vec::new();
        self.for_each(|entity, _| found.push(entity));
        found
    }

    pub fn count(&mut self) -> usize {
        let mut n = 0;
        self.for_each(|_, _| n += 1);
        n
    }
}
