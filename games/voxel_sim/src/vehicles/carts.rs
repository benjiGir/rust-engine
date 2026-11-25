//! Les wagons : des entités de l'ECS qui roulent sur les rails (voir `engine_voxel::rails`).
//!
//! # Les sortes
//!
//! - **Wagonnet** : on y monte (composant [`Riding`] sur le passager ; la caméra suit le wagon).
//! - **Wagon coffre** : 27 cases.
//! - **Wagon trémie** : 27 cases ; ramasse les objets au sol sur son passage et vide le conteneur
//!   posé au-dessus du rail. Un rail activateur alimenté le coupe.
//! - **Locomotive** : une case de combustible (charbon, bois...) ; tant qu'elle brûle, elle pousse
//!   dans son sens de marche, et tire le train qui lui est attelé.
//!
//! # Le mouvement
//!
//! Un wagon est soit sur un rail ([`Ride::Rail`] : la physique pure de `engine_voxel::rail_motion`,
//! abscisse et vitesse le long du chemin), soit libre ([`Ride::Free`] : la physique d'une boîte,
//! avec un fort frottement). Au bout de la voie il déraille ; un wagon libre qui passe au-dessus
//! d'un rail le reprend. Tant que le terrain autour n'est pas chargé, le wagon attend, figé.
//!
//! Les wagons attelés (liens par numéro, voir [`Cart::links`]) gardent leur écart et partagent
//! leur vitesse ; deux wagons qui se touchent se repoussent. Un wagon sur un rail détecteur le
//! fait émettre : le système range les changements dans [`CartSignals`], que le jeu applique aux
//! machines.
//!
//! # Le dessin
//!
//! Des boîtes, comme les créatures : le wagon s'incline dans les pentes et tourne dans les virages,
//! interpolé entre deux pas de simulation.

use std::collections::HashSet;
use std::sync::Arc;

use engine_ecs::{Entity, World};
use engine_voxel::{
    Derail, Drive, GRAVITY, Inventory, ItemRegistry, ItemStack, PlayerBody, Rail, RailKind,
    RailMotion, SavedCart, SavedRail, Smelting, rail_at, ray_aabb,
};
use glam::{IVec3, Vec3};

use crate::components::{Collider, Mob, MoveIntent, PreviousPosition, Transform, Velocity};
use crate::mobs::FixedDt;
use crate::shared::StreamerExt;

/// Les cases d'un wagon coffre ou trémie.
const CHEST_SLOTS: usize = 27;
/// L'écart entre deux wagons attelés, centre à centre.
pub const COUPLING_LENGTH: f32 = 1.3;
/// Deux wagons plus proches que cela (non attelés) se repoussent.
const BUMP_GAP: f32 = 1.1;
/// Où s'assoit le passager par rapport aux roues du wagon.
const SEAT: Vec3 = Vec3::new(0.0, 0.3, 0.0);
/// La poussée d'un passager qui avance, et la vitesse qu'il peut donner seul.
const RIDER_PUSH: f32 = 3.0;
const RIDER_TOP_SPEED: f32 = 4.5;
/// L'accélération d'une locomotive qui brûle.
const LOCOMOTIVE_THRUST: f32 = 12.0;
/// Le frottement d'un wagon qui a quitté la voie (par seconde), au sol et en l'air.
const FREE_FRICTION: (f32, f32) = (3.0, 0.2);
/// Combien de temps un wagon qui vient de dérailler ne reprend pas de rail.
const SNAP_DELAY: f32 = 0.6;
/// Dimensions de la boîte de visée et de collision avec les objets.
const HALF_WIDTH: f32 = 0.5;
const HEIGHT: f32 = 0.85;

/// Les sortes de wagons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum CartKind {
    Minecart,
    Chest,
    Hopper,
    Locomotive,
}

impl CartKind {
    pub const ALL: [CartKind; 4] = [Self::Minecart, Self::Chest, Self::Hopper, Self::Locomotive];

    /// Le nom court (sauvegarde et commandes).
    pub fn name(self) -> &'static str {
        match self {
            Self::Minecart => "minecart",
            Self::Chest => "chest_cart",
            Self::Hopper => "hopper_cart",
            Self::Locomotive => "locomotive",
        }
    }

    /// L'identifiant de l'objet qui pose ce wagon (et que le wagon lâche en cassant).
    pub fn item_key(self) -> String {
        format!("core:{}", self.name())
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Minecart => "Wagonnet",
            Self::Chest => "Wagon coffre",
            Self::Hopper => "Wagon trémie",
            Self::Locomotive => "Locomotive",
        }
    }

    /// La sorte posée par l'objet de cet identifiant (`core:minecart`), s'il en pose une.
    pub fn from_item_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.item_key() == key)
    }

    /// Reconnaît un nom court, un identifiant d'objet, ou une abréviation de commande.
    pub fn parse(text: &str) -> Option<Self> {
        let name = text.strip_prefix("core:").unwrap_or(text);
        Self::ALL.into_iter().find(|k| {
            k.name() == name || k.name().trim_end_matches("_cart") == name || k.title() == name
        })
    }

    /// Les cases de son inventaire.
    pub fn slots(self) -> usize {
        match self {
            Self::Minecart => 0,
            Self::Chest | Self::Hopper => CHEST_SLOTS,
            Self::Locomotive => 1,
        }
    }

    /// Les couleurs de ses boîtes.
    fn paint(self) -> (Vec3, Vec3) {
        match self {
            Self::Locomotive => (Vec3::new(0.62, 0.2, 0.16), Vec3::new(0.42, 0.12, 0.1)),
            _ => (Vec3::new(0.5, 0.54, 0.56), Vec3::new(0.34, 0.37, 0.39)),
        }
    }
}

/// Comment roule un wagon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Ride {
    Rail(RailMotion),
    Free { velocity: Vec3 },
}

/// L'état d'un wagon (le composant principal). La position est dans `Transform`, le contenu dans
/// [`CartCargo`].
#[derive(Debug, Clone, PartialEq)]
pub struct Cart {
    /// Le numéro, stable (sauvegarde, attelages).
    pub id: u32,
    pub kind: CartKind,
    pub ride: Ride,
    /// Secondes de combustible qui brûlent encore (locomotive).
    pub burn: f32,
    /// Les numéros des wagons attelés à celui-ci.
    pub links: Vec<u32>,
    /// Le sens de la marche (lacet) et l'inclinaison, avec leur valeur au pas précédent.
    pub yaw: f32,
    pub pitch: f32,
    pub previous_yaw: f32,
    pub previous_pitch: f32,
    /// Qui est assis dedans.
    pub rider: Option<Entity>,
    /// Les coups reçus récemment (le wagon casse au deuxième) : secondes avant qu'ils s'oublient.
    pub hurt: f32,
    /// Pas de nouveau passager (une créature) avant la fin de ce délai.
    pub boarding_delay: f32,
    /// Pas de reprise de rail avant la fin de ce délai (après un déraillement).
    pub snap_delay: f32,
    /// Le terrain autour n'est pas chargé : le wagon attend.
    pub frozen: bool,
}

impl Cart {
    pub fn new(id: u32, kind: CartKind, ride: Ride) -> Self {
        Self {
            id,
            kind,
            ride,
            burn: 0.0,
            links: Vec::new(),
            yaw: 0.0,
            pitch: 0.0,
            previous_yaw: 0.0,
            previous_pitch: 0.0,
            rider: None,
            hurt: 0.0,
            boarding_delay: 0.0,
            snap_delay: 0.0,
            frozen: false,
        }
    }

    /// La vitesse (blocs par seconde, positive), sur un rail ou non.
    pub fn speed(&self) -> f32 {
        match &self.ride {
            Ride::Rail(m) => m.speed.abs(),
            Ride::Free { velocity } => velocity.length(),
        }
    }

    pub fn on_rail(&self) -> bool {
        matches!(self.ride, Ride::Rail(_))
    }

    /// Le bloc du rail sur lequel il roule.
    pub fn rail_block(&self) -> Option<IVec3> {
        match &self.ride {
            Ride::Rail(m) => Some(m.pos),
            Ride::Free { .. } => None,
        }
    }

    /// Tourne vers le sens de la marche et s'incline selon la pente.
    fn orient(&mut self) {
        match &self.ride {
            Ride::Rail(m) => {
                let dir = m.tangent() * m.heading;
                self.yaw = dir.z.atan2(dir.x);
                self.pitch = dir.y.clamp(-1.0, 1.0).asin();
            }
            Ride::Free { velocity } => {
                if velocity.x.hypot(velocity.z) > 0.5 {
                    self.yaw = velocity.z.atan2(velocity.x);
                }
                self.pitch *= 0.9;
            }
        }
    }

    /// Un pas de physique. `position` est celle du wagon (ses roues), mise à jour. Renvoie `true`
    /// si le wagon vient de dérailler.
    pub fn advance(
        &mut self,
        position: &mut Vec3,
        dt: f32,
        drive: &Drive,
        rails: &dyn Fn(IVec3) -> Option<Rail>,
        world: &engine_voxel::World,
        gravity: f32,
    ) -> bool {
        self.snap_delay = (self.snap_delay - dt).max(0.0);
        let mut derailed = false;
        match &mut self.ride {
            Ride::Rail(motion) => match motion.step(dt, drive, rails) {
                None => *position = motion.position(),
                Some(Derail {
                    position: p,
                    velocity,
                }) => {
                    *position = p;
                    self.ride = Ride::Free { velocity };
                    self.snap_delay = SNAP_DELAY;
                    derailed = true;
                }
            },
            Ride::Free { velocity } => {
                let mut body = PlayerBody::with_size(*position, 0.9, 0.7);
                body.velocity = *velocity;
                body.gravity = gravity;
                body.step(world, Vec3::new(velocity.x, 0.0, velocity.z), false, dt);
                let friction = if body.on_ground {
                    FREE_FRICTION.0
                } else {
                    FREE_FRICTION.1
                };
                let keep = (1.0 - friction * dt).max(0.0);
                *velocity = Vec3::new(
                    body.velocity.x * keep,
                    body.velocity.y,
                    body.velocity.z * keep,
                );
                if velocity.x.hypot(velocity.z) < 0.05 {
                    velocity.x = 0.0;
                    velocity.z = 0.0;
                }
                *position = body.position;
                // Un rail tout près, dessous : le wagon le reprend.
                if self.snap_delay <= 0.0
                    && velocity.y < 1.0
                    && let Some(motion) = RailMotion::snap(*position, *velocity, rails)
                {
                    *position = motion.position();
                    self.ride = Ride::Rail(motion);
                }
            }
        }
        self.orient();
        derailed
    }
}

/// Le contenu du wagon (coffre, trémie) ou son combustible (locomotive).
#[derive(Debug, Clone, PartialEq)]
pub struct CartCargo(pub Inventory);

/// Un passager : l'entité du wagon où il est assis (le joueur ou une créature).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Riding(pub Entity);

/// Les réglages et compteurs partagés (ressource).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CartConfig {
    /// La vitesse maximale, en blocs par seconde (le réglage des options).
    pub max_speed: f32,
    /// Le prochain numéro de wagon.
    pub next_id: u32,
}

/// Les rails détecteurs allumés, et les changements que le jeu doit encore appliquer aux machines.
#[derive(Debug, Default)]
pub struct CartSignals {
    pub lit: HashSet<IVec3>,
    pub pending: Vec<(IVec3, bool)>,
    /// Les chargeurs et déchargeurs qui ont fini (ils émettent un signal).
    pub loaders: HashSet<IVec3>,
    /// Pas depuis le dernier passage des chargeurs.
    pub tick: u32,
}

/// Le premier wagon cliqué avec l'attelage, en attente du second.
#[derive(Debug, Default, Clone, Copy)]
pub struct CartPending(pub Option<Entity>);

/// Ce dont le système a besoin pour lire les objets (clés de texte) et le combustible.
pub struct CartItems {
    pub items: Arc<ItemRegistry>,
    pub smelting: Arc<Smelting>,
}

/// Prépare l'ECS : réglages, signaux, tables d'objets.
pub fn install(
    world: &mut World,
    max_speed: f32,
    items: Arc<ItemRegistry>,
    smelting: Arc<Smelting>,
) {
    world.insert_resource(CartConfig {
        max_speed: max_speed.clamp(1.0, engine_voxel::SPEED_LIMIT),
        next_id: 1,
    });
    world.insert_resource(CartSignals::default());
    world.insert_resource(CartPending::default());
    world.insert_resource(CartItems { items, smelting });
}

/// Prépare l'ECS pour un monde : les réglages, puis les wagons du dossier de sauvegarde (`dir`).
pub fn setup(
    world: &mut World,
    settings: &crate::settings::Settings,
    content: &crate::content::Content,
    store: Option<&engine_voxel::SaveStore>,
) {
    install(
        world,
        settings.cart_max_speed,
        content.items.clone(),
        content.smelting.clone(),
    );
    if let Some(store) = store {
        match engine_voxel::load_carts(store.dir()) {
            Ok(saved) => {
                if !saved.is_empty() {
                    log::info!("{} wagon(s) chargé(s)", saved.len());
                }
                load_saved(world, &saved);
            }
            Err(err) => log::error!("wagons non chargés : {err:#}"),
        }
    }
}

/// Écrit les wagons dans le dossier du monde (voir `world/persistence.rs`).
pub fn save(world: &World, dir: &std::path::Path) {
    if let Err(err) = engine_voxel::save_carts(dir, &to_saved(world)) {
        log::error!("sauvegarde des wagons : {err:#}");
    }
}

/// Crée un wagon. `ride` : sur un rail ou libre. Renvoie son entité.
pub fn spawn(world: &mut World, kind: CartKind, position: Vec3, ride: Ride) -> Entity {
    let id = {
        let mut config = world.resource_mut::<CartConfig>();
        let id = config.next_id;
        config.next_id += 1;
        id
    };
    spawn_with_id(world, id, kind, position, ride)
}

fn spawn_with_id(world: &mut World, id: u32, kind: CartKind, position: Vec3, ride: Ride) -> Entity {
    let mut cart = Cart::new(id, kind, ride);
    cart.orient();
    cart.previous_yaw = cart.yaw;
    cart.previous_pitch = cart.pitch;
    let entity = world.spawn((
        Transform {
            position,
            yaw: cart.yaw,
        },
        PreviousPosition(position),
        cart,
    ));
    world.insert(entity, CartCargo(Inventory::new(kind.slots())));
    entity
}

/// Pose un wagon sur le rail en `block`, tourné dans le sens de `look` (la direction du regard).
/// `None` s'il n'y a pas de rail.
pub fn place(world: &mut World, kind: CartKind, block: IVec3, look: Vec3) -> Option<Entity> {
    let shared = world.streamer();
    let rail = rail_at(&*shared.borrow(), block)?;
    let mut motion = RailMotion::on(block, rail.shape, rail.shape.length() * 0.5);
    let along = motion.tangent().dot(Vec3::new(look.x, 0.0, look.z));
    motion.heading = if along < 0.0 { -1.0 } else { 1.0 };
    let position = motion.position();
    Some(spawn(world, kind, position, Ride::Rail(motion)))
}

/// Tous les wagons, dans l'ordre de leur numéro.
pub fn all(world: &World) -> Vec<Entity> {
    let mut list: Vec<(u32, Entity)> = Vec::new();
    world
        .query::<(&Cart,)>()
        .for_each(|e, (cart,)| list.push((cart.id, e)));
    list.sort_by_key(|(id, _)| *id);
    list.into_iter().map(|(_, e)| e).collect()
}

/// Le terrain est-il chargé autour de ce point (le wagon peut bouger d'un demi-bloc par sous-pas) ?
fn ready(streamer: &engine_voxel::ChunkStreamer, position: Vec3) -> bool {
    let first = engine_voxel::World::chunk_containing(position - Vec3::splat(2.5));
    let last = engine_voxel::World::chunk_containing(position + Vec3::splat(2.5));
    (first.x..=last.x).all(|x| {
        (first.y..=last.y)
            .all(|y| (first.z..=last.z).all(|z| streamer.is_generated(IVec3::new(x, y, z))))
    })
}

/// La poussée d'un passager qui avance (positive : dans le sens des abscisses du chemin).
fn rider_push(world: &World, rider: Option<Entity>, tangent: Vec3, speed: f32) -> f32 {
    let Some(rider) = rider else { return 0.0 };
    let Some(intent) = world.get::<MoveIntent>(rider).map(|i| i.walk) else {
        return 0.0;
    };
    let wish = Vec3::new(intent.x, 0.0, intent.z);
    let flat = Vec3::new(tangent.x, 0.0, tangent.z);
    if wish.length() < 0.1 || flat.length() < 0.2 {
        return 0.0;
    }
    let along = flat.normalize().dot(wish.normalize());
    if along.abs() < 0.3 {
        return 0.0;
    }
    let sign = along.signum();
    // On ne donne pas plus qu'une petite vitesse.
    if speed * sign >= RIDER_TOP_SPEED {
        return 0.0;
    }
    sign * RIDER_PUSH
}

/// Le combustible d'une locomotive : rallume avec la case de combustible quand c'est éteint.
fn feed_locomotive(world: &World, entity: Entity, cart: &mut Cart, dt: f32) {
    cart.burn = (cart.burn - dt).max(0.0);
    if cart.burn > 0.0 {
        return;
    }
    let items = world.resource::<CartItems>();
    let Some(mut cargo) = world.get_mut::<CartCargo>(entity) else {
        return;
    };
    let Some(slot) = cargo.0.slots.first_mut() else {
        return;
    };
    let Some(stack) = slot.as_mut() else { return };
    let Some(ticks) = items
        .items
        .id(&stack.item)
        .and_then(|id| items.smelting.burn_ticks(id))
    else {
        return;
    };
    stack.count -= 1;
    if stack.count == 0 {
        *slot = None;
    }
    cart.burn = ticks as f32 / engine_voxel::MACHINE_TICKS_PER_SECOND as f32;
}

/// Fait asseoir `rider` dans le wagon `cart`. `false` si le wagon est plein ou n'est pas un
/// wagonnet.
pub fn board(world: &mut World, rider: Entity, cart: Entity) -> bool {
    let Some(mut state) = world.get_mut::<Cart>(cart) else {
        return false;
    };
    if state.kind != CartKind::Minecart || state.rider.is_some() {
        return false;
    }
    state.rider = Some(rider);
    drop(state);
    // Un passager déjà assis ailleurs en descend d'abord.
    dismount(world, rider);
    if let Some(mut state) = world.get_mut::<Cart>(cart) {
        state.rider = Some(rider);
    }
    world.insert(rider, Riding(cart));
    true
}

/// Fait descendre le passager : il se retrouve au-dessus du wagon et retombe.
pub fn dismount(world: &mut World, rider: Entity) {
    let Some(Riding(cart)) = world.remove::<Riding>(rider) else {
        return;
    };
    let above = world
        .get::<Transform>(cart)
        .map_or(Vec3::ZERO, |t| t.position + Vec3::Y * 1.0);
    if let Some(mut state) = world.get_mut::<Cart>(cart) {
        state.rider = None;
        state.boarding_delay = 3.0;
    }
    if let Some(mut transform) = world.get_mut::<Transform>(rider) {
        transform.position = above;
    }
    if let Some(mut previous) = world.get_mut::<PreviousPosition>(rider) {
        previous.0 = above;
    }
    if let Some(mut velocity) = world.get_mut::<Velocity>(rider) {
        velocity.0 = Vec3::ZERO;
    }
}

/// Le système de pas fixe : fait rouler, atteler, porter les passagers et ramasser les objets.
pub fn update(world: &mut World) {
    if world.get_resource::<CartConfig>().is_none() {
        return;
    }
    let carts = all(world);
    if carts.is_empty() {
        // Plus de wagon : plus de détecteur allumé.
        let mut signals = world.resource_mut::<CartSignals>();
        let lit: Vec<IVec3> = signals.lit.drain().collect();
        signals.pending.extend(lit.into_iter().map(|p| (p, false)));
        return;
    }
    let dt = world.resource::<FixedDt>().0;
    let max_speed = world.resource::<CartConfig>().max_speed;
    let gravity = GRAVITY
        * world
            .get_resource::<crate::components::GravityScale>()
            .map_or(1.0, |g| g.0);
    let shared = world.streamer();
    let streamer = shared.borrow();
    let rails = |p: IVec3| rail_at(&*streamer, p);
    let mut ejected: Vec<Entity> = Vec::new();
    let mut lit: HashSet<IVec3> = HashSet::new();

    // 1. Chaque wagon roule.
    for &entity in &carts {
        let position = world
            .get::<Transform>(entity)
            .map(|t| t.position)
            .unwrap_or_default();
        let mut cart = world.get_mut::<Cart>(entity).expect("un wagon");
        cart.previous_yaw = cart.yaw;
        cart.previous_pitch = cart.pitch;
        cart.hurt = (cart.hurt - dt).max(0.0);
        cart.boarding_delay = (cart.boarding_delay - dt).max(0.0);
        if let Some(mut previous) = world.get_mut::<PreviousPosition>(entity) {
            previous.0 = position;
        }
        cart.frozen = !ready(&streamer, position);
        if cart.frozen {
            continue;
        }
        let here = cart.rail_block().and_then(rails);
        let (mut thrust, mut push) = (0.0, 0.0);
        if let Ride::Rail(m) = &cart.ride {
            push = rider_push(world, cart.rider, m.tangent(), m.speed);
        }
        if cart.kind == CartKind::Locomotive {
            feed_locomotive(world, entity, &mut cart, dt);
            if cart.burn > 0.0 {
                thrust = LOCOMOTIVE_THRUST;
            }
        }
        if here.is_some_and(|r| r.kind == RailKind::Activator && r.powered) && cart.rider.is_some()
        {
            ejected.extend(cart.rider);
        }
        let drive = Drive {
            max_speed,
            thrust,
            push,
        };
        let mut new_position = position;
        cart.advance(
            &mut new_position,
            dt,
            &drive,
            &rails,
            streamer.world(),
            gravity,
        );
        if let Some(mut transform) = world.get_mut::<Transform>(entity) {
            transform.position = new_position;
            transform.yaw = cart.yaw;
        }
    }

    // 2. L'attelage et les chocs, sur rail ou non (un train déraillé garde ses écarts).
    let solid = |p: IVec3| streamer.world().is_solid(p);
    couple_all(world, &carts, &rails, &solid);

    // 3. Les détecteurs : allumés tant qu'un wagon roule dessus.
    for &entity in &carts {
        if let Some(cart) = world.get::<Cart>(entity)
            && let Some(block) = cart.rail_block()
            && rails(block).is_some_and(|r| r.kind == RailKind::Detector)
        {
            lit.insert(block);
        }
    }
    drop(streamer);
    {
        let mut signals = world.resource_mut::<CartSignals>();
        let changes: Vec<(IVec3, bool)> = lit
            .difference(&signals.lit)
            .map(|&p| (p, true))
            .chain(signals.lit.difference(&lit).map(|&p| (p, false)))
            .collect();
        signals.pending.extend(changes);
        signals.lit = lit;
    }

    for rider in ejected {
        dismount(world, rider);
    }
    carry_riders(world);
    board_mobs(world, &carts);
    collect_items(world, &carts);
}

/// Un wagon vu par l'attelage et les chocs : sur un rail, il ne se corrige que le long de sa voie ;
/// libre (déraillé, tombé), il se corrige dans le monde, sans entrer dans un bloc plein.
enum Body {
    Rail(RailMotion),
    Free { position: Vec3, velocity: Vec3 },
}

impl Body {
    fn position(&self) -> Vec3 {
        match self {
            Body::Rail(m) => m.position(),
            Body::Free { position, .. } => *position,
        }
    }

    fn along(&self, u: Vec3) -> f32 {
        match self {
            Body::Rail(m) => m.along(u),
            Body::Free { velocity, .. } => velocity.dot(u),
        }
    }

    fn nudge(&mut self, u: Vec3, delta: f32) {
        match self {
            Body::Rail(m) => m.nudge(u, delta),
            Body::Free { velocity, .. } => *velocity += u * delta,
        }
    }

    /// Avance de `amount` vers `u`.
    fn pull(
        &mut self,
        u: Vec3,
        amount: f32,
        rails: &dyn Fn(IVec3) -> Option<Rail>,
        solid: &dyn Fn(IVec3) -> bool,
    ) {
        match self {
            Body::Rail(m) => m.pull(u, amount, rails),
            Body::Free { position, .. } => {
                // À l'horizontale : la pesanteur et le sol s'occupent de la hauteur.
                let flat = Vec3::new(u.x, 0.0, u.z);
                let target = *position + flat * amount;
                let blocked = [0.1, 0.6]
                    .iter()
                    .any(|&dy| solid((target + Vec3::Y * dy).floor().as_ivec3()));
                if !blocked {
                    *position = target;
                }
            }
        }
    }
}

/// La direction de `a` vers `b` et leur distance. Deux wagons libres exactement l'un sur
/// l'autre (un train tombé d'un coup) s'écartent selon leur numéro, toujours dans le même sens.
fn separation(a: &Body, b: &Body, ids: (u32, u32)) -> (Vec3, f32) {
    let d = b.position() - a.position();
    let dist = d.length();
    if dist > 1e-3 {
        (d / dist, dist)
    } else if ids.0 < ids.1 {
        (Vec3::X, 0.0)
    } else {
        (-Vec3::X, 0.0)
    }
}

/// Attelage : `a` et `b` restent à `length` l'un de l'autre et partagent leur vitesse.
fn link(
    a: &mut Body,
    b: &mut Body,
    ids: (u32, u32),
    length: f32,
    rails: &dyn Fn(IVec3) -> Option<Rail>,
    solid: &dyn Fn(IVec3) -> bool,
) {
    let (u, dist) = separation(a, b, ids);
    let err = dist - length;
    if err.abs() > 1e-4 {
        a.pull(u, err * 0.5, rails, solid);
        b.pull(-u, err * 0.5, rails, solid);
    }
    let (va, vb) = (a.along(u), b.along(u));
    let common = (va + vb) * 0.5;
    a.nudge(u, common - va);
    b.nudge(u, common - vb);
}

/// Choc : deux wagons à moins de `gap` se repoussent et échangent leur élan (rebond de 30 %).
fn push_apart(
    a: &mut Body,
    b: &mut Body,
    ids: (u32, u32),
    gap: f32,
    rails: &dyn Fn(IVec3) -> Option<Rail>,
    solid: &dyn Fn(IVec3) -> bool,
) {
    let (u, dist) = separation(a, b, ids);
    if dist >= gap {
        return;
    }
    a.pull(u, (dist - gap) * 0.5, rails, solid);
    b.pull(-u, (dist - gap) * 0.5, rails, solid);
    let (va, vb) = (a.along(u), b.along(u));
    if va > vb {
        let common = (va + vb) * 0.5;
        let spread = (va - vb) * 0.5 * 0.3;
        a.nudge(u, common - spread - va);
        b.nudge(u, common + spread - vb);
    }
}

/// Attelages et chocs entre tous les wagons, sur rail ou non.
fn couple_all(
    world: &World,
    carts: &[Entity],
    rails: &dyn Fn(IVec3) -> Option<Rail>,
    solid: &dyn Fn(IVec3) -> bool,
) {
    struct Item {
        entity: Entity,
        id: u32,
        links: Vec<u32>,
        body: Body,
    }
    let mut items: Vec<Item> = Vec::new();
    for &entity in carts {
        let Some(cart) = world.get::<Cart>(entity) else {
            continue;
        };
        if cart.frozen {
            continue;
        }
        let body = match cart.ride {
            Ride::Rail(motion) => Body::Rail(motion),
            Ride::Free { velocity } => Body::Free {
                position: world
                    .get::<Transform>(entity)
                    .map(|t| t.position)
                    .unwrap_or_default(),
                velocity,
            },
        };
        items.push(Item {
            entity,
            id: cart.id,
            links: cart.links.clone(),
            body,
        });
    }
    let linked = |a: &Item, b: &Item| a.links.contains(&b.id) || b.links.contains(&a.id);
    // Les attelages, plusieurs fois pour que la correction se propage dans un long train.
    for _ in 0..4 {
        for i in 0..items.len() {
            for j in i + 1..items.len() {
                if !linked(&items[i], &items[j]) {
                    continue;
                }
                let (left, right) = items.split_at_mut(j);
                let (a, b) = (&mut left[i], &mut right[0]);
                link(
                    &mut a.body,
                    &mut b.body,
                    (a.id, b.id),
                    COUPLING_LENGTH,
                    rails,
                    solid,
                );
            }
        }
    }
    // Les chocs entre wagons qui ne sont pas attelés.
    for i in 0..items.len() {
        for j in i + 1..items.len() {
            if linked(&items[i], &items[j])
                || items[i]
                    .body
                    .position()
                    .distance_squared(items[j].body.position())
                    > 4.0
            {
                continue;
            }
            let (left, right) = items.split_at_mut(j);
            let (a, b) = (&mut left[i], &mut right[0]);
            push_apart(
                &mut a.body,
                &mut b.body,
                (a.id, b.id),
                BUMP_GAP,
                rails,
                solid,
            );
        }
    }
    for item in items {
        let position = item.body.position();
        if let Some(mut cart) = world.get_mut::<Cart>(item.entity) {
            cart.ride = match item.body {
                Body::Rail(motion) => Ride::Rail(motion),
                Body::Free { velocity, .. } => Ride::Free { velocity },
            };
            cart.orient();
            if let Some(mut transform) = world.get_mut::<Transform>(item.entity) {
                transform.position = position;
                transform.yaw = cart.yaw;
            }
        }
    }
}

/// Les passagers suivent leur wagon (le joueur : la caméra suit le joueur).
fn carry_riders(world: &mut World) {
    let mut riders: Vec<(Entity, Entity)> = Vec::new();
    world
        .query::<(&Riding,)>()
        .for_each(|e, (r,)| riders.push((e, r.0)));
    for (rider, cart) in riders {
        let Some(cart_position) = world.get::<Transform>(cart).map(|t| t.position) else {
            // Le wagon n'existe plus.
            world.remove::<Riding>(rider);
            continue;
        };
        let previous = world
            .get::<PreviousPosition>(cart)
            .map_or(cart_position, |p| p.0);
        // Un cheval a son propre siège (voir `mobs/mounts.rs`), qui suit son cap.
        let seat = crate::mobs::mounts::seat(world, cart).map_or(SEAT, |s| s - cart_position);
        if let Some(mut transform) = world.get_mut::<Transform>(rider) {
            transform.position = cart_position + seat;
        }
        if let Some(mut p) = world.get_mut::<PreviousPosition>(rider) {
            p.0 = previous + seat;
        }
        if let Some(mut velocity) = world.get_mut::<Velocity>(rider) {
            velocity.0 = Vec3::ZERO;
        }
        // Assis : pas de chute.
        if let Some(mut collider) = world.get_mut::<Collider>(rider) {
            collider.on_ground = true;
        }
    }
}

/// Une créature qui entre dans un wagonnet vide y monte.
fn board_mobs(world: &mut World, carts: &[Entity]) {
    let mut mobs: Vec<(Entity, Vec3)> = Vec::new();
    world.query::<(&Transform, &Mob)>().for_each(|e, (t, _)| {
        if !world.has::<Riding>(e) {
            mobs.push((e, t.position));
        }
    });
    if mobs.is_empty() {
        return;
    }
    for &cart in carts {
        let (position, free) = {
            let Some(state) = world.get::<Cart>(cart) else {
                continue;
            };
            let free = state.kind == CartKind::Minecart
                && state.rider.is_none()
                && state.boarding_delay <= 0.0
                && !state.frozen;
            let position = world.get::<Transform>(cart).map(|t| t.position);
            (position, free)
        };
        let Some(position) = position.filter(|_| free) else {
            continue;
        };
        let near = mobs.iter().position(|(_, p)| {
            Vec3::new(p.x - position.x, 0.0, p.z - position.z).length() < 0.55
                && (p.y - position.y).abs() < 1.0
        });
        if let Some(i) = near {
            let (mob, _) = mobs.swap_remove(i);
            board(world, mob, cart);
        }
    }
}

/// Le wagon trémie ramasse les objets au sol qu'il survole, et vide le conteneur au-dessus.
fn collect_items(world: &mut World, carts: &[Entity]) {
    let hoppers: Vec<(Entity, Vec3)> = carts
        .iter()
        .filter_map(|&e| {
            let cart = world.get::<Cart>(e)?;
            if cart.kind != CartKind::Hopper || cart.frozen {
                return None;
            }
            Some((e, world.get::<Transform>(e)?.position))
        })
        .collect();
    if hoppers.is_empty() {
        return;
    }
    // Un wagon coupé par un rail activateur alimenté ne ramasse rien.
    let shared = world.streamer();
    let off: HashSet<Entity> = {
        let streamer = shared.borrow();
        hoppers
            .iter()
            .filter(|(e, _)| {
                world
                    .get::<Cart>(*e)
                    .and_then(|c| c.rail_block())
                    .and_then(|b| rail_at(&*streamer, b))
                    .is_some_and(|r| r.kind == RailKind::Activator && r.powered)
            })
            .map(|(e, _)| *e)
            .collect()
    };
    let mut found: Vec<(Entity, Vec3, crate::player::inventory::ItemStack)> = Vec::new();
    world
        .query::<(&Transform, &crate::player::drops::DroppedItem)>()
        .for_each(|e, (t, d)| found.push((e, t.position, d.stack)));
    let items = world.resource::<CartItems>().items.clone();
    for (item_entity, at, stack) in found {
        let Some((cart, _)) = hoppers.iter().find(|(e, p)| {
            !off.contains(e)
                && Vec3::new(p.x - at.x, 0.0, p.z - at.z).length() < 1.1
                && (-0.3..1.4).contains(&(at.y - p.y))
        }) else {
            continue;
        };
        let key = items.get(stack.item).key.clone();
        let Some(mut cargo) = world.get_mut::<CartCargo>(*cart) else {
            continue;
        };
        let taken = cargo.0.insert(None, &key, stack.count, false);
        drop(cargo);
        if taken >= stack.count {
            world.despawn(item_entity);
        } else if taken > 0
            && let Some(mut dropped) =
                world.get_mut::<crate::player::drops::DroppedItem>(item_entity)
        {
            dropped.stack.count -= taken;
        }
    }
}

/// Les wagons touchés par un rayon : le plus proche, avant `max` blocs.
pub fn hit_cart(world: &World, origin: Vec3, dir: Vec3, max: f32) -> Option<(Entity, f32)> {
    let mut best: Option<(Entity, f32)> = None;
    world
        .query::<(&Transform, &Cart)>()
        .for_each(|entity, (transform, _)| {
            let half = Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH);
            let min = transform.position - half;
            let top = transform.position + half + Vec3::Y * HEIGHT;
            if let Some(t) = ray_aabb(origin, dir, min, top)
                && t <= max
                && best.is_none_or(|(_, b)| t < b)
            {
                best = Some((entity, t));
            }
        });
    best
}

/// Retire un wagon (il est cassé ou ramassé) : rend le passager, défait les attelages. Renvoie
/// ce qu'il contenait.
pub fn remove(world: &mut World, entity: Entity) -> Vec<ItemStack> {
    let (id, rider) = match world.get::<Cart>(entity) {
        Some(c) => (c.id, c.rider),
        None => return Vec::new(),
    };
    if let Some(rider) = rider {
        dismount(world, rider);
    }
    let cargo = world
        .remove::<CartCargo>(entity)
        .map(|c| c.0.slots.into_iter().flatten().collect())
        .unwrap_or_default();
    for other in all(world) {
        if let Some(mut cart) = world.get_mut::<Cart>(other) {
            cart.links.retain(|&l| l != id);
        }
    }
    world.despawn(entity);
    cargo
}

/// Attelle deux wagons (ou les détache s'ils l'étaient). `true` : maintenant attelés.
pub fn toggle_coupling(world: &World, a: Entity, b: Entity) -> Option<bool> {
    if a == b {
        return None;
    }
    let (ida, idb) = (world.get::<Cart>(a)?.id, world.get::<Cart>(b)?.id);
    let linked = world.get::<Cart>(a)?.links.contains(&idb);
    let mut cart_a = world.get_mut::<Cart>(a)?;
    cart_a.links.retain(|&l| l != idb);
    if !linked {
        cart_a.links.push(idb);
    }
    drop(cart_a);
    let mut cart_b = world.get_mut::<Cart>(b)?;
    cart_b.links.retain(|&l| l != ida);
    if !linked {
        cart_b.links.push(ida);
    }
    Some(!linked)
}

// ----- Sauvegarde -----

/// L'état de tous les wagons, prêt à être écrit (voir `engine_voxel::save_carts`).
pub fn to_saved(world: &World) -> Vec<SavedCart> {
    let mut out = Vec::new();
    for entity in all(world) {
        let (Some(cart), Some(transform)) =
            (world.get::<Cart>(entity), world.get::<Transform>(entity))
        else {
            continue;
        };
        let (rail, velocity) = match &cart.ride {
            Ride::Rail(m) => (
                Some(SavedRail {
                    block: m.pos.to_array(),
                    s: m.s,
                    speed: m.speed,
                    heading: m.heading,
                }),
                [0.0; 3],
            ),
            Ride::Free { velocity } => (None, velocity.to_array()),
        };
        out.push(SavedCart {
            id: cart.id,
            kind: cart.kind.name().to_string(),
            position: transform.position.to_array(),
            yaw: cart.yaw,
            rail,
            velocity,
            items: world
                .get::<CartCargo>(entity)
                .map(|c| c.0.slots.clone())
                .unwrap_or_default(),
            burn: cart.burn,
            links: cart.links.clone(),
        });
    }
    out
}

/// Remet les wagons sauvegardés dans l'ECS. Un wagon d'une sorte inconnue est ignoré. Sur rail,
/// la forme du rail est relue au premier pas (le terrain n'est peut-être pas encore là) : on garde
/// la dernière forme connue à l'abscisse sauvegardée.
pub fn load_saved(world: &mut World, carts: &[SavedCart]) {
    let mut next = 1;
    for saved in carts {
        let Some(kind) = CartKind::parse(&saved.kind) else {
            log::warn!("wagon de sorte inconnue ignoré : {}", saved.kind);
            continue;
        };
        let position = Vec3::from_array(saved.position);
        let ride = match &saved.rail {
            Some(rail) => {
                let block = IVec3::from_array(rail.block);
                // La forme exacte est lue sur le rail quand le terrain est là ; en attendant, la
                // droite qui passe par la position sauvegardée.
                let shape = engine_voxel::RailShape::NorthSouth;
                let mut motion = RailMotion::on(block, shape, 0.0);
                motion.s = rail.s;
                motion.speed = rail.speed;
                motion.heading = rail.heading;
                Ride::Rail(motion)
            }
            None => Ride::Free {
                velocity: Vec3::from_array(saved.velocity),
            },
        };
        let entity = spawn_with_id(world, saved.id, kind, position, ride);
        if let Some(mut cart) = world.get_mut::<Cart>(entity) {
            cart.burn = saved.burn;
            cart.links = saved.links.clone();
            cart.yaw = saved.yaw;
            cart.previous_yaw = saved.yaw;
        }
        if let Some(mut cargo) = world.get_mut::<CartCargo>(entity) {
            let mut inventory = Inventory::new(kind.slots());
            for (slot, stack) in inventory.slots.iter_mut().zip(&saved.items) {
                slot.clone_from(stack);
            }
            cargo.0 = inventory;
        }
        next = next.max(saved.id + 1);
    }
    world.resource_mut::<CartConfig>().next_id = next;
}

// ----- Dessin -----

/// Les boîtes d'un wagon : (décalage local, taille, couleur plus ou moins claire). Le décalage est
/// dans le repère du wagon : x vers l'avant, y vers le haut, z sur le côté.
pub fn parts(kind: CartKind) -> Vec<(Vec3, Vec3, Vec3)> {
    let (body, dark) = kind.paint();
    let wheel = Vec3::new(0.13, 0.13, 0.15);
    let mut out = vec![
        (Vec3::new(0.0, 0.22, 0.0), Vec3::new(0.96, 0.08, 0.7), dark),
        (
            Vec3::new(0.44, 0.42, 0.0),
            Vec3::new(0.08, 0.38, 0.72),
            body,
        ),
        (
            Vec3::new(-0.44, 0.42, 0.0),
            Vec3::new(0.08, 0.38, 0.72),
            body,
        ),
        (
            Vec3::new(0.0, 0.42, 0.32),
            Vec3::new(0.96, 0.38, 0.08),
            body,
        ),
        (
            Vec3::new(0.0, 0.42, -0.32),
            Vec3::new(0.96, 0.38, 0.08),
            body,
        ),
    ];
    for (x, z) in [(0.32, 0.38), (0.32, -0.38), (-0.32, 0.38), (-0.32, -0.38)] {
        out.push((Vec3::new(x, 0.12, z), Vec3::new(0.22, 0.22, 0.06), wheel));
    }
    match kind {
        CartKind::Minecart => {}
        CartKind::Chest => {
            out.push((
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(0.64, 0.34, 0.56),
                Vec3::new(0.5, 0.34, 0.2),
            ));
            out.push((
                Vec3::new(0.0, 0.68, 0.0),
                Vec3::new(0.66, 0.06, 0.58),
                Vec3::new(0.38, 0.25, 0.14),
            ));
            out.push((
                Vec3::new(0.0, 0.58, 0.29),
                Vec3::new(0.1, 0.1, 0.03),
                Vec3::new(0.85, 0.7, 0.25),
            ));
        }
        CartKind::Hopper => {
            out.push((
                Vec3::new(0.0, 0.58, 0.0),
                Vec3::new(0.84, 0.12, 0.62),
                Vec3::new(0.2, 0.22, 0.24),
            ));
            out.push((
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(0.5, 0.22, 0.4),
                Vec3::new(0.16, 0.17, 0.19),
            ));
            out.push((
                Vec3::new(0.0, 0.65, 0.0),
                Vec3::new(0.44, 0.02, 0.34),
                Vec3::new(0.03, 0.03, 0.04),
            ));
        }
        CartKind::Locomotive => {
            out.push((
                Vec3::new(0.12, 0.62, 0.0),
                Vec3::new(0.66, 0.42, 0.6),
                Vec3::new(0.32, 0.34, 0.36),
            ));
            out.push((
                Vec3::new(0.3, 0.98, 0.0),
                Vec3::new(0.18, 0.32, 0.18),
                Vec3::new(0.1, 0.1, 0.12),
            ));
            out.push((
                Vec3::new(0.46, 0.55, 0.0),
                Vec3::new(0.06, 0.14, 0.14),
                Vec3::new(1.0, 0.85, 0.4),
            ));
            out.push((
                Vec3::new(-0.3, 0.66, 0.0),
                Vec3::new(0.24, 0.3, 0.56),
                Vec3::new(0.26, 0.08, 0.07),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use engine_voxel::{RailShape, couple};

    use super::*;

    type Track = HashMap<IVec3, Rail>;

    fn straight(n: i32) -> Track {
        (0..n)
            .map(|x| {
                (
                    IVec3::new(x, 0, 0),
                    Rail {
                        kind: RailKind::Normal,
                        shape: RailShape::EastWest,
                        powered: false,
                    },
                )
            })
            .collect()
    }

    fn on_rail(id: u32, kind: CartKind, x: f32) -> (Cart, Vec3) {
        let motion = RailMotion::on(
            IVec3::new(x.floor() as i32, 0, 0),
            RailShape::EastWest,
            x.fract(),
        );
        let position = motion.position();
        (Cart::new(id, kind, Ride::Rail(motion)), position)
    }

    #[test]
    fn a_cart_that_leaves_the_track_falls_slows_down_and_takes_a_rail_back() {
        // Un sol de pierre sous la voie, qui s'arrête au bout.
        let mut world = engine_voxel::World::new();
        let stone = engine_voxel::BlockRegistry::builtin()
            .id("core:stone")
            .unwrap();
        for x in -3..30 {
            for z in -3..3 {
                world.set_block(IVec3::new(x, -1, z), stone);
            }
        }
        let track = straight(6);
        let rails = |p: IVec3| track.get(&p).copied();
        let (mut cart, mut position) = on_rail(1, CartKind::Minecart, 2.5);
        if let Ride::Rail(m) = &mut cart.ride {
            m.speed = 8.0;
        }
        let drive = Drive::coasting(8.0);
        let mut derailed_at = None;
        for step in 0..600 {
            if cart.advance(&mut position, 1.0 / 60.0, &drive, &rails, &world, GRAVITY) {
                derailed_at = Some(step);
            }
        }
        assert!(derailed_at.is_some(), "il déraille au bout de la voie");
        assert!(
            !cart.on_rail(),
            "et ne reprend pas le rail qu'il vient de quitter"
        );
        assert!(
            cart.speed() < 0.1,
            "le frottement l'arrête : {}",
            cart.speed()
        );
        assert!((6.0..12.0).contains(&position.x), "{position:?}");
        assert!(position.y.abs() < 0.1, "posé sur le sol : {position:?}");
        // Poussé de nouveau vers la voie, il la reprend.
        cart.ride = Ride::Free {
            velocity: Vec3::new(-3.0, 0.0, 0.0),
        };
        cart.snap_delay = 0.0;
        position = Vec3::new(6.6, 0.0, 0.5);
        for _ in 0..30 {
            cart.advance(&mut position, 1.0 / 60.0, &drive, &rails, &world, GRAVITY);
        }
        assert!(cart.on_rail(), "{position:?}");
        assert!(
            position.x < 6.0 && (position.y - 0.06).abs() < 0.05,
            "{position:?}"
        );
    }

    #[test]
    fn a_locomotive_pulls_a_coupled_cart_and_the_two_keep_their_gap() {
        let world = engine_voxel::World::new();
        let track = straight(120);
        let rails = |p: IVec3| track.get(&p).copied();
        let (mut loco, mut loco_pos) = on_rail(1, CartKind::Locomotive, 10.5);
        let (mut wagon, mut wagon_pos) = on_rail(2, CartKind::Chest, 9.2);
        loco.links = vec![2];
        wagon.links = vec![1];
        let thrust = Drive {
            thrust: LOCOMOTIVE_THRUST,
            ..Drive::coasting(8.0)
        };
        let coast = Drive::coasting(8.0);
        let mut worst = 0.0f32;
        for _ in 0..480 {
            loco.advance(&mut loco_pos, 1.0 / 60.0, &thrust, &rails, &world, GRAVITY);
            wagon.advance(&mut wagon_pos, 1.0 / 60.0, &coast, &rails, &world, GRAVITY);
            for _ in 0..4 {
                if let (Ride::Rail(a), Ride::Rail(b)) = (&mut loco.ride, &mut wagon.ride) {
                    couple(a, b, COUPLING_LENGTH, &rails);
                }
            }
            if let (Ride::Rail(a), Ride::Rail(b)) = (&loco.ride, &wagon.ride) {
                worst = worst.max(((a.position() - b.position()).length() - COUPLING_LENGTH).abs());
                loco_pos = a.position();
                wagon_pos = b.position();
            }
        }
        assert!(worst < 0.1, "écart : {worst}");
        assert!(wagon_pos.x > 30.0, "tiré par la locomotive : {wagon_pos:?}");
        assert!(wagon.speed() > 5.0, "{}", wagon.speed());
    }

    #[test]
    fn couplings_toggle_both_ways_and_names_are_recognised() {
        let mut world = World::new();
        install(
            &mut world,
            8.0,
            Arc::new(ItemRegistry::builtin(&engine_voxel::BlockRegistry::builtin()).unwrap()),
            Arc::new(Smelting::default()),
        );
        let motion = RailMotion::on(IVec3::ZERO, RailShape::EastWest, 0.5);
        let a = spawn(
            &mut world,
            CartKind::Minecart,
            Vec3::ZERO,
            Ride::Rail(motion),
        );
        let b = spawn(&mut world, CartKind::Chest, Vec3::X, Ride::Rail(motion));
        assert_eq!(toggle_coupling(&world, a, b), Some(true));
        assert_eq!(world.get::<Cart>(a).unwrap().links, vec![2]);
        assert_eq!(world.get::<Cart>(b).unwrap().links, vec![1]);
        assert_eq!(toggle_coupling(&world, a, b), Some(false));
        assert!(world.get::<Cart>(a).unwrap().links.is_empty());
        assert_eq!(toggle_coupling(&world, a, a), None);
        assert_eq!(CartKind::parse("hopper"), Some(CartKind::Hopper));
        assert_eq!(
            CartKind::parse("core:locomotive"),
            Some(CartKind::Locomotive)
        );
        assert_eq!(CartKind::parse("chest_cart"), Some(CartKind::Chest));
        assert_eq!(CartKind::parse("barque"), None);
    }

    #[test]
    fn carts_survive_a_save_and_load_with_their_cargo_links_and_fuel() {
        let items =
            Arc::new(ItemRegistry::builtin(&engine_voxel::BlockRegistry::builtin()).unwrap());
        let mut world = World::new();
        install(
            &mut world,
            8.0,
            items.clone(),
            Arc::new(Smelting::default()),
        );
        let mut motion = RailMotion::on(IVec3::new(3, 70, -2), RailShape::EastWest, 0.25);
        motion.speed = -3.5;
        motion.heading = -1.0;
        let a = spawn(
            &mut world,
            CartKind::Locomotive,
            Vec3::new(3.25, 70.06, -1.5),
            Ride::Rail(motion),
        );
        let b = spawn(
            &mut world,
            CartKind::Chest,
            Vec3::new(8.0, 65.0, 1.0),
            Ride::Free {
                velocity: Vec3::new(1.0, -2.0, 0.5),
            },
        );
        toggle_coupling(&world, a, b);
        world.get_mut::<Cart>(a).unwrap().burn = 12.5;
        world
            .get_mut::<CartCargo>(a)
            .unwrap()
            .0
            .insert(None, "core:coal", 5, false);
        world
            .get_mut::<CartCargo>(b)
            .unwrap()
            .0
            .insert(None, "core:stone", 70, false);

        let saved = to_saved(&world);
        let dir = std::env::temp_dir().join(format!("voxel-cart-game-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        engine_voxel::save_carts(&dir, &saved).unwrap();
        let loaded = engine_voxel::load_carts(&dir).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(loaded, saved);

        let mut fresh = World::new();
        install(&mut fresh, 8.0, items, Arc::new(Smelting::default()));
        load_saved(&mut fresh, &loaded);
        assert_eq!(fresh.resource::<CartConfig>().next_id, 3);
        // Le même état, rangé de nouveau, donne la même sauvegarde (sauf la forme du rail, relue
        // au premier pas).
        let again = to_saved(&fresh);
        assert_eq!(again.len(), 2);
        assert_eq!(again[0].links, vec![2]);
        assert_eq!(again[0].burn, 12.5);
        assert_eq!(again[0].rail, saved[0].rail);
        assert_eq!(again[0].items, saved[0].items);
        assert_eq!(again[1].velocity, saved[1].velocity);
        assert_eq!(again[1].items, saved[1].items);
        assert_eq!(again[1].kind, "chest_cart");
        assert_eq!(again[1].position, saved[1].position);
    }

    /// Un train tombé de la voie (wagons libres, tous au même endroit) reprend ses écarts
    /// d'attelage au lieu de rester entassé ; un mur l'empêche de traverser un bloc.
    #[test]
    fn a_derailed_train_spreads_back_to_its_coupling_gap() {
        let none = |_: IVec3| None;
        let open = |_: IVec3| false;
        let at = Vec3::new(10.5, 62.0, 0.5);
        let mut bodies = [
            Body::Free {
                position: at,
                velocity: Vec3::ZERO,
            },
            Body::Free {
                position: at + Vec3::X * 0.06,
                velocity: Vec3::ZERO,
            },
            Body::Free {
                position: at,
                velocity: Vec3::ZERO,
            },
        ];
        for _ in 0..60 {
            for _ in 0..4 {
                for (i, j) in [(0, 1), (1, 2)] {
                    let (left, right) = bodies.split_at_mut(j);
                    link(
                        &mut left[i],
                        &mut right[0],
                        (i as u32, j as u32),
                        COUPLING_LENGTH,
                        &none,
                        &open,
                    );
                }
            }
        }
        for (i, j) in [(0, 1), (1, 2)] {
            let gap = bodies[i].position().distance(bodies[j].position());
            assert!((gap - COUPLING_LENGTH).abs() < 0.1, "écart {i}-{j} : {gap}");
        }
        // Contre un mur : le wagon ne le traverse pas.
        let wall = |p: IVec3| p.x >= 11;
        let mut a = Body::Free {
            position: Vec3::new(10.5, 62.0, 0.5),
            velocity: Vec3::ZERO,
        };
        let mut b = Body::Free {
            position: Vec3::new(10.5, 62.0, 0.5),
            velocity: Vec3::ZERO,
        };
        for _ in 0..20 {
            push_apart(&mut a, &mut b, (2, 1), BUMP_GAP, &none, &wall);
        }
        assert!(a.position().x < 11.0 && b.position().x < 11.0);
    }
}
