//! Les arènes GPU : la géométrie des chunks vit dans quelques grands tampons (des
//! « pages ») au lieu de trois tampons par chunk.
//!
//! - [`RangeAllocator`] : un allocateur de plages (meilleur ajustement, fusion des plages
//!   libres voisines). Du pur calcul, testé sans GPU.
//! - [`GeometryArena`] : des pages de sommets et d'indices. Un maillage occupe une plage
//!   de sommets et une plage d'indices **dans la même page** ; on le dessine avec
//!   `base_vertex` et la plage d'indices, sans rien rebrancher tant que la page ne change pas.
//! - [`OriginTable`] : l'origine, le lacet et la base des textures de chaque chunk (32 octets),
//!   dans un seul tampon lu comme des instances : le chunk `n` se dessine avec
//!   `first_instance = n`.
//!
//! Les plages libérées ne sont réutilisées qu'après quelques images ([`REUSE_DELAY`]) : le GPU
//! peut encore lire les images précédentes. Les pages entièrement vides sont rendues
//! (sauf une de réserve) : la mémoire suit l'exploration au lieu de croître.
//!
//! # Défragmentation
//!
//! Quand le joueur avance, les chunks déchargés derrière lui laissent des trous dans toutes les
//! pages, et une page n'est rendue que vide : on a mesuré des arènes pleines à moins de 50 %
//! (1 Gio de maillages dans 2,2 Gio de pages). [`GeometryArena::drain_target`] choisit alors la
//! page la moins remplie, n'y place plus rien, et [`GeometryArena::relocate`] déplace ses
//! maillages dans les trous des autres pages par une copie GPU → GPU (aucun aller-retour par le
//! processeur), quelques Mio par image. Vidée, la page est rendue comme les autres.

use std::collections::BTreeMap;

use bytemuck::Pod;

use crate::mesh::ColorVertex;

/// Sommets par page (24 octets chacun : 6 Mio).
pub const PAGE_VERTICES: u32 = 1 << 18;
/// Indices par page (4 octets chacun : 2 Mio) ; un quad coûte 4 sommets et 6 indices.
pub const PAGE_INDICES: u32 = 1 << 19;
/// Images d'attente avant de réutiliser une plage libérée.
pub const REUSE_DELAY: u64 = 3;
/// On ne vide une page que si elle est remplie à moins de cette fraction...
const DRAIN_MAX_FILL: f32 = 0.6;
/// ... et si les autres pages ont au moins ce multiple de sa place occupée de libre (les trous
/// sont de tailles variées : il faut de la marge pour tout caser).
const DRAIN_ROOM_FACTOR: u64 = 2;
/// Octets déplacés au plus par image.
pub const DEFRAG_BYTES_PER_FRAME: u64 = 4 << 20;
/// Entrées de la table d'origines au départ ; elle double quand elle est pleine.
const INITIAL_ORIGINS: u32 = 1024;

/// Un allocateur de plages d'entiers : `alloc(n)` renvoie le début d'une plage libre de
/// `n` éléments, `free(start, n)` la rend. Les plages libres voisines fusionnent.
#[derive(Debug, Clone)]
pub struct RangeAllocator {
    capacity: u32,
    /// Plages libres : début → longueur.
    free: BTreeMap<u32, u32>,
    used: u32,
}

impl RangeAllocator {
    pub fn new(capacity: u32) -> Self {
        let mut free = BTreeMap::new();
        if capacity > 0 {
            free.insert(0, capacity);
        }
        Self {
            capacity,
            free,
            used: 0,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    /// Éléments occupés.
    pub fn used(&self) -> u32 {
        self.used
    }

    /// La plus grande plage libre d'un seul tenant.
    pub fn largest_free(&self) -> u32 {
        self.free.values().copied().max().unwrap_or(0)
    }

    /// Nombre de plages libres (1 : aucune fragmentation).
    #[cfg(test)]
    pub fn free_ranges(&self) -> usize {
        self.free.len()
    }

    pub fn can_alloc(&self, len: u32) -> bool {
        len == 0 || self.largest_free() >= len
    }

    /// Meilleur ajustement : la plus petite plage libre qui contient `len` (les grandes
    /// plages restent entières pour les gros maillages).
    pub fn alloc(&mut self, len: u32) -> Option<u32> {
        if len == 0 {
            return Some(0);
        }
        let (&start, &size) = self
            .free
            .iter()
            .filter(|(_, size)| **size >= len)
            .min_by_key(|(start, size)| (**size, **start))?;
        self.free.remove(&start);
        if size > len {
            self.free.insert(start + len, size - len);
        }
        self.used += len;
        Some(start)
    }

    /// Rend une plage obtenue par `alloc`. Une plage vide ne fait rien.
    pub fn free(&mut self, start: u32, len: u32) {
        if len == 0 {
            return;
        }
        debug_assert!(start + len <= self.capacity);
        self.used -= len;
        let mut start = start;
        let mut len = len;
        if let Some((&before, &size)) = self.free.range(..start).next_back()
            && before + size == start
        {
            self.free.remove(&before);
            start = before;
            len += size;
        }
        if let Some(&size) = self.free.get(&(start + len)) {
            self.free.remove(&(start + len));
            len += size;
        }
        self.free.insert(start, len);
    }
}

/// Où se trouve un maillage dans l'arène.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slice {
    pub page: u32,
    /// Premier sommet (c'est le `base_vertex` du dessin).
    pub first_vertex: u32,
    pub vertex_count: u32,
    /// Premier indice, dans le tampon d'indices de la page.
    pub first_index: u32,
    pub index_count: u32,
}

/// Une page : un tampon de sommets, un d'indices, et leurs plages libres.
pub struct Page {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    vertices: RangeAllocator,
    indices: RangeAllocator,
}

impl Page {
    fn new(device: &wgpu::Device, vertex_capacity: u32, index_capacity: u32) -> Self {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arène : sommets"),
            size: u64::from(vertex_capacity) * size_of::<ColorVertex>() as u64,
            // `COPY_SRC` : la défragmentation copie des maillages d'une page à l'autre.
            usage: wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arène : indices"),
            size: u64::from(index_capacity) * 4,
            usage: wgpu::BufferUsages::INDEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        Self {
            vertex_buffer,
            index_buffer,
            vertices: RangeAllocator::new(vertex_capacity),
            indices: RangeAllocator::new(index_capacity),
        }
    }

    fn is_empty(&self) -> bool {
        self.vertices.used() == 0 && self.indices.used() == 0
    }

    /// Le remplissage, de 0 à 1 : celui des sommets ou des indices, le plus haut.
    fn fill(&self) -> f32 {
        let ratio = |a: &RangeAllocator| a.used() as f32 / a.capacity().max(1) as f32;
        ratio(&self.vertices).max(ratio(&self.indices))
    }

    fn fits(&self, vertex_count: u32, index_count: u32) -> bool {
        self.vertices.can_alloc(vertex_count) && self.indices.can_alloc(index_count)
    }
}

/// Ce que l'arène occupe, pour les statistiques et les tests de fuite.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ArenaStats {
    pub pages: usize,
    /// Octets réservés au GPU (sommets + indices), toutes pages confondues.
    pub reserved_bytes: u64,
    /// Octets occupés par des maillages vivants.
    pub used_bytes: u64,
    /// Plages libérées qui attendent leur réutilisation.
    pub pending_ranges: usize,
    /// Entrées de la table d'origines occupées (ou en attente de réutilisation).
    pub origin_slots: usize,
    /// Octets déplacés par la défragmentation depuis le début.
    pub defrag_bytes: u64,
}

struct Pending {
    free_at: u64,
    slice: Slice,
}

/// Les pages de géométrie de tous les chunks.
pub struct GeometryArena {
    pages: Vec<Option<Page>>,
    pending: Vec<Pending>,
    frame: u64,
    /// La page qu'on vide : plus aucun maillage n'y est placé.
    draining: Option<u32>,
    defrag_bytes: u64,
}

impl Default for GeometryArena {
    fn default() -> Self {
        Self::new()
    }
}

impl GeometryArena {
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            pending: Vec::new(),
            frame: 0,
            draining: None,
            defrag_bytes: 0,
        }
    }

    pub fn page(&self, index: u32) -> Option<&Page> {
        self.pages.get(index as usize)?.as_ref()
    }

    /// Place un maillage et envoie ses octets. `indices` sont relatifs au premier sommet
    /// du maillage (le dessin ajoute `base_vertex`).
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        vertices: &[ColorVertex],
        indices: &[u32],
    ) -> Slice {
        let vertex_count = vertices.len() as u32;
        let index_count = indices.len() as u32;
        let found = self.find_page(vertex_count, index_count, None);
        let index = match found {
            Some(i) => i,
            None => {
                // Une page neuve ; un maillage plus grand que la norme a la sienne.
                let page = Page::new(
                    device,
                    PAGE_VERTICES.max(vertex_count),
                    PAGE_INDICES.max(index_count),
                );
                match self.pages.iter().position(Option::is_none) {
                    Some(free) => {
                        self.pages[free] = Some(page);
                        free
                    }
                    None => {
                        self.pages.push(Some(page));
                        self.pages.len() - 1
                    }
                }
            }
        };
        let page = self.pages[index].as_mut().expect("page vivante");
        let first_vertex = page.vertices.alloc(vertex_count).expect("plage de sommets");
        let first_index = page.indices.alloc(index_count).expect("plage d'indices");
        uploads.write(
            device,
            &page.vertex_buffer,
            u64::from(first_vertex) * size_of::<ColorVertex>() as u64,
            bytemuck::cast_slice(vertices),
        );
        uploads.write(
            device,
            &page.index_buffer,
            u64::from(first_index) * 4,
            bytemuck::cast_slice(indices),
        );
        Slice {
            page: index as u32,
            first_vertex,
            vertex_count,
            first_index,
            index_count,
        }
    }

    /// Une page existante (ni celle qu'on vide, ni `except`) où ce maillage tient.
    fn find_page(&self, vertex_count: u32, index_count: u32, except: Option<u32>) -> Option<usize> {
        self.pages
            .iter()
            .enumerate()
            .filter(|&(i, _)| Some(i as u32) != self.draining && Some(i as u32) != except)
            .find(|(_, page)| {
                page.as_ref()
                    .is_some_and(|p| p.fits(vertex_count, index_count))
            })
            .map(|(i, _)| i)
    }

    /// La page à vider, s'il y en a une qui vaut la peine : la moins remplie (sous
    /// [`DRAIN_MAX_FILL`]), quand les autres ont largement la place de reprendre ses maillages.
    /// Le choix est gardé jusqu'à ce qu'elle soit vide ou qu'on n'arrive plus à la vider.
    pub fn drain_target(&mut self) -> Option<u32> {
        if let Some(page) = self.draining {
            match self.pages.get(page as usize) {
                Some(Some(p)) if p.vertices.used() > 0 || p.indices.used() > 0 => {
                    return Some(page);
                }
                _ => self.draining = None,
            }
        }
        // Seulement quand rien n'attend : une plage en attente compte encore comme occupée.
        let vertex = size_of::<ColorVertex>() as u64;
        let used =
            |p: &Page| u64::from(p.vertices.used()) * vertex + u64::from(p.indices.used()) * 4;
        let reserved = |p: &Page| {
            u64::from(p.vertices.capacity()) * vertex + u64::from(p.indices.capacity()) * 4
        };
        let (index, page) = self
            .pages
            .iter()
            .enumerate()
            .filter_map(|(i, p)| Some((i, p.as_ref()?)))
            // Une page vide (la réserve) ou surdimensionnée (un seul gros maillage) : rien à faire.
            .filter(|(_, p)| !p.is_empty() && p.vertices.capacity() == PAGE_VERTICES)
            .min_by(|a, b| a.1.fill().total_cmp(&b.1.fill()))?;
        if page.fill() > DRAIN_MAX_FILL {
            return None;
        }
        let room: u64 = self
            .pages
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != index)
            .filter_map(|(_, p)| p.as_ref())
            .map(|p| reserved(p) - used(p))
            .sum();
        if room < used(page) * DRAIN_ROOM_FACTOR {
            return None;
        }
        self.draining = Some(index as u32);
        self.draining
    }

    /// On n'arrive plus à placer les maillages de la page qu'on vide : on arrête (on recommencera
    /// quand d'autres trous se seront ouverts).
    pub fn stop_draining(&mut self) {
        self.draining = None;
    }

    /// Déplace un maillage dans une autre page existante (jamais une neuve) : copie GPU → GPU
    /// enregistrée dans `uploads`, l'ancienne place libérée après [`REUSE_DELAY`] images.
    /// `None` s'il ne tient nulle part ailleurs.
    pub fn relocate(&mut self, uploads: &mut Uploads, slice: Slice) -> Option<Slice> {
        let target = self.find_page(slice.vertex_count, slice.index_count, Some(slice.page))?;
        let page = self.pages[target].as_mut()?;
        let first_vertex = page.vertices.alloc(slice.vertex_count)?;
        let first_index = page.indices.alloc(slice.index_count)?;
        let moved = Slice {
            page: target as u32,
            first_vertex,
            vertex_count: slice.vertex_count,
            first_index,
            index_count: slice.index_count,
        };
        let (Some(Some(from)), Some(Some(to))) =
            (self.pages.get(slice.page as usize), self.pages.get(target))
        else {
            return None;
        };
        let vertex = size_of::<ColorVertex>() as u64;
        uploads.copy(
            &from.vertex_buffer,
            u64::from(slice.first_vertex) * vertex,
            &to.vertex_buffer,
            u64::from(first_vertex) * vertex,
            u64::from(slice.vertex_count) * vertex,
        );
        uploads.copy(
            &from.index_buffer,
            u64::from(slice.first_index) * 4,
            &to.index_buffer,
            u64::from(first_index) * 4,
            u64::from(slice.index_count) * 4,
        );
        self.defrag_bytes +=
            u64::from(slice.vertex_count) * vertex + u64::from(slice.index_count) * 4;
        self.free(slice);
        Some(moved)
    }

    /// Libère un maillage : ses plages ne sont réutilisées qu'après [`REUSE_DELAY`] images.
    pub fn free(&mut self, slice: Slice) {
        self.pending.push(Pending {
            free_at: self.frame + REUSE_DELAY,
            slice,
        });
    }

    /// À appeler une fois par image : rend les plages arrivées à échéance et les pages vides
    /// (sauf une de réserve).
    pub fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|p| p.free_at <= frame);
        self.pending = waiting;
        for Pending { slice, .. } in due {
            if let Some(Some(page)) = self.pages.get_mut(slice.page as usize) {
                page.vertices.free(slice.first_vertex, slice.vertex_count);
                page.indices.free(slice.first_index, slice.index_count);
            }
        }
        // Une page vide est gardée en réserve ; les autres sont rendues au GPU. Une page
        // dont des plages attendent encore n'est jamais vide : leur compteur d'usage les
        // compte jusqu'à la libération effective.
        let mut spare_kept = false;
        for page in &mut self.pages {
            if page.as_ref().is_some_and(Page::is_empty) {
                if spare_kept {
                    *page = None;
                } else {
                    spare_kept = true;
                }
            }
        }
        while self.pages.last().is_some_and(Option::is_none) {
            self.pages.pop();
        }
    }

    /// Libère tout (changement de monde). Les plages en attente sont oubliées avec les pages.
    pub fn clear(&mut self) {
        self.pages.clear();
        self.pending.clear();
        self.draining = None;
    }

    pub fn stats(&self) -> ArenaStats {
        let vertex = size_of::<ColorVertex>() as u64;
        let mut stats = ArenaStats {
            pending_ranges: self.pending.len(),
            defrag_bytes: self.defrag_bytes,
            ..ArenaStats::default()
        };
        for page in self.pages.iter().flatten() {
            stats.pages += 1;
            stats.reserved_bytes += u64::from(page.vertices.capacity()) * vertex
                + u64::from(page.indices.capacity()) * 4;
            stats.used_bytes +=
                u64::from(page.vertices.used()) * vertex + u64::from(page.indices.used()) * 4;
        }
        stats
    }
}

/// Une entrée de la table d'origines : origine du chunk (xyz) et lacet (w), puis base des
/// textures (xyz) et échelle (w). Même disposition qu'avant (deux `vec4`).
pub type OriginEntry = [[f32; 4]; 2];

/// La table des origines de tous les chunks : un tampon, une entrée par maillage.
pub struct OriginTable {
    buffer: wgpu::Buffer,
    shadow: Vec<OriginEntry>,
    free: Vec<u32>,
    /// Entrées libérées, réutilisables à l'échéance.
    pending: Vec<(u64, u32)>,
    frame: u64,
}

impl OriginTable {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            buffer: Self::buffer(device, INITIAL_ORIGINS),
            shadow: vec![[[0.0; 4]; 2]; INITIAL_ORIGINS as usize],
            free: (0..INITIAL_ORIGINS).rev().collect(),
            pending: Vec::new(),
            frame: 0,
        }
    }

    fn buffer(device: &wgpu::Device, capacity: u32) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("table des origines"),
            size: u64::from(capacity) * size_of::<OriginEntry>() as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn buffer_ref(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Entrées occupées ou en attente de réutilisation.
    pub fn in_use(&self) -> usize {
        self.shadow.len() - self.free.len()
    }

    /// Réserve une entrée et l'écrit. La table double si elle est pleine (copie de l'ombre
    /// côté processeur : les images en vol gardent l'ancien tampon vivant).
    pub fn alloc(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        entry: OriginEntry,
    ) -> u32 {
        if self.free.is_empty() {
            let old = self.shadow.len() as u32;
            let new = old * 2;
            self.shadow.resize(new as usize, [[0.0; 4]; 2]);
            self.free.extend((old..new).rev());
            self.buffer = Self::buffer(device, new);
            uploads.write(device, &self.buffer, 0, bytemuck::cast_slice(&self.shadow));
        }
        let slot = self.free.pop().expect("entrée libre");
        self.write(device, uploads, slot, entry);
        slot
    }

    pub fn entry(&self, slot: u32) -> OriginEntry {
        self.shadow[slot as usize]
    }

    pub fn write(
        &mut self,
        device: &wgpu::Device,
        uploads: &mut Uploads,
        slot: u32,
        entry: OriginEntry,
    ) {
        self.shadow[slot as usize] = entry;
        uploads.write(
            device,
            &self.buffer,
            u64::from(slot) * size_of::<OriginEntry>() as u64,
            bytemuck::bytes_of(&entry),
        );
    }

    pub fn free(&mut self, slot: u32) {
        self.pending.push((self.frame + REUSE_DELAY, slot));
    }

    pub fn begin_frame(&mut self) {
        self.frame += 1;
        let frame = self.frame;
        let mut i = 0;
        while i < self.pending.len() {
            if self.pending[i].0 <= frame {
                let (_, slot) = self.pending.swap_remove(i);
                self.free.push(slot);
            } else {
                i += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.free = (0..self.shadow.len() as u32).rev().collect();
        self.pending.clear();
    }
}

/// Taille d'un tampon de transit : plus grande que le budget d'envoi d'une image (3 Mio)
/// et que presque tous les maillages (un plus gros a son propre tampon, rendu ensuite).
const STAGING_CHUNK: u64 = 4 << 20;
/// Au-delà de ces octets en attente, on envoie sans attendre la prochaine image : une fenêtre
/// cachée (aucune image dessinée) ne doit pas accumuler des tampons de transit.
const FLUSH_BYTES: u64 = 16 << 20;

/// Les envois de géométrie vers le GPU, regroupés dans des tampons de transit **réutilisés**.
///
/// `queue.write_buffer` crée un tampon de transit neuf à chaque appel. Pour les centaines de
/// maillages envoyés chaque seconde en explorant, le pilote Metal garde ces tampons détruits
/// en cache avant de rendre leur mémoire : on a mesuré plus de 2 Gio de tampons morts
/// (des dizaines de milliers de régions « IOAccelerator »). Une `StagingBelt` recycle
/// quelques tampons de 4 Mio à la place : la mémoire de transit reste à quelques dizaines de Mio.
pub struct Uploads {
    belt: wgpu::util::StagingBelt,
    belt_device: wgpu::Device,
    encoder: Option<wgpu::CommandEncoder>,
    pending: u64,
}

impl Uploads {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            belt: wgpu::util::StagingBelt::new(device.clone(), STAGING_CHUNK),
            belt_device: device.clone(),
            encoder: None,
            pending: 0,
        }
    }

    /// Copie `bytes` dans `target` à `offset`, à la prochaine [`Uploads::flush`].
    /// `offset` et la taille doivent être des multiples de 4 (sommets et indices le sont).
    pub fn write(
        &mut self,
        device: &wgpu::Device,
        target: &wgpu::Buffer,
        offset: u64,
        bytes: &[u8],
    ) {
        let Some(size) = wgpu::BufferSize::new(bytes.len() as u64) else {
            return;
        };
        let encoder = self.encoder.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("envois de géométrie"),
            })
        });
        self.belt
            .write_buffer(encoder, target, offset, size)
            .copy_from_slice(bytes);
        self.pending += size.get();
    }

    /// Copie `size` octets d'un tampon de l'arène à un autre (la défragmentation), dans le même
    /// ordre que les envois : une copie voit les octets envoyés avant elle.
    pub fn copy(
        &mut self,
        source: &wgpu::Buffer,
        source_offset: u64,
        target: &wgpu::Buffer,
        target_offset: u64,
        size: u64,
    ) {
        if size == 0 {
            return;
        }
        let encoder = self.encoder.get_or_insert_with(|| {
            self.belt_device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("envois de géométrie"),
                })
        });
        encoder.copy_buffer_to_buffer(source, source_offset, target, target_offset, size);
    }

    /// Trop d'octets en attente : à envoyer sans attendre l'image.
    pub fn should_flush(&self) -> bool {
        self.pending >= FLUSH_BYTES
    }

    /// Envoie les copies en attente. À appeler avant de dessiner (les copies passent avant).
    pub fn flush(&mut self, queue: &wgpu::Queue) {
        let Some(encoder) = self.encoder.take() else {
            return;
        };
        self.belt.finish();
        queue.submit([encoder.finish()]);
        // Les tampons reviennent quand le GPU a fini de les lire (au prochain `submit`).
        self.belt.recall();
        self.pending = 0;
    }
}

/// Vrai si `T` peut être écrit tel quel (garde-fou de compilation pour les types envoyés).
const _: fn() = || {
    fn assert_pod<T: Pod>() {}
    assert_pod::<ColorVertex>();
    assert_pod::<OriginEntry>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_then_free_restores_one_range() {
        let mut a = RangeAllocator::new(100);
        let x = a.alloc(30).unwrap();
        let y = a.alloc(20).unwrap();
        let z = a.alloc(10).unwrap();
        assert_eq!(a.used(), 60);
        a.free(y, 20);
        a.free(x, 30);
        a.free(z, 10);
        assert_eq!(a.used(), 0);
        assert_eq!(a.free_ranges(), 1);
        assert_eq!(a.largest_free(), 100);
    }

    #[test]
    fn best_fit_keeps_large_ranges() {
        let mut a = RangeAllocator::new(100);
        let big = a.alloc(50).unwrap();
        let _gap = a.alloc(10).unwrap();
        let tail = a.alloc(40).unwrap();
        a.free(big, 50);
        a.free(tail, 40);
        // Libres : [0, 50) et [60, 100) ; 35 va dans la plus petite qui convient (40).
        assert_eq!(a.alloc(35), Some(60));
        // Il reste 50 d'un seul tenant pour un gros maillage.
        assert_eq!(a.alloc(50), Some(0));
    }

    #[test]
    fn exhausted_allocator_refuses() {
        let mut a = RangeAllocator::new(10);
        assert!(a.alloc(11).is_none());
        assert_eq!(a.alloc(10), Some(0));
        assert!(a.alloc(1).is_none());
        assert!(!a.can_alloc(1));
    }

    #[test]
    fn random_use_never_overlaps_and_ends_clean() {
        // Un petit générateur déterministe : pas besoin de dépendance pour ce test.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |bound: u32| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % u64::from(bound)) as u32
        };
        let mut a = RangeAllocator::new(4096);
        let mut live: Vec<(u32, u32)> = Vec::new();
        for _ in 0..5000 {
            if live.is_empty() || next(100) < 55 {
                let len = 1 + next(120);
                if let Some(start) = a.alloc(len) {
                    for &(s, l) in &live {
                        assert!(start + len <= s || s + l <= start, "chevauchement");
                    }
                    live.push((start, len));
                }
            } else {
                let i = next(live.len() as u32) as usize;
                let (s, l) = live.swap_remove(i);
                a.free(s, l);
            }
            assert_eq!(a.used(), live.iter().map(|&(_, l)| l).sum::<u32>());
        }
        for (s, l) in live {
            a.free(s, l);
        }
        assert_eq!(a.used(), 0);
        assert_eq!(a.free_ranges(), 1, "toutes les plages ont fusionné");
    }

    #[test]
    fn zero_length_ranges_are_free() {
        let mut a = RangeAllocator::new(8);
        assert_eq!(a.alloc(0), Some(0));
        a.free(0, 0);
        assert_eq!(a.used(), 0);
        assert_eq!(a.free_ranges(), 1);
    }

    /// Lit `count` éléments de `T` à partir de `offset` (en octets) dans un tampon de l'arène.
    fn read_back<T: Pod>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        offset: u64,
        count: u32,
    ) -> Vec<T> {
        let size = u64::from(count) * size_of::<T>() as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(buffer, offset, &readback, 0, size);
        queue.submit([encoder.finish()]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        bytemuck::cast_slice(&readback.slice(..).get_mapped_range().unwrap()).to_vec()
    }

    #[test]
    #[ignore = "nécessite un adaptateur GPU ; vérifie les vraies copies de la défragmentation"]
    fn defragmentation_empties_sparse_pages_and_keeps_every_byte() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let mut arena = GeometryArena::new();
        let mut uploads = Uploads::new(&device);

        // 64 maillages de 16 000 sommets : 16 par page, donc 4 pages pleines.
        let mesh = |k: u32| {
            let vertices: Vec<ColorVertex> = (0..16_000u32)
                .map(|j| {
                    let mut v: ColorVertex = bytemuck::Zeroable::zeroed();
                    v.position = [k as u16, (j & 0xFFFF) as u16, 7, 9];
                    v.texture = k * 100_000 + j;
                    v
                })
                .collect();
            let indices: Vec<u32> = (0..24_000u32).map(|j| (j * 7 + k) % 16_000).collect();
            (vertices, indices)
        };
        let mut live: Vec<(u32, Slice)> = (0..64)
            .map(|k| {
                let (v, i) = mesh(k);
                (k, arena.alloc(&device, &mut uploads, &v, &i))
            })
            .collect();
        uploads.flush(&queue);
        assert_eq!(arena.stats().pages, 4);

        // Le joueur s'éloigne : trois maillages sur quatre partent, dans chaque page.
        live.retain(|&(k, slice)| {
            let keep = k % 4 == 0;
            if !keep {
                arena.free(slice);
            }
            keep
        });
        for _ in 0..=REUSE_DELAY {
            arena.begin_frame();
        }
        let before = arena.stats();
        assert_eq!(before.pages, 4, "aucune page n'est vide toute seule");

        // Défragmenter jusqu'à ce qu'il n'y ait plus rien à gagner.
        for _ in 0..50 {
            if let Some(page) = arena.drain_target() {
                for (_, slice) in live.iter_mut().filter(|(_, s)| s.page == page) {
                    *slice = arena
                        .relocate(&mut uploads, *slice)
                        .expect("de la place ailleurs");
                }
            }
            uploads.flush(&queue);
            arena.begin_frame();
        }
        let after = arena.stats();
        assert_eq!(after.used_bytes, before.used_bytes);
        // 16 maillages tiennent dans une page ; on garde une page vide de réserve.
        assert!(
            after.pages <= 2,
            "{} pages après défragmentation",
            after.pages
        );
        assert!(after.defrag_bytes > 0);
        assert!(arena.drain_target().is_none());

        for (k, slice) in &live {
            let (vertices, indices) = mesh(*k);
            let page = arena.page(slice.page).unwrap();
            let got: Vec<ColorVertex> = read_back(
                &device,
                &queue,
                &page.vertex_buffer,
                u64::from(slice.first_vertex) * size_of::<ColorVertex>() as u64,
                slice.vertex_count,
            );
            assert!(
                bytemuck::cast_slice::<_, u8>(&got) == bytemuck::cast_slice::<_, u8>(&vertices),
                "sommets du maillage {k}"
            );
            let got: Vec<u32> = read_back(
                &device,
                &queue,
                &page.index_buffer,
                u64::from(slice.first_index) * 4,
                slice.index_count,
            );
            assert_eq!(got, indices, "indices du maillage {k}");
        }
    }
}
