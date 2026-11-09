//! Le monde de l'ECS : les entités, leurs composants, les ressources et les commandes.
//!
//! # Un stockage par type, rangé par `TypeId`
//!
//! Le monde ne sait pas à l'avance quels types de composants existeront (le jeu, puis
//! les mods, en ajoutent). Il garde donc une table `TypeId → Box<dyn ComponentStorage>` :
//! `TypeId::of::<Position>()` est un identifiant unique du type `Position`, calculé par
//! le compilateur. Pour retrouver un `SparseSet<Position>` à partir de la boîte, on
//! « descend » (`downcast`) depuis `dyn Any` : vérifié à l'exécution, sans `unsafe`.
//!
//! # Pourquoi un `RefCell` par stockage ?
//!
//! Une requête `(&Position, &mut Velocity)` emprunte deux stockages en même temps, l'un
//! en lecture, l'autre en écriture. Le compilateur ne sait pas prouver, à travers une
//! `HashMap`, que ce sont deux cases différentes : il refuserait deux emprunts de la
//! table, dont un mutable. `RefCell` déplace cette vérification à l'exécution : chaque
//! stockage compte ses emprunts, et panique si on essaie d'en emprunter un en écriture
//! deux fois. On garde ainsi la sûreté de Rust sans écrire d'`unsafe`, pour le prix
//! d'un compteur par stockage (pas par composant).
//!
//! Un vrai ECS comme `hecs` ou `bevy_ecs` évite même ce compteur avec du code `unsafe`
//! soigneusement vérifié. Le benchmark (`benches/ecs.rs`) mesure ce que ça nous coûte.

use std::any::{Any, TypeId};
use std::cell::{Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use crate::entity::{Entities, Entity};
use crate::query::{Query, QueryParam};
use crate::storage::{ComponentStorage, SparseSet};

/// Une fonction de hachage « qui ne fait rien » pour les `TypeId`.
///
/// Un `TypeId` est déjà un nombre pseudo-aléatoire calculé par le compilateur : le
/// re-hacher avec SipHash (la fonction par défaut, lente mais résistante aux attaques)
/// ne sert à rien. Ce hacheur garde simplement les bits qu'on lui donne. Implémenter le
/// trait `Hasher` suffit pour que la `HashMap` standard l'utilise.
#[derive(Default)]
pub struct TypeIdHasher(u64);

impl Hasher for TypeIdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Au cas où le `TypeId` écrirait des octets : on les replie sur 64 bits.
        for chunk in bytes.chunks(8) {
            let mut word = [0u8; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.0 = self.0.rotate_left(5) ^ u64::from_le_bytes(word);
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = self.0.rotate_left(5) ^ value;
    }
}

/// Une table indexée par type, avec le hacheur ci-dessus.
type TypeMap<V> = HashMap<TypeId, V, BuildHasherDefault<TypeIdHasher>>;

/// Une action à faire plus tard, quand le monde n'est plus emprunté.
type Command = Box<dyn FnOnce(&mut World)>;

#[derive(Default)]
pub struct World {
    entities: Entities,
    storages: TypeMap<RefCell<Box<dyn ComponentStorage>>>,
    resources: TypeMap<RefCell<Box<dyn Any>>>,
    commands: RefCell<Vec<Command>>,
}

impl World {
    pub fn new() -> Self {
        Self::default()
    }

    // ----- Entités -----

    /// Crée une entité avec un ensemble de composants : `world.spawn((Position(..), Velocity(..)))`.
    pub fn spawn(&mut self, bundle: impl Bundle) -> Entity {
        let entity = self.entities.allocate();
        bundle.insert_into(self, entity);
        entity
    }

    /// Détruit une entité et tous ses composants. `false` si elle n'existait plus.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.free(entity) {
            return false;
        }
        // `get_mut` sur un `RefCell` : on a `&mut self`, aucun besoin de compter les emprunts.
        for storage in self.storages.values_mut() {
            storage.get_mut().remove_entity(entity);
        }
        true
    }

    pub fn is_alive(&self, entity: Entity) -> bool {
        self.entities.is_alive(entity)
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter()
    }

    // ----- Composants -----

    /// Le stockage de `T`, créé au premier usage.
    fn storage_mut<T: 'static>(&mut self) -> &mut SparseSet<T> {
        self.storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| RefCell::new(Box::new(SparseSet::<T>::default())))
            .get_mut()
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()
            .expect("le stockage d'un TypeId a toujours ce type")
    }

    /// Ajoute (ou remplace) un composant. Ne fait rien si l'entité est morte.
    pub fn insert<T: 'static>(&mut self, entity: Entity, component: T) -> Option<T> {
        if !self.is_alive(entity) {
            return None;
        }
        self.storage_mut::<T>().insert(entity, component)
    }

    pub fn remove<T: 'static>(&mut self, entity: Entity) -> Option<T> {
        self.storages
            .get_mut(&TypeId::of::<T>())?
            .get_mut()
            .as_any_mut()
            .downcast_mut::<SparseSet<T>>()?
            .remove(entity)
    }

    /// Emprunte le stockage de `T` en lecture (pour les requêtes).
    pub(crate) fn borrow_storage<T: 'static>(&self) -> Option<Ref<'_, SparseSet<T>>> {
        let cell = self.storages.get(&TypeId::of::<T>())?;
        let borrowed = cell.try_borrow().unwrap_or_else(|_| {
            panic!(
                "{} est déjà emprunté en écriture",
                std::any::type_name::<T>()
            )
        });
        // `Ref::map` transforme l'emprunt de la boîte en emprunt du type concret.
        Some(Ref::map(borrowed, |b| {
            b.as_any()
                .downcast_ref::<SparseSet<T>>()
                .expect("le stockage d'un TypeId a toujours ce type")
        }))
    }

    /// Emprunte le stockage de `T` en écriture (pour les requêtes).
    pub(crate) fn borrow_storage_mut<T: 'static>(&self) -> Option<RefMut<'_, SparseSet<T>>> {
        let cell = self.storages.get(&TypeId::of::<T>())?;
        let borrowed = cell
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("{} est déjà emprunté", std::any::type_name::<T>()));
        Some(RefMut::map(borrowed, |b| {
            b.as_any_mut()
                .downcast_mut::<SparseSet<T>>()
                .expect("le stockage d'un TypeId a toujours ce type")
        }))
    }

    /// Le composant `T` d'une entité, en lecture.
    pub fn get<T: 'static>(&self, entity: Entity) -> Option<Ref<'_, T>> {
        let storage = self.borrow_storage::<T>()?;
        // `filter_map` : garde l'emprunt seulement si l'entité a ce composant.
        Ref::filter_map(storage, |s| s.get(entity)).ok()
    }

    /// Le composant `T` d'une entité, en écriture.
    pub fn get_mut<T: 'static>(&self, entity: Entity) -> Option<RefMut<'_, T>> {
        let storage = self.borrow_storage_mut::<T>()?;
        RefMut::filter_map(storage, |s| s.get_mut(entity)).ok()
    }

    pub fn has<T: 'static>(&self, entity: Entity) -> bool {
        self.borrow_storage::<T>()
            .is_some_and(|s| s.get(entity).is_some())
    }

    /// Les noms des types de composants d'une entité (pour l'inspecteur).
    pub fn component_names(&self, entity: Entity) -> Vec<&'static str> {
        let mut names: Vec<&'static str> = self
            .storages
            .values()
            .filter_map(|cell| {
                let storage = cell.borrow();
                storage.contains(entity).then(|| storage.type_name())
            })
            .collect();
        names.sort_unstable();
        names
    }

    /// Une requête : `world.query::<(&Position, &mut Velocity)>()`.
    ///
    /// Panique si la requête demande deux fois le même type dont au moins une fois en
    /// écriture (`(&mut A, &A)`) : ce serait deux accès simultanés à la même donnée,
    /// dont un en écriture, exactement ce que Rust interdit.
    pub fn query<Q: QueryParam>(&self) -> Query<'_, Q> {
        let mut access = Vec::new();
        Q::access(&mut access);
        for (i, (type_a, name, mut_a)) in access.iter().enumerate() {
            for (type_b, _, mut_b) in &access[i + 1..] {
                if type_a == type_b && (*mut_a || *mut_b) {
                    panic!("requête invalide : {name} demandé deux fois, dont une en écriture");
                }
            }
        }
        Query::new(Q::borrow(self))
    }

    // ----- Ressources : des valeurs uniques, rangées par type (l'heure, le registre...) -----

    pub fn insert_resource<R: 'static>(&mut self, resource: R) {
        self.resources
            .insert(TypeId::of::<R>(), RefCell::new(Box::new(resource)));
    }

    pub fn get_resource<R: 'static>(&self) -> Option<Ref<'_, R>> {
        let cell = self.resources.get(&TypeId::of::<R>())?;
        Ref::filter_map(cell.borrow(), |b| b.downcast_ref::<R>()).ok()
    }

    pub fn get_resource_mut<R: 'static>(&self) -> Option<RefMut<'_, R>> {
        let cell = self.resources.get(&TypeId::of::<R>())?;
        RefMut::filter_map(cell.borrow_mut(), |b| b.downcast_mut::<R>()).ok()
    }

    /// La ressource `R` ; panique si elle n'a pas été ajoutée (une erreur de programmation).
    pub fn resource<R: 'static>(&self) -> Ref<'_, R> {
        self.get_resource::<R>()
            .unwrap_or_else(|| panic!("ressource {} absente", std::any::type_name::<R>()))
    }

    pub fn resource_mut<R: 'static>(&self) -> RefMut<'_, R> {
        self.get_resource_mut::<R>()
            .unwrap_or_else(|| panic!("ressource {} absente", std::any::type_name::<R>()))
    }

    // ----- Commandes différées -----

    /// Les commandes : créer ou détruire des entités pendant une requête est impossible
    /// (le monde est emprunté). On les note ici ; elles s'appliquent dans `apply_commands`.
    pub fn commands(&self) -> Commands<'_> {
        Commands {
            queue: self.commands.borrow_mut(),
        }
    }

    pub fn apply_commands(&mut self) {
        // `take` vide la file avant de l'exécuter : une commande peut en ajouter d'autres.
        let commands = std::mem::take(self.commands.get_mut());
        for command in commands {
            command(self);
        }
    }
}

/// Une file de commandes, empruntée au monde.
pub struct Commands<'w> {
    queue: RefMut<'w, Vec<Command>>,
}

impl Commands<'_> {
    pub fn spawn(&mut self, bundle: impl Bundle + 'static) {
        self.queue.push(Box::new(move |world| {
            world.spawn(bundle);
        }));
    }

    pub fn despawn(&mut self, entity: Entity) {
        self.queue.push(Box::new(move |world| {
            world.despawn(entity);
        }));
    }

    pub fn insert<T: 'static>(&mut self, entity: Entity, component: T) {
        self.queue.push(Box::new(move |world| {
            world.insert(entity, component);
        }));
    }

    pub fn remove<T: 'static>(&mut self, entity: Entity) {
        self.queue.push(Box::new(move |world| {
            world.remove::<T>(entity);
        }));
    }

    /// N'importe quelle action sur le monde.
    pub fn add(&mut self, command: impl FnOnce(&mut World) + 'static) {
        self.queue.push(Box::new(command));
    }
}

/// Un ensemble de composants à ajouter d'un coup (un tuple).
pub trait Bundle {
    fn insert_into(self, world: &mut World, entity: Entity);
}

/// `macro_rules!` écrit le code répétitif à notre place : l'implémentation de `Bundle`
/// pour les tuples de 1 à 8 composants. `$name` parcourt les lettres données.
macro_rules! impl_bundle {
    ($($name:ident),+) => {
        impl<$($name: 'static),+> Bundle for ($($name,)+) {
            #[allow(non_snake_case)]
            fn insert_into(self, world: &mut World, entity: Entity) {
                let ($($name,)+) = self;
                $( world.insert(entity, $name); )+
            }
        }
    };
}

impl_bundle!(A);
impl_bundle!(A, B);
impl_bundle!(A, B, C);
impl_bundle!(A, B, C, D);
impl_bundle!(A, B, C, D, E);
impl_bundle!(A, B, C, D, E, F);
impl_bundle!(A, B, C, D, E, F, G);
impl_bundle!(A, B, C, D, E, F, G, H);
