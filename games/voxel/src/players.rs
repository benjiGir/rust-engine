//! Les joueurs : ce qui appartient à un joueur, et la table qui les range.
//!
//! Le jeu n'a longtemps eu qu'un seul joueur, dont l'inventaire, la faim, l'équipement... étaient
//! des champs de `VoxelGame`. Pour que le monde accueille un jour plusieurs joueurs, tout cela est
//! regroupé dans un [`PlayerState`], et le jeu garde une table [`Players`]. Le joueur de la
//! fenêtre (le « local ») est l'un d'eux ; `players.me()` donne son état.
//!
//! Ce qui reste dans `VoxelGame` est ce qui est propre à la **fenêtre** (caméra, interface, bras,
//! son) ou au **monde** (heure, météo, fluides, villages).
//!
//! L'entité ECS de chaque joueur porte le composant [`PlayerId`], qui permet de remonter de
//! l'entité à son état ; la ressource [`PlayerEntities`](crate::mobs::PlayerEntities) liste ces
//! entités pour les créatures (voir `mobs.rs`).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};

use engine_ecs::Entity;
use glam::IVec3;

use crate::adventure::Adventure;
use crate::gear::Gear;
use crate::inventory::Inventory;
use crate::potions::Effects;
use crate::quest_play::QuestBook;
use crate::survival::{Experience, Hunger, HungerRules, SurvivalRules, Vitals};

/// Le numéro d'un joueur dans la partie en cours. Il n'est jamais écrit sur disque (c'est
/// l'identifiant [`uuid`](PlayerState::uuid) qui reconnaît un joueur d'une séance à l'autre) ; il
/// sert aussi de composant sur l'entité ECS du joueur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// Qui est le joueur : son numéro dans la partie, son nom et son identifiant stable.
#[derive(Debug, Clone)]
pub struct PlayerIdentity {
    pub id: PlayerId,
    pub name: String,
    pub uuid: u128,
}

/// Le dernier joueur qui a frappé une créature : à sa mort, c'est lui le tueur (voir
/// `EntityDied::killer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastHitBy(pub PlayerId);

/// Tout ce qui appartient à un joueur.
pub struct PlayerState {
    pub id: PlayerId,
    /// Le nom affiché (venu de `settings.ron` pour le joueur local).
    pub name: String,
    /// L'identifiant stable : le fichier `players/<uuid>.ron` porte son écriture hexadécimale.
    pub uuid: u128,
    /// L'entité du joueur dans l'ECS.
    pub entity: Entity,
    pub inventory: Inventory,
    /// L'air, le feu, la chute : ce qui le blesse.
    pub vitals: Vitals,
    pub hunger: Hunger,
    /// Ses attributs, calculés depuis son équipement (voir `gear.rs`).
    pub gear: Gear,
    pub effects: Effects,
    /// Le palier d'aventure et la progression du joueur (voir `adventure.rs`).
    pub adventure: Adventure,
    /// Le livre de quêtes et sa progression (voir `quest_play.rs`).
    pub quest_book: QuestBook,
    pub experience: Experience,
    /// Le lit où réapparaître, s'il y en a un.
    pub bed: Option<IVec3>,
    /// Le bloc en cours de minage (bouton gauche maintenu) et l'avancement, de 0 à 1.
    pub mining: Option<(IVec3, f32)>,
    /// Le bouton gauche est tenu sans rien à casser : le bras frappe quand même, en boucle.
    pub air_swing: bool,
    /// L'avancement du repas en cours (0 à 1), pour la barre.
    pub eat_progress: Option<f32>,
    /// Le bouton de repas est tenu (message `Eat`) : le simulateur mâche à chaque pas.
    pub eating: bool,
    /// Mort, en attente de réapparaître : sa mort a déjà été traitée (objets lâchés, message).
    pub dead: bool,
    /// L'écran que le serveur a ouvert pour ce joueur (ses gestes ne valent que pour lui).
    pub screen: Option<crate::ui::Screen>,
    /// Le dernier regard connu du joueur (direction, d'après ses messages `Move`).
    pub look: glam::Vec3,
    /// Son orientation (`ClientMsg::Look`), pour la sauvegarde.
    pub yaw: f32,
    pub pitch: f32,
    /// Le menu de mod ouvert par ce joueur (voir `mod_menu.rs`).
    pub mod_menu: Option<crate::mod_menu::OpenModMenu>,
    /// Le conteneur ouvert par ce joueur (coffre, four, sac, wagon) : les clics de cases ne valent
    /// que pour lui (voir `intents.rs`).
    pub open: Option<OpenContainer>,
    /// Le minage que le simulateur a vu commencer, et son avancement théorique (le contrôle du
    /// message `Dig Finish`).
    pub dig: Option<DigState>,
    /// Le dernier mouvement accepté, pour contrôler le suivant (voir `movement.rs`).
    pub last_move: Option<crate::movement::MoveSample>,
    /// Les pieds sur lesquels le serveur et la fenêtre de ce joueur sont d'accord (le dernier `Move`
    /// rangé, ou le dernier `Correct`) : ailleurs, c'est que le serveur l'a déplacé (voir
    /// `predicted.rs`). `None` : le premier `Move` fera foi.
    pub synced: Option<glam::Vec3>,
    /// Le dernier `Correct` disait le joueur porté (wagon, structure).
    pub carried: bool,
    /// Un joueur du serveur sans fenêtre (`ctl ghost`, voir `ghosts.rs`) : il n'est jamais sauvegardé.
    pub ghost: bool,
}

/// Un conteneur ouvert devant un joueur : ce que `SlotClick` peut viser en dehors de son inventaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenContainer {
    /// Un coffre (de n'importe quel palier), par la position du bloc qui porte l'inventaire (la
    /// moitié gauche d'un grand coffre).
    Chest(IVec3),
    /// Un four ou un alambic.
    Machine(IVec3),
    /// Le contrôleur de stockage.
    Controller(IVec3),
    /// L'inventaire d'un wagon.
    Cart(Entity),
    /// Un sac à dos, par son numéro.
    Bag(u32),
}

/// Le minage d'un joueur vu du simulateur : le bloc, et la part du temps de minage déjà écoulée
/// (de 0 à 1, comptée en pas fixes avec l'outil tenu à chaque pas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigState {
    pub pos: IVec3,
    pub block: engine_voxel::BlockId,
    pub progress: f32,
}

impl PlayerState {
    /// Un joueur neuf : l'inventaire et l'équipement donnés, le reste à son état de départ.
    pub fn new(
        who: PlayerIdentity,
        entity: Entity,
        inventory: Inventory,
        gear: Gear,
        survival: &SurvivalRules,
        hunger: &HungerRules,
    ) -> Self {
        Self {
            id: who.id,
            name: who.name,
            uuid: who.uuid,
            entity,
            inventory,
            vitals: Vitals::new(survival),
            hunger: Hunger::new(hunger),
            gear,
            effects: Effects::default(),
            adventure: Adventure::default(),
            quest_book: QuestBook::default(),
            experience: Experience::default(),
            bed: None,
            mining: None,
            air_swing: false,
            eat_progress: None,
            eating: false,
            dead: false,
            screen: None,
            mod_menu: None,
            look: glam::Vec3::NEG_Z,
            yaw: 0.0,
            pitch: 0.0,
            open: None,
            dig: None,
            last_move: None,
            synced: None,
            carried: false,
            ghost: false,
        }
    }

    /// Reprend ce qu'un joueur avait sauvegardé : expérience, faim, lit.
    pub fn apply_save(&mut self, saved: &engine_voxel::PlayerSave) {
        self.experience.points = saved.xp.unwrap_or(0);
        self.hunger.food = saved.food.unwrap_or(self.hunger.food);
        self.hunger.saturation = saved.saturation.unwrap_or(self.hunger.saturation);
        self.bed = saved.spawn_point.map(IVec3::from);
    }

    /// Le dossier où ce joueur range sa progression de quêtes. Le joueur hôte (`host`) garde
    /// `quests.ron` à la racine du monde, comme avant les joueurs multiples ; les autres ont leur
    /// sous-dossier de `players/`.
    pub fn quest_dir(&self, world_dir: &Path, host: bool) -> PathBuf {
        if host {
            world_dir.to_path_buf()
        } else {
            world_dir.join("players").join(uuid_name(self.uuid))
        }
    }
}

/// L'écriture d'un identifiant : 32 chiffres hexadécimaux (le nom du fichier du joueur).
pub fn uuid_name(uuid: u128) -> String {
    format!("{uuid:032x}")
}

/// Relit un identifiant écrit par [`uuid_name`] ; `None` s'il est mal formé.
pub fn parse_uuid(text: &str) -> Option<u128> {
    u128::from_str_radix(text.trim(), 16).ok()
}

/// Donne un identifiant au joueur local s'il n'en a pas (ou s'il est illisible). Renvoie `true`
/// quand il vient d'en tirer un : il faut alors écrire `settings.ron`, pour que le joueur retrouve
/// son fichier d'un lancement à l'autre.
pub fn ensure_identity(settings: &mut crate::settings::Settings) -> bool {
    if settings.player_uuid.len() == 32 && parse_uuid(&settings.player_uuid).is_some() {
        return false;
    }
    settings.player_uuid = uuid_name(random_uuid());
    true
}

/// Un identifiant tiré au hasard (pour un joueur qui n'en a pas encore).
pub fn random_uuid() -> u128 {
    let mut parts = [0u64; 2];
    for part in &mut parts {
        // Chaque `RandomState` démarre avec ses propres clés tirées par le système.
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(
            std::time::UNIX_EPOCH
                .elapsed()
                .map_or(0, |d| d.as_nanos() as u64),
        );
        *part = hasher.finish();
    }
    (u128::from(parts[0]) << 64) | u128::from(parts[1])
}

/// Les joueurs de la partie, et lequel est devant la fenêtre.
pub struct Players {
    list: Vec<PlayerState>,
    local: PlayerId,
    next_id: u32,
    /// Le joueur dont le simulateur traite un message (voir `intents.rs`) : tant qu'il est là,
    /// `me()` est lui, et plus le joueur de la fenêtre.
    acting: Option<PlayerId>,
}

// Les arrivées et départs de joueurs ne servent encore qu'aux tests : le premier joueur distant
// arrive avec le réseau (lot 5), la table est prête pour lui.
#[allow(dead_code)]
impl Players {
    /// La table avec son premier joueur, qui est le joueur local.
    pub fn new(local: PlayerState) -> Self {
        let id = local.id;
        Self {
            next_id: id.0 + 1,
            local: id,
            list: vec![local],
            acting: None,
        }
    }

    /// Le prochain numéro libre.
    pub fn next_id(&mut self) -> PlayerId {
        let id = PlayerId(self.next_id);
        self.next_id += 1;
        id
    }

    pub fn local_id(&self) -> PlayerId {
        self.local
    }

    /// Le joueur « courant » : celui dont `handle` traite un message, sinon celui de la fenêtre.
    /// Tout ce que le simulateur appelle depuis un message agit donc sur le bon joueur sans le
    /// recevoir en argument ; l'interface, qui ne tourne jamais pendant un message, voit le joueur
    /// de la fenêtre.
    pub fn me(&self) -> &PlayerState {
        self.get(self.current_id())
            .expect("le joueur courant existe")
    }

    pub fn me_mut(&mut self) -> &mut PlayerState {
        let id = self.current_id();
        self.get_mut(id).expect("le joueur courant existe")
    }

    /// Le numéro du joueur courant (voir [`Players::me`]).
    pub fn current_id(&self) -> PlayerId {
        self.acting.unwrap_or(self.local)
    }

    /// Le joueur courant est celui de la fenêtre : les retours d'interface (bras, écran ouvert,
    /// message) ne valent que pour lui.
    pub fn current_is_local(&self) -> bool {
        self.current_id() == self.local
    }

    /// Désigne le joueur courant ; renvoie le précédent, à rendre ensuite.
    pub fn set_acting(&mut self, who: Option<PlayerId>) -> Option<PlayerId> {
        std::mem::replace(&mut self.acting, who)
    }

    /// Le joueur de la fenêtre, quel que soit le message en cours.
    pub fn local(&self) -> &PlayerState {
        self.get(self.local).expect("le joueur local existe")
    }

    pub fn local_mut(&mut self) -> &mut PlayerState {
        let id = self.local;
        self.get_mut(id).expect("le joueur local existe")
    }

    /// L'entité du joueur courant (un raccourci qui n'emprunte que la table).
    pub fn entity(&self) -> Entity {
        self.me().entity
    }

    pub fn get(&self, id: PlayerId) -> Option<&PlayerState> {
        self.list.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: PlayerId) -> Option<&mut PlayerState> {
        self.list.iter_mut().find(|p| p.id == id)
    }

    /// Le joueur qui possède cette entité ECS.
    pub fn by_entity(&self, entity: Entity) -> Option<&PlayerState> {
        self.list.iter().find(|p| p.entity == entity)
    }

    pub fn by_uuid(&self, uuid: u128) -> Option<&PlayerState> {
        self.list.iter().find(|p| p.uuid == uuid)
    }

    pub fn iter(&self) -> impl Iterator<Item = &PlayerState> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut PlayerState> {
        self.list.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// Ajoute un joueur (le numéro doit venir de [`Players::next_id`]).
    pub fn add(&mut self, player: PlayerState) {
        debug_assert!(self.get(player.id).is_none(), "numéro de joueur déjà pris");
        self.list.push(player);
    }

    /// Retire un joueur. Le joueur local ne peut pas partir.
    pub fn remove(&mut self, id: PlayerId) -> Option<PlayerState> {
        if id == self.local {
            return None;
        }
        let at = self.list.iter().position(|p| p.id == id)?;
        Some(self.list.remove(at))
    }

    /// Les entités de tous les joueurs : le local en premier, puis les autres dans l'ordre d'arrivée.
    pub fn entities(&self) -> Vec<Entity> {
        let local = self.local;
        std::iter::once(self.local().entity)
            .chain(self.list.iter().filter(|p| p.id != local).map(|p| p.entity))
            .collect()
    }

    /// Dit à l'ECS qui sont les joueurs : le composant [`PlayerId`] de chaque entité et la
    /// ressource [`PlayerEntities`](crate::mobs::PlayerEntities) que lisent les créatures. À
    /// appeler à chaque arrivée ou départ.
    pub fn sync_ecs(&self, world: &mut engine_ecs::World) {
        for player in &self.list {
            world.insert(player.entity, player.id);
        }
        world.insert_resource(crate::mobs::PlayerEntities(self.entities()));
    }
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Les centres de chargement du monde du serveur : un par joueur. Le serveur ne maille pas : le
    /// maillage de la fenêtre se fait dans sa réplique (voir `client_world.rs`), qui suit la caméra.
    pub(crate) fn stream_views(&self) -> Vec<engine_voxel::View> {
        self.players
            .iter()
            // Le regard de chaque joueur (ses messages `Look`) : on charge devant lui d'abord.
            .map(|p| engine_voxel::View {
                position: crate::player::eye(&self.ecs, p.entity, 1.0),
                forward: p.look,
                mesh: false,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use engine_ecs::{Events, World};
    use glam::{IVec3, Vec3};

    use super::*;
    use crate::attributes::BaseAttributes;
    use crate::components::{
        Collider, Flying, Health, Mob, MoveIntent, PreviousPosition, Transform, Velocity,
    };
    use crate::events::PlayerAttacked;
    use crate::mobs::{self, MobRegistry, PlayerEntities, SimClock, spawn_mob};

    /// Un joueur sans fenêtre : son entité (au sol, pas en vol) et son état.
    fn add_player(world: &mut World, players: &mut Players, at: Vec3) -> PlayerId {
        let entity = world.spawn((
            Transform {
                position: at,
                yaw: 0.0,
            },
            PreviousPosition(at),
            Velocity::default(),
            Collider::new(0.6, 1.8),
            MoveIntent::default(),
            Health::full(20.0),
            crate::components::Player,
        ));
        let id = players.next_id();
        let state = PlayerState::new(
            PlayerIdentity {
                id,
                name: format!("joueur {}", id.0),
                uuid: random_uuid(),
            },
            entity,
            Inventory::default(),
            Gear::new(BaseAttributes::default()),
            &SurvivalRules::default(),
            &HungerRules::default(),
        );
        players.add(state);
        players.sync_ecs(world);
        id
    }

    /// Une table à un joueur (le local), dans un monde ECS de test chargé autour de `center`.
    fn two_player_world(center: Vec3, a: Vec3, b: Vec3) -> (World, Players, PlayerId, PlayerId) {
        let mut world = mobs::tests::sim_at(center, 5);
        // Le joueur que `sim_at` a créé n'est pas dans la table : on le retire.
        let spare = world.resource::<PlayerEntities>().0[0];
        world.despawn(spare);
        // La table démarre avec son premier joueur, créé à la main comme les suivants.
        let first = world.spawn((
            Transform {
                position: a,
                yaw: 0.0,
            },
            PreviousPosition(a),
            Velocity::default(),
            Collider::new(0.6, 1.8),
            MoveIntent::default(),
            Health::full(20.0),
            crate::components::Player,
        ));
        let local = PlayerState::new(
            PlayerIdentity {
                id: PlayerId(0),
                name: "hôte".into(),
                uuid: random_uuid(),
            },
            first,
            Inventory::default(),
            Gear::new(BaseAttributes::default()),
            &SurvivalRules::default(),
            &HungerRules::default(),
        );
        let mut players = Players::new(local);
        players.sync_ecs(&mut world);
        let second = add_player(&mut world, &mut players, b);
        (world, players, PlayerId(0), second)
    }

    #[test]
    fn uuids_round_trip_through_their_file_name() {
        let uuid = random_uuid();
        let name = uuid_name(uuid);
        assert_eq!(name.len(), 32);
        assert_eq!(parse_uuid(&name), Some(uuid));
        assert_eq!(parse_uuid("pas un identifiant"), None);
    }

    #[test]
    fn random_uuids_differ() {
        assert_ne!(random_uuid(), random_uuid());
    }

    #[test]
    fn the_table_keeps_the_local_player_first_and_their_inventories_apart() {
        let (mut world, mut players, a, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.0, 70.0, 0.0),
            Vec3::new(30.0, 70.0, 0.0),
        );
        assert_eq!(players.len(), 2);
        assert_eq!(players.local_id(), a);
        let entities = world.resource::<PlayerEntities>().0.clone();
        assert_eq!(entities, players.entities());
        assert_eq!(entities[0], players.me().entity);
        assert_eq!(players.by_entity(entities[1]).map(|p| p.id), Some(b));
        assert_eq!(world.get::<PlayerId>(entities[1]).map(|id| *id), Some(b));

        // Les inventaires sont séparés.
        let items = engine_voxel::ItemRegistry::builtin(&crate::test_registry()).unwrap();
        let stone = items.id("core:stone").unwrap();
        players
            .get_mut(b)
            .unwrap()
            .inventory
            .add_stack(crate::inventory::ItemStack::new(stone, 5), &items);
        let count = |inv: &Inventory| inv.slots.iter().flatten().map(|s| s.count).sum::<u32>();
        assert_eq!(count(&players.get(b).unwrap().inventory), 5);
        assert_eq!(count(&players.me().inventory), 0);

        // Le joueur local ne part pas ; l'autre, si, et l'ECS suit.
        assert!(players.remove(a).is_none());
        assert!(players.remove(b).is_some());
        players.sync_ecs(&mut world);
        assert_eq!(world.resource::<PlayerEntities>().0.len(), 1);
        assert_eq!(
            players.next_id(),
            PlayerId(2),
            "un numéro n'est jamais réutilisé"
        );
    }

    #[test]
    fn the_current_player_is_whoever_a_message_acts_for() {
        let (_world, mut players, a, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.0, 70.0, 0.0),
            Vec3::new(30.0, 70.0, 0.0),
        );
        assert_eq!(players.current_id(), a);
        assert!(players.current_is_local());
        // Pendant le message d'un autre joueur, `me()` est lui ; le joueur local reste joignable.
        let before = players.set_acting(Some(b));
        assert_eq!(before, None);
        assert_eq!(players.current_id(), b);
        assert!(!players.current_is_local());
        assert_eq!(players.me().id, b);
        assert_eq!(players.local().id, a);
        players.me_mut().eating = true;
        assert!(players.get(b).unwrap().eating);
        assert!(!players.get(a).unwrap().eating);
        // Le message fini, on rend le précédent.
        players.set_acting(before);
        assert_eq!(players.me().id, a);
        assert_eq!(players.entity(), players.local().entity);
    }

    #[test]
    fn quest_progress_of_guests_lives_in_their_own_folder() {
        let (_world, players, _, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.0, 70.0, 0.0),
            Vec3::new(30.0, 70.0, 0.0),
        );
        let world_dir = Path::new("/monde");
        assert_eq!(players.me().quest_dir(world_dir, true), world_dir);
        let guest = players.get(b).unwrap();
        let dir = guest.quest_dir(world_dir, false);
        assert_eq!(dir, world_dir.join("players").join(uuid_name(guest.uuid)));
    }

    /// Une plateforme de pierre dans le ciel (x de -8 à 8, z de -3 à 3, à y = 150).
    fn platform(world: &World) {
        let stone = crate::test_registry().id("core:stone").unwrap();
        let shared = crate::shared::StreamerExt::streamer(world);
        let mut streamer = shared.borrow_mut();
        for x in -8..9 {
            for z in -3..4 {
                streamer.set_block_raw(IVec3::new(x, 150, z), stone);
            }
        }
    }

    #[test]
    fn a_hostile_creature_goes_for_the_nearest_player() {
        let far = Vec3::new(-7.5, 151.0, 0.5);
        let near = Vec3::new(7.5, 151.0, 0.5);
        let slime_at = Vec3::new(5.5, 151.0, 0.5);
        let run = |both: bool| {
            let (mut world, _players, a, b) =
                two_player_world(Vec3::new(5.0, 150.0, 0.5), far, near);
            platform(&world);
            if !both {
                // Seul le joueur lointain : la créature (à 13 blocs) va vers lui.
                let gone = world.resource::<PlayerEntities>().0[1];
                world.despawn(gone);
                world.insert_resource(PlayerEntities(vec![world_entity(&world, a)]));
            }
            let _ = b;
            let slime = world
                .resource::<Arc<MobRegistry>>()
                .id("example:slime")
                .unwrap();
            let mob = spawn_mob(&mut world, slime, slime_at).unwrap();
            for _ in 0..90 {
                mobs::wander(&mut world);
                mobs::physics(&mut world);
            }
            world.get::<Transform>(mob).unwrap().position.x
        };
        let with_both = run(true);
        let alone = run(false);
        assert!(
            with_both > slime_at.x - 0.5,
            "il reste près du joueur le plus proche : {with_both}"
        );
        assert!(
            alone < slime_at.x - 1.0,
            "seul, il va vers l'autre : {alone}"
        );
    }

    /// L'entité ECS d'un joueur, retrouvée par son composant.
    fn world_entity(world: &World, id: PlayerId) -> engine_ecs::Entity {
        let mut found = None;
        world
            .query::<(&PlayerId,)>()
            .for_each(|e, (p,)| found = found.or((*p == id).then_some(e)));
        found.expect("le joueur existe")
    }

    #[test]
    fn a_creature_strikes_the_player_beside_it_and_names_them() {
        let (mut world, _players, a, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.5, 150.0, 0.5),
            Vec3::new(40.5, 150.0, 0.5),
        );
        let slime = world
            .resource::<Arc<MobRegistry>>()
            .id("example:slime")
            .unwrap();
        spawn_mob(&mut world, slime, Vec3::new(40.5, 150.0, 1.0)).unwrap();
        let mut reader = world.resource::<Events<PlayerAttacked>>().reader();
        mobs::attack(&mut world);
        let hits: Vec<_> = reader
            .read(&world.resource::<Events<PlayerAttacked>>())
            .map(|e| e.player)
            .collect();
        assert_eq!(
            hits,
            [Some(b)],
            "le coup va au joueur le plus proche, pas à {a:?}"
        );

        // Celui qui vole n'est pas touché.
        let flying = world_entity(&world, b);
        world.insert(flying, Flying);
        world.resource_mut::<Events<PlayerAttacked>>().update();
        world.resource_mut::<Events<PlayerAttacked>>().update();
        for _ in 0..2 {
            mobs::attack(&mut world);
        }
        let more = reader
            .read(&world.resource::<Events<PlayerAttacked>>())
            .count();
        assert_eq!(more, 0, "un joueur en vol n'est pas frappé");
    }

    #[test]
    fn creatures_do_not_appear_next_to_another_player() {
        let a = Vec3::new(5.0, 70.0, 5.0);
        let b = Vec3::new(29.0, 70.0, 5.0);
        let (mut world, _players, _, _) = two_player_world(a, a, b);
        for _ in 0..400 {
            world.resource_mut::<SimClock>().spawn_timer = 0.0;
            crate::spawning::spawn(&mut world);
            world.apply_commands();
        }
        let mut count = 0;
        world.query::<(&Transform, &Mob)>().for_each(|_, (t, _)| {
            count += 1;
            for player in [a, b] {
                let d = crate::spawning::horizontal_distance(t.position, player);
                // 16 blocs d'écart au minimum (un groupe déborde d'une case de chaque côté).
                assert!(d >= 14.0, "une créature est apparue à {d} d'un joueur");
            }
        });
        assert!(count > 0, "rien n'est apparu");
    }

    #[test]
    fn a_creature_stays_while_any_player_is_near_and_leaves_when_none_is() {
        let (mut world, mut players, _, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.0, 150.0, 0.0),
            Vec3::new(130.0, 150.0, 0.0),
        );
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        // À 130 du premier joueur (au-delà de 96), à 5 du second.
        let mob = spawn_mob(&mut world, pig, Vec3::new(125.0, 150.0, 0.0)).unwrap();
        mobs::cleanup(&mut world);
        world.apply_commands();
        assert!(
            world.get::<Transform>(mob).is_some(),
            "un joueur est proche"
        );
        // Le second part : plus personne près d'elle.
        players.remove(b);
        players.sync_ecs(&mut world);
        mobs::cleanup(&mut world);
        world.apply_commands();
        assert!(
            world.get::<Transform>(mob).is_none(),
            "elle aurait dû disparaître"
        );
    }

    #[test]
    fn a_death_names_the_player_who_struck_last() {
        let (mut world, _players, _, b) = two_player_world(
            Vec3::new(5.0, 40.0, 5.0),
            Vec3::new(0.5, 150.0, 0.5),
            Vec3::new(3.5, 150.0, 0.5),
        );
        let pig = world.resource::<Arc<MobRegistry>>().id("core:pig").unwrap();
        let struck = spawn_mob(&mut world, pig, Vec3::new(2.5, 150.0, 0.5)).unwrap();
        let alone = spawn_mob(&mut world, pig, Vec3::new(2.5, 150.0, 2.5)).unwrap();
        world.insert(struck, LastHitBy(b));
        for mob in [struck, alone] {
            world.get_mut::<Health>(mob).unwrap().current = 0.0;
        }
        let mut reader = world
            .resource::<Events<crate::events::EntityDied>>()
            .reader();
        mobs::cleanup(&mut world);
        let killers: Vec<(engine_ecs::Entity, Option<PlayerId>)> = reader
            .read(&world.resource::<Events<crate::events::EntityDied>>())
            .map(|e| (e.entity, e.killer))
            .collect();
        assert_eq!(killers.len(), 2);
        let by_entity = |entity| killers.iter().find(|(e, _)| *e == entity).unwrap().1;
        assert_eq!(by_entity(struck), Some(b));
        assert_eq!(
            by_entity(alone),
            None,
            "pas de tueur : chute, feu, soleil..."
        );
    }
}
