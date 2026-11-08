//! La sauvegarde du monde sur disque.
//!
//! # Ce qu'on écrit
//!
//! Un dossier par monde :
//! - `world.ron` : les réglages du monde (graine, heure, joueur, mods), lisibles à la main ;
//! - `regions/r.<x>.<z>.bin` : les chunks, regroupés par « régions » de 16 × 16 colonnes.
//!   Un fichier par chunk ferait des milliers de petits fichiers, lents à ouvrir ;
//!   un seul fichier pour tout le monde obligerait à tout réécrire à chaque sauvegarde.
//!
//! Seuls les chunks **modifiés par le joueur** sont écrits : les autres se régénèrent à
//! l'identique depuis la graine. Un monde fraîchement créé ne pèse donc presque rien.
//! La lumière n'est pas écrite non plus : elle se recalcule depuis les blocs.
//!
//! # Jamais de numéro de bloc sur disque
//!
//! Le numéro d'un bloc ([`BlockId`]) dépend des mods installés et de leur ordre. Chaque
//! chunk écrit donc sa propre **palette** : la liste des identifiants texte
//! (`core:stone`, `example:ruby`...) qu'il contient. Les blocs sont stockés comme des
//! indices dans cette palette. Au chargement, on retraduit chaque identifiant avec le
//! registre du moment. Bonus : un chunk avec peu de types de blocs se compresse très bien.
//!
//! # Format binaire
//!
//! `serde` décrit les structures ; `postcard` les transforme en octets (compact :
//! un petit entier tient sur un octet) ; `lz4_flex` compresse chaque chunk (très rapide,
//! idéal pour une sauvegarde qui ne doit pas faire saccader le jeu).
//!
//! # Écriture atomique
//!
//! On écrit d'abord dans un fichier temporaire, puis on le renomme. Un renommage est
//! atomique : si le jeu plante au milieu, on garde l'ancien fichier entier plutôt
//! qu'un fichier à moitié écrit, donc illisible.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, bail};
use glam::{IVec2, IVec3};
use serde::{Deserialize, Serialize};

use crate::block::BlockId;
use crate::chunk::Chunk;
use crate::registry::BlockRegistry;

/// Version du format. À augmenter si la structure des fichiers change.
pub const FORMAT_VERSION: u32 = 1;
/// Côté d'une région, en colonnes de chunks.
pub const REGION_SIZE: i32 = 16;
const REGION_MAGIC: &[u8; 4] = b"VXRG";
/// Remplace un bloc dont le mod a disparu.
pub const UNKNOWN_BLOCK: &str = "core:unknown";

/// Le joueur tel qu'il est sauvegardé.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSave {
    pub position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    /// `"walk"` ou `"fly"`.
    pub mode: String,
    /// Identifiant texte du bloc choisi dans la barre.
    pub selected_block: Option<String>,
}

/// Le contenu de `world.ron`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldMeta {
    pub format_version: u32,
    pub seed: u32,
    /// L'heure, de 0 à 1.
    pub time_of_day: f32,
    pub player: Option<PlayerSave>,
    /// Les espaces de noms chargés lors de la sauvegarde (`core`, `example`...).
    pub mods: Vec<String>,
}

/// Un chunk sur disque : sa palette et ses blocs (indices dans la palette).
#[derive(Debug, Serialize, Deserialize)]
struct ChunkData {
    palette: Vec<String>,
    blocks: Vec<u16>,
}

/// Un fichier région : pour chaque chunk, sa position et ses données compressées.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RegionData {
    format_version: u32,
    chunks: Vec<([i32; 3], Vec<u8>)>,
}

/// Le contenu d'une région en mémoire : position du chunk → données compressées.
type RegionChunks = HashMap<IVec3, Vec<u8>>;

/// Les blocs inconnus d'un chunk : position dans le chunk ([`Chunk::index`]) → identifiant.
pub type UnknownBlocks = HashMap<usize, String>;

/// Un chunk relu depuis le disque.
#[derive(Debug)]
pub struct LoadedChunk {
    pub chunk: Chunk,
    /// Les blocs remplacés par `core:unknown`, avec leur vrai identifiant.
    pub unknown: UnknownBlocks,
}

/// Un chunk à écrire.
pub struct ChunkToSave {
    pub pos: IVec3,
    pub chunk: Arc<Chunk>,
    pub unknown: UnknownBlocks,
}

/// Le bilan d'une sauvegarde.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SaveReport {
    pub chunks: usize,
    pub regions: usize,
    /// Taille totale des régions écrites, en octets.
    pub bytes: u64,
}

/// L'accès au dossier d'un monde.
///
/// Partagé entre threads avec un `Arc` : les tâches de fond y lisent les chunks
/// pendant qu'une sauvegarde en écrit d'autres. Le cache des régions est donc protégé
/// par un `Mutex` : ici c'est le bon outil, car on le tient très peu de temps (une
/// recherche dans une `HashMap`) et rarement (une fois par chunk chargé).
#[derive(Debug)]
pub struct SaveStore {
    dir: PathBuf,
    /// Régions déjà lues : position du chunk → données compressées.
    cache: Mutex<HashMap<IVec2, Arc<RegionChunks>>>,
}

impl SaveStore {
    /// Ouvre (ou crée) le dossier d'un monde.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(dir.join("regions"))
            .with_context(|| format!("création de {}", dir.display()))?;
        Ok(Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn meta_path(&self) -> PathBuf {
        self.dir.join("world.ron")
    }

    /// Lit `world.ron`, ou `None` si le monde est neuf.
    pub fn load_meta(&self) -> Result<Option<WorldMeta>> {
        let path = self.meta_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)?;
        let meta: WorldMeta =
            ron::from_str(&text).with_context(|| format!("{} illisible", path.display()))?;
        if meta.format_version > FORMAT_VERSION {
            bail!(
                "{} vient d'une version plus récente du jeu (format {})",
                path.display(),
                meta.format_version
            );
        }
        Ok(Some(meta))
    }

    pub fn save_meta(&self, meta: &WorldMeta) -> Result<()> {
        let text = ron::ser::to_string_pretty(meta, ron::ser::PrettyConfig::default())?;
        write_atomic(&self.meta_path(), text.as_bytes())
    }

    /// La région qui contient ce chunk.
    pub fn region_of(chunk: IVec3) -> IVec2 {
        IVec2::new(
            chunk.x.div_euclid(REGION_SIZE),
            chunk.z.div_euclid(REGION_SIZE),
        )
    }

    fn region_path(&self, region: IVec2) -> PathBuf {
        self.dir
            .join("regions")
            .join(format!("r.{}.{}.bin", region.x, region.y))
    }

    /// Les chunks d'une région (depuis le cache, sinon le disque).
    fn region(&self, region: IVec2) -> Result<Arc<RegionChunks>> {
        // `lock()` renvoie une erreur seulement si un autre thread a paniqué en tenant
        // le verrou ; dans ce cas on récupère quand même les données.
        if let Some(found) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&region)
        {
            return Ok(found.clone());
        }
        let path = self.region_path(region);
        let chunks = if path.exists() {
            read_region(&path)?
        } else {
            HashMap::new()
        };
        let chunks = Arc::new(chunks);
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(region, chunks.clone());
        Ok(chunks)
    }

    /// Relit un chunk sauvegardé, ou `None` s'il n'a jamais été modifié.
    /// Appelé depuis les threads de fond.
    pub fn load_chunk(&self, pos: IVec3, registry: &BlockRegistry) -> Result<Option<LoadedChunk>> {
        let region = self.region(Self::region_of(pos))?;
        let Some(bytes) = region.get(&pos) else {
            return Ok(None);
        };
        decode_chunk(bytes, registry)
            .with_context(|| format!("chunk {pos} illisible"))
            .map(Some)
    }

    /// Écrit des chunks, en fusionnant avec ce que contiennent déjà leurs régions.
    pub fn save_chunks(
        &self,
        chunks: &[ChunkToSave],
        registry: &BlockRegistry,
    ) -> Result<SaveReport> {
        let mut by_region: HashMap<IVec2, Vec<&ChunkToSave>> = HashMap::new();
        for chunk in chunks {
            by_region
                .entry(Self::region_of(chunk.pos))
                .or_default()
                .push(chunk);
        }
        let mut report = SaveReport::default();
        for (region, chunks) in by_region {
            // On repart du contenu actuel de la région : on n'écrase que nos chunks.
            let mut content = (*self.region(region)?).clone();
            for chunk in chunks {
                content.insert(
                    chunk.pos,
                    encode_chunk(&chunk.chunk, &chunk.unknown, registry)?,
                );
                report.chunks += 1;
            }
            let bytes = write_region(&self.region_path(region), &content)?;
            report.regions += 1;
            report.bytes += bytes;
            self.cache
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(region, Arc::new(content));
        }
        Ok(report)
    }
}

/// Encode un chunk : palette + indices, sérialisés puis compressés.
fn encode_chunk(
    chunk: &Chunk,
    unknown: &UnknownBlocks,
    registry: &BlockRegistry,
) -> Result<Vec<u8>> {
    let mut palette: Vec<String> = Vec::new();
    let mut lookup: HashMap<String, u16> = HashMap::new();
    let mut blocks = Vec::with_capacity(chunk.blocks().len());
    for (i, &id) in chunk.blocks().iter().enumerate() {
        // Un bloc inconnu retrouve son vrai identifiant.
        let key = match unknown.get(&i) {
            Some(original) => original.as_str(),
            None => registry.def(id).key.as_str(),
        };
        let index = match lookup.get(key) {
            Some(&index) => index,
            None => {
                let index = u16::try_from(palette.len()).context("palette trop grande")?;
                palette.push(key.to_string());
                lookup.insert(key.to_string(), index);
                index
            }
        };
        blocks.push(index);
    }
    let bytes = postcard::to_allocvec(&ChunkData { palette, blocks })?;
    // `prepend_size` : la taille d'origine est écrite au début, pour la décompression.
    Ok(lz4_flex::compress_prepend_size(&bytes))
}

fn decode_chunk(bytes: &[u8], registry: &BlockRegistry) -> Result<LoadedChunk> {
    let raw = lz4_flex::decompress_size_prepended(bytes)?;
    let data: ChunkData = postcard::from_bytes(&raw)?;
    // Traduire la palette : identifiant texte → numéro du registre actuel.
    let fallback = registry.id(UNKNOWN_BLOCK).unwrap_or(BlockId::AIR);
    let ids: Vec<Option<BlockId>> = data.palette.iter().map(|key| registry.id(key)).collect();
    for (key, id) in data.palette.iter().zip(&ids) {
        if id.is_none() {
            log::warn!("bloc « {key} » inconnu (mod absent ?) : remplacé par {UNKNOWN_BLOCK}");
        }
    }
    let mut unknown = UnknownBlocks::new();
    let mut blocks = Vec::with_capacity(data.blocks.len());
    for (i, &index) in data.blocks.iter().enumerate() {
        let index = index as usize;
        let key = data
            .palette
            .get(index)
            .context("indice hors de la palette")?;
        match ids[index] {
            Some(id) => blocks.push(id),
            None => {
                blocks.push(fallback);
                unknown.insert(i, key.clone());
            }
        }
    }
    let chunk = Chunk::from_blocks(blocks).context("chunk de taille invalide")?;
    Ok(LoadedChunk { chunk, unknown })
}

fn read_region(path: &Path) -> Result<RegionChunks> {
    let bytes = std::fs::read(path).with_context(|| format!("lecture de {}", path.display()))?;
    let Some(body) = bytes.strip_prefix(REGION_MAGIC.as_slice()) else {
        bail!("{} n'est pas un fichier région", path.display());
    };
    let region: RegionData =
        postcard::from_bytes(body).with_context(|| format!("{} corrompu", path.display()))?;
    if region.format_version > FORMAT_VERSION {
        bail!("{} vient d'une version plus récente du jeu", path.display());
    }
    Ok(region
        .chunks
        .into_iter()
        .map(|(pos, data)| (IVec3::from_array(pos), data))
        .collect())
}

/// Écrit une région ; renvoie sa taille en octets.
fn write_region(path: &Path, chunks: &RegionChunks) -> Result<u64> {
    let mut entries: Vec<([i32; 3], Vec<u8>)> = chunks
        .iter()
        .map(|(pos, data)| (pos.to_array(), data.clone()))
        .collect();
    // Trier rend le fichier identique d'une sauvegarde à l'autre (la `HashMap` n'a pas d'ordre).
    entries.sort_by_key(|(pos, _)| *pos);
    let region = RegionData {
        format_version: FORMAT_VERSION,
        chunks: entries,
    };
    let mut bytes = REGION_MAGIC.to_vec();
    bytes.extend(postcard::to_allocvec(&region)?);
    write_atomic(path, &bytes)?;
    Ok(bytes.len() as u64)
}

/// Écrit `bytes` dans `path` sans jamais laisser un fichier à moitié écrit.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension("tmp");
    let mut file =
        std::fs::File::create(&tmp).with_context(|| format!("écriture de {}", tmp.display()))?;
    file.write_all(bytes)?;
    // `sync_all` attend que les octets soient vraiment sur le disque, pas seulement dans
    // le cache du système : sans ça, une coupure de courant juste après le renommage
    // pourrait laisser un fichier vide.
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path).with_context(|| format!("renommage vers {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::registry::test_blocks::{dirt, stone};

    /// Un dossier temporaire neuf, propre à ce test.
    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("voxel-save-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample_chunk() -> Chunk {
        let mut chunk = Chunk::new();
        for x in 0..32 {
            for z in 0..32 {
                chunk.set(x, 0, z, stone());
                chunk.set(x, 1, z, dirt());
            }
        }
        chunk.set(5, 9, 7, stone());
        chunk
    }

    #[test]
    fn chunk_round_trip_is_identical() {
        let registry = BlockRegistry::builtin();
        let store = SaveStore::open(temp_dir("round-trip")).unwrap();
        let chunk = sample_chunk();
        let pos = IVec3::new(-3, 1, 40);
        let report = store
            .save_chunks(
                &[ChunkToSave {
                    pos,
                    chunk: Arc::new(chunk.clone()),
                    unknown: UnknownBlocks::new(),
                }],
                &registry,
            )
            .unwrap();
        assert_eq!((report.chunks, report.regions), (1, 1));

        // Une nouvelle instance relit depuis le disque, sans le cache.
        let reopened = SaveStore::open(store.dir()).unwrap();
        let loaded = reopened.load_chunk(pos, &registry).unwrap().unwrap();
        assert_eq!(loaded.chunk.blocks(), chunk.blocks());
        assert!(loaded.unknown.is_empty());
        assert!(
            reopened
                .load_chunk(IVec3::new(-3, 0, 40), &registry)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn saving_one_chunk_keeps_the_others_of_the_region() {
        let registry = BlockRegistry::builtin();
        let store = SaveStore::open(temp_dir("merge")).unwrap();
        let save = |pos: IVec3| ChunkToSave {
            pos,
            chunk: Arc::new(sample_chunk()),
            unknown: UnknownBlocks::new(),
        };
        store
            .save_chunks(&[save(IVec3::new(0, 0, 0))], &registry)
            .unwrap();
        store
            .save_chunks(&[save(IVec3::new(1, 0, 0))], &registry)
            .unwrap();
        let reopened = SaveStore::open(store.dir()).unwrap();
        assert!(
            reopened
                .load_chunk(IVec3::new(0, 0, 0), &registry)
                .unwrap()
                .is_some()
        );
        assert!(
            reopened
                .load_chunk(IVec3::new(1, 0, 0), &registry)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn missing_mod_blocks_become_unknown_and_keep_their_id() {
        // Avec le mod : un rubis dans le chunk.
        let mut with_mod = BlockRegistry::builtin().as_ref().clone();
        with_mod
            .add_source(
                "example",
                r#"[(id: "example:ruby", name: "Rubis", color: (0.7, 0.0, 0.1))]"#,
                "test",
            )
            .unwrap();
        let ruby = with_mod.id("example:ruby").unwrap();
        let mut chunk = sample_chunk();
        chunk.set(3, 4, 5, ruby);
        let store = SaveStore::open(temp_dir("missing-mod")).unwrap();
        let pos = IVec3::ZERO;
        let save = |chunk: Chunk, unknown: UnknownBlocks| ChunkToSave {
            pos,
            chunk: Arc::new(chunk),
            unknown,
        };
        store
            .save_chunks(&[save(chunk, UnknownBlocks::new())], &with_mod)
            .unwrap();

        // Sans le mod : le rubis devient « inconnu », mais son identifiant est gardé.
        let without = BlockRegistry::builtin();
        let loaded = SaveStore::open(store.dir())
            .unwrap()
            .load_chunk(pos, &without)
            .unwrap()
            .unwrap();
        let unknown_id = without.id(UNKNOWN_BLOCK).unwrap();
        assert_eq!(loaded.chunk.get(3, 4, 5), unknown_id);
        assert_eq!(
            loaded
                .unknown
                .get(&Chunk::index(3, 4, 5))
                .map(String::as_str),
            Some("example:ruby")
        );

        // On resauvegarde sans le mod, puis on le réinstalle : le rubis revient.
        store
            .save_chunks(&[save(loaded.chunk, loaded.unknown)], &without)
            .unwrap();
        let back = SaveStore::open(store.dir())
            .unwrap()
            .load_chunk(pos, &with_mod)
            .unwrap()
            .unwrap();
        assert_eq!(back.chunk.get(3, 4, 5), ruby);
        assert!(back.unknown.is_empty());
    }

    #[test]
    fn atomic_write_replaces_the_whole_file() {
        let dir = temp_dir("atomic");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fichier.bin");
        write_atomic(&path, b"premier contenu, assez long").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        // Le fichier temporaire a disparu après le renommage.
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn meta_round_trip() {
        let store = SaveStore::open(temp_dir("meta")).unwrap();
        assert_eq!(store.load_meta().unwrap(), None);
        let meta = WorldMeta {
            format_version: FORMAT_VERSION,
            seed: 7,
            time_of_day: 0.3,
            player: Some(PlayerSave {
                position: [1.0, 2.0, 3.0],
                yaw: 0.5,
                pitch: -0.2,
                mode: "walk".into(),
                selected_block: Some("core:stone".into()),
            }),
            mods: vec!["core".into(), "example".into()],
        };
        store.save_meta(&meta).unwrap();
        assert_eq!(store.load_meta().unwrap(), Some(meta));
    }

    #[test]
    fn region_of_handles_negative_chunks() {
        assert_eq!(
            SaveStore::region_of(IVec3::new(-1, 3, 16)),
            IVec2::new(-1, 1)
        );
        assert_eq!(
            SaveStore::region_of(IVec3::new(15, 0, -16)),
            IVec2::new(0, -1)
        );
    }
}
