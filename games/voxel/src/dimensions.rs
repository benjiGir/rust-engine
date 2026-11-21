//! Les dimensions côté jeu : laquelle est ouverte, comment on passe de l'une à l'autre, ce
//! qu'elle change (ciel, gravité). La description des dimensions (`dimensions.ron`) et leur
//! générateur sont dans `engine_voxel::dimension`.
//!
//! **Passer une dimension, c'est recharger le monde** : le jeu sauvegarde ce qui est ouvert
//! (chaque dimension a son propre dossier de chunks dans celui du monde), note la dimension
//! d'arrivée dans `world.ron` (qui reste à la racine et porte le joueur : inventaire, vie,
//! faim, expérience), puis relance le monde par le chemin ordinaire du menu. Le joueur arrive
//! aux mêmes (x, z), au sol, et une dimension peut y poser un bloc d'arrivée (le portail de
//! retour). Les créatures, objets au sol et scripts repartent à neuf ; les données des mods
//! (`mod_data.ron`) sont celles du monde et se retrouvent.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use engine_voxel::{
    BiomeRegistry, BlockId, BlockRegistry, DimensionDef, DimensionRegistry, GenerationMeta,
    GenerationVersion, OVERWORLD, SaveStore, TerrainGenerator, WorldKind, WorldMeta,
};
use glam::{IVec3, Vec3};

use crate::VoxelGame;
use crate::launch::{Launch, Request};

/// Le temps qu'il faut rester dans un portail pour le franchir (secondes).
const PORTAL_TIME: f32 = 1.5;

/// Les dimensions d'un monde ouvert, et celle où l'on est.
pub struct Dimensions {
    pub registry: Arc<DimensionRegistry>,
    pub current: Arc<DimensionDef>,
    /// Le dossier racine du monde, où vit `world.ron` : le joueur y est sauvegardé quelle que
    /// soit la dimension. `None` pour un monde sans sauvegarde.
    pub meta_store: Option<Arc<SaveStore>>,
    /// La graine du monde (celle de `world.ron`) ; le générateur d'une dimension y ajoute son décalage.
    pub base_seed: u32,
    /// La version de génération et la configuration effective, telles qu'elles sont écrites dans
    /// `world.ron`. Pour un monde existant, c'est la **référence enregistrée** (celle de sa
    /// création) : elle n'est pas réécrite quand un mod change, pour que l'écart reste signalé.
    pub generation: GenerationMeta,
    /// La dimension vers laquelle on part : `world.ron` annonce l'arrivée, y compris dans la
    /// dernière sauvegarde que le jeu fait en se déchargeant.
    pub leaving: Option<String>,
    /// Le bloc d'arrivée à poser dès que son chunk est chargé : (case, bloc).
    pending_arrival: Option<(IVec3, BlockId)>,
    /// Le temps passé dans un portail, et s'il est armé (on n'enchaîne pas deux portails : après
    /// une arrivée il faut d'abord en sortir).
    portal_timer: f32,
    portal_armed: bool,
}

/// Ouvre les dimensions d'un monde : choisit la bonne (celle du lancement, sinon celle que
/// `world.ron` indique, sinon la dimension de départ) et son dossier de chunks.
pub fn open(
    blocks: &BlockRegistry,
    mods: &[(String, PathBuf)],
    root: Option<Arc<SaveStore>>,
    meta: Option<&WorldMeta>,
    wanted: Option<&str>,
) -> Result<(Dimensions, Option<Arc<SaveStore>>)> {
    open_versioned(
        blocks,
        mods,
        root,
        meta,
        wanted,
        GenerationVersion::for_new_world(),
    )
}

/// Comme [`open`], avec la version qu'aurait un monde neuf (`for_new_world()` : v3, ou v1 demandée par
/// `VOXEL_GENERATION=1`). Un monde enregistré garde la sienne.
pub fn open_versioned(
    blocks: &BlockRegistry,
    mods: &[(String, PathBuf)],
    root: Option<Arc<SaveStore>>,
    meta: Option<&WorldMeta>,
    wanted: Option<&str>,
    new_world_version: GenerationVersion,
) -> Result<(Dimensions, Option<Arc<SaveStore>>)> {
    let biomes = BiomeRegistry::load_mods(blocks, mods)?;
    let registry = match DimensionRegistry::load_mods(blocks, &biomes, mods) {
        Ok(registry) => registry,
        Err(err) => {
            log::error!("dimensions des mods refusées : {err:#}");
            DimensionRegistry::builtin()
        }
    };
    let key = wanted
        .map(str::to_string)
        .or_else(|| meta.map(|m| m.dimension.clone()).filter(|d| !d.is_empty()))
        .unwrap_or_else(|| OVERWORLD.to_string());
    let current = registry.get(&key).cloned().unwrap_or_else(|| {
        log::warn!("dimension « {key} » inconnue (mod retiré ?) : retour à la dimension de départ");
        registry.overworld().clone()
    });
    // Les chunks d'une dimension autre que celle de départ sont dans un sous-dossier du monde.
    let store = match (&root, &current.folder) {
        (Some(root), Some(folder)) => Some(Arc::new(SaveStore::open(root.dir().join(folder))?)),
        _ => root.clone(),
    };
    log::info!("dimension : {} ({})", current.name, current.key);
    let generation = open_generation(meta, &registry, blocks, &biomes, mods, new_world_version)?;
    let arriving = meta.is_some_and(|m| m.arriving);
    Ok((
        Dimensions {
            registry: Arc::new(registry),
            current: Arc::new(current),
            meta_store: root,
            base_seed: 0,
            generation,
            leaving: None,
            pending_arrival: None,
            portal_timer: 0.0,
            // Arrivé par un portail : il faut en sortir avant de pouvoir en reprendre un.
            portal_armed: !arriving,
        },
        store,
    ))
}

/// La version de génération et la configuration à enregistrer. Un monde neuf prend la version
/// des nouveaux mondes (v3, ou v1 si `VOXEL_GENERATION=1` : voir `GenerationVersion::for_new_world`) et
/// la configuration d'aujourd'hui ; un monde sans métadonnée de génération
/// (d'avant ce champ) reste en génération historique v1 et adopte la configuration d'aujourd'hui
/// comme référence ; un monde qui en a une la garde, et chaque écart avec aujourd'hui (mod retiré,
/// modifié ou ajouté) est signalé dans les logs.
fn open_generation(
    meta: Option<&WorldMeta>,
    dimensions: &DimensionRegistry,
    blocks: &BlockRegistry,
    biomes: &BiomeRegistry,
    mods: &[(String, PathBuf)],
    new_world_version: GenerationVersion,
) -> Result<GenerationMeta> {
    let version = match meta {
        Some(meta) => meta.generation_version()?,
        None => new_world_version,
    };
    log::info!("génération : {version}");
    let now = GenerationMeta::describe(version, dimensions, blocks, biomes, mods);
    let Some(saved) = meta.and_then(|m| m.generation.as_ref()) else {
        if meta.is_some() {
            log::info!("monde sans version de génération : génération historique {version}");
        }
        return Ok(now);
    };
    for issue in saved.compare(&now) {
        log::warn!("génération du monde : {issue}");
    }
    Ok(saved.clone())
}

impl Dimensions {
    /// Le joueur arrive par un portail : où le poser (au sol, aux mêmes x et z que dans l'autre
    /// dimension) et où poser le bloc d'arrivée, à deux blocs de lui.
    pub fn arrive(
        &mut self,
        eye: Vec3,
        meta: Option<&WorldMeta>,
        generator: &TerrainGenerator,
    ) -> Vec3 {
        if !meta.is_some_and(|m| m.arriving) {
            return eye;
        }
        log::info!("arrivée dans « {} »", self.current.name);
        let (x, z) = match self.current.kind {
            // Dans le vide, la plateforme est à l'origine.
            WorldKind::Void { .. } => (0, 0),
            _ => (eye.x.floor() as i32, eye.z.floor() as i32),
        };
        let ground = generator.column(x, z).height;
        if let Some(block) = self.current.arrival {
            let (bx, bz) = (x + 2, z);
            let height = generator.column(bx, bz).height;
            self.pending_arrival = Some((IVec3::new(bx, height + 1, bz), block));
        }
        Vec3::new(x as f32 + 0.5, ground as f32 + 1.0 + 1.62, z as f32 + 0.5)
    }
}

impl VoxelGame {
    /// Les noms des dimensions du monde, pour `ctl` et la console.
    pub(crate) fn dimension_names(&self) -> Vec<String> {
        self.server
            .dims
            .registry
            .iter()
            .map(|d| d.key.clone())
            .collect()
    }
}

impl crate::server_side::VoxelServer {
    /// À chaque pas : pose le bloc d'arrivée quand son chunk est là, et fait passer le portail où le
    /// joueur se tient depuis assez longtemps.
    pub(crate) fn update_portals(&mut self, dt: f32) {
        if let Some((cell, block)) = self.dims.pending_arrival {
            let loaded = self
                .streamer()
                .world()
                .chunk(engine_voxel::World::chunk_containing(cell.as_vec3()))
                .is_some();
            if loaded {
                self.dims.pending_arrival = None;
                let done = self.edit(cell, block);
                log::info!(
                    "bloc d'arrivée en {cell} : {}",
                    if done { "posé" } else { "refusé" }
                );
            }
        }
        let feet = crate::player::body(&self.ecs, self.players.entity()).position;
        let cells = [feet.floor().as_ivec3(), (feet + Vec3::Y).floor().as_ivec3()];
        let portal = {
            let streamer = self.streamer();
            cells
                .iter()
                .find_map(|c| self.registry.def(streamer.world().block(*c)).portal.clone())
        };
        match portal {
            None => {
                self.dims.portal_timer = 0.0;
                self.dims.portal_armed = true;
            }
            Some(target) if self.dims.portal_armed => {
                self.dims.portal_timer += dt;
                if self.dims.portal_timer >= PORTAL_TIME {
                    self.dims.portal_timer = 0.0;
                    self.travel(&target);
                }
            }
            Some(_) => {}
        }
    }

    /// Le ciel de la dimension (couleur et lumière fixes, ou le cycle du jour).
    pub(crate) fn update_dimension_sky(&mut self) {
        let sky = self.dims.current.sky.clone();
        self.day
            .set_dimension_sky(sky.color.map(Vec3::from), sky.daylight);
    }

    // (lot 4b) Méthodes du serveur.
    /// Passe dans une dimension : sauvegarde le monde ouvert, note l'arrivée dans `world.ron`, puis
    /// recharge le monde (voir le début de ce fichier). Refusé dans un monde sans sauvegarde.
    pub(crate) fn travel(&mut self, target: &str) -> bool {
        let Some(root) = self.dims.meta_store.clone() else {
            log::warn!("les dimensions demandent un monde sauvegardé (pas le monde jetable)");
            return false;
        };
        if target == self.dims.current.key {
            return false;
        }
        if self.dims.registry.get(target).is_none() {
            log::warn!("dimension « {target} » inconnue");
            return false;
        }
        log::info!("passage vers la dimension « {target} »");
        // Les chunks de la dimension quittée, puis le joueur : `save_now` écrit les deux, et le
        // `world.ron` définitif est celui-ci (position conservée, dimension et arrivée notées).
        self.dims.leaving = Some(target.to_string());
        self.save_now();
        // La fenêtre recharge le monde dans la nouvelle dimension (`take_travel`).
        self.travel = Some(Request::Play(Launch {
            world: Some(root.dir().to_path_buf()),
            seed: None,
            title: false,
            difficulty: None,
            dimension: Some(target.to_string()),
        }));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Arc<BlockRegistry> {
        BlockRegistry::builtin()
    }

    fn meta(dimension: &str, arriving: bool) -> WorldMeta {
        WorldMeta {
            format_version: engine_voxel::FORMAT_VERSION,
            seed: 1,
            time_of_day: 0.5,
            player: None,
            mods: Vec::new(),
            inventory: Vec::new(),
            items: None,
            difficulty: 1,
            dimension: dimension.into(),
            arriving,
            adventure: Default::default(),
            generation: None,
        }
    }

    #[test]
    fn a_world_without_a_dimension_starts_in_the_overworld_and_unknown_ones_fall_back() {
        let blocks = registry();
        let (dims, store) = open(&blocks, &[], None, None, None).unwrap();
        assert_eq!(dims.current.key, OVERWORLD);
        assert!(store.is_none(), "monde sans sauvegarde : pas de dossier");
        // Un mod retiré : sa dimension n'existe plus, on retombe sur celle de départ.
        let missing = meta("gone:mod_dimension", false);
        let (dims, _) = open(&blocks, &[], None, Some(&missing), None).unwrap();
        assert_eq!(dims.current.key, OVERWORLD);
        // La dimension demandée au lancement l'emporte sur celle du fichier.
        let (dims, _) = open(&blocks, &[], None, Some(&missing), Some(OVERWORLD)).unwrap();
        assert_eq!(dims.current.key, OVERWORLD);
    }

    #[test]
    fn new_worlds_are_v3_old_ones_stay_v1_and_the_saved_reference_is_kept() {
        let blocks = registry();
        // Monde neuf : la version des nouveaux mondes et la configuration d'aujourd'hui.
        let new_version = GenerationVersion::FOR_NEW_WORLDS;
        let (new, _) = open_versioned(&blocks, &[], None, None, None, new_version).unwrap();
        assert_eq!(
            new.generation.version,
            GenerationVersion::FOR_NEW_WORLDS.number()
        );
        assert_eq!(new.generation.dimensions.len(), 1);
        assert_eq!(new_version, GenerationVersion::V3);
        // Un monde d'avant le champ : génération historique (v1), configuration d'aujourd'hui.
        let old = meta(OVERWORLD, false);
        let (dims, _) = open_versioned(&blocks, &[], None, Some(&old), None, new_version).unwrap();
        assert_eq!(dims.generation.version, GenerationVersion::V1.number());
        assert_eq!(dims.generation.dimensions, new.generation.dimensions);
        assert_eq!(dims.generation.data, new.generation.data);
        // Un monde qui a une référence la garde, même si elle ne correspond plus.
        let mut saved = new.generation.clone();
        saved.data[0].hash ^= 1;
        let mut kept = meta(OVERWORLD, false);
        kept.generation = Some(saved.clone());
        let (dims, _) = open_versioned(&blocks, &[], None, Some(&kept), None, new_version).unwrap();
        assert_eq!(dims.generation, saved);
        // Une version inconnue (monde d'un jeu plus récent) est refusée.
        let mut future = meta(OVERWORLD, false);
        future.generation = Some(GenerationMeta {
            version: 99,
            ..saved
        });
        assert!(open_versioned(&blocks, &[], None, Some(&future), None, new_version).is_err());
    }

    /// L'aiguillage par `world.ron` : un monde neuf est enregistré en v3 et s'y rouvre ; un monde v1 ou
    /// sans métadonnée de génération reste v1 ; un monde v2 (prototype retiré) est refusé.
    #[test]
    fn the_saved_version_decides_which_generator_a_world_opens_with() {
        let blocks = registry();
        let v3 = GenerationVersion::V3;
        // Un monde neuf : v3, et `world.ron` l'écrira.
        let (new, _) = open_versioned(&blocks, &[], None, None, None, v3).unwrap();
        assert_eq!(new.generation.generation_version().unwrap(), v3);
        // Rouvert (même si la variable demande v1) : il reste v3.
        let mut reopened = meta(OVERWORLD, false);
        reopened.generation = Some(new.generation.clone());
        let (dims, _) = open_versioned(
            &blocks,
            &[],
            None,
            Some(&reopened),
            None,
            GenerationVersion::V1,
        )
        .unwrap();
        assert_eq!(dims.generation.generation_version().unwrap(), v3);
        // Un monde d'avant le champ, ou enregistré en v1, reste v1 même si les nouveaux sont en v3.
        let old = meta(OVERWORLD, false);
        let (dims, _) = open_versioned(&blocks, &[], None, Some(&old), None, v3).unwrap();
        assert_eq!(
            dims.generation.generation_version().unwrap(),
            GenerationVersion::V1
        );
        // Un monde v2 : refusé, avec une explication.
        let mut retired = meta(OVERWORLD, false);
        retired.generation = Some(GenerationMeta {
            version: 2,
            ..new.generation.clone()
        });
        let err = open_versioned(&blocks, &[], None, Some(&retired), None, v3)
            .err()
            .expect("un monde v2 ne s'ouvre plus");
        assert!(format!("{err:#}").contains("v2"), "{err:#}");
        // Les dossiers temporaires : `world.ron` écrit puis relu garde la version.
        let root = std::env::temp_dir().join(format!("voxel-genver-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Arc::new(SaveStore::open(&root).unwrap());
        let mut saved_meta = meta(OVERWORLD, false);
        saved_meta.generation = Some(new.generation.clone());
        store.save_meta(&saved_meta).unwrap();
        let loaded = store.load_meta().unwrap().unwrap();
        assert_eq!(loaded.generation_version().unwrap(), v3);
        // Le générateur construit pour cette version est bien v3.
        let generator = crate::modding::terrain(3, &blocks, &[], &new.current, v3).unwrap();
        assert!(generator.v3_parameters(0, 0).is_some(), "générateur v3");
        let v1 =
            crate::modding::terrain(3, &blocks, &[], &new.current, GenerationVersion::V1).unwrap();
        assert!(v1.v3_parameters(0, 0).is_none(), "générateur v1");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn each_dimension_has_its_own_chunk_folder_under_the_world() {
        let blocks = registry();
        let root = std::env::temp_dir().join(format!("voxel-dims-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = Arc::new(SaveStore::open(&root).unwrap());
        // La dimension de départ garde la racine ; une autre aurait `dimensions/<nom>`.
        let (_, chunks) = open(&blocks, &[], Some(store.clone()), None, None).unwrap();
        assert_eq!(chunks.unwrap().dir(), store.dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn arriving_puts_the_player_on_the_ground_at_the_same_x_and_z() {
        let blocks = registry();
        let (mut dims, _) = open(&blocks, &[], None, None, None).unwrap();
        let generator = TerrainGenerator::new(5);
        let eye = Vec3::new(10.5, 200.0, -7.5);
        // Pas d'arrivée : la position sauvegardée est gardée.
        assert_eq!(dims.arrive(eye, Some(&meta("", false)), &generator), eye);
        let landed = dims.arrive(eye, Some(&meta("", true)), &generator);
        let ground = generator.column(10, -8).height as f32;
        assert_eq!((landed.x, landed.z), (10.5, -7.5));
        assert!((landed.y - (ground + 1.0 + 1.62)).abs() < 1e-3, "{landed}");
    }
}
