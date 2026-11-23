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

use std::collections::BTreeMap;

use bytemuck::Pod;

use crate::mesh::ColorVertex;

/// Sommets par page (24 octets chacun : 6 Mio).
pub const PAGE_VERTICES: u32 = 1 << 18;
/// Indices par page (4 octets chacun : 2 Mio) ; un quad coûte 4 sommets et 6 indices.
pub const PAGE_INDICES: u32 = 1 << 19;
/// Images d'attente avant de réutiliser une plage libérée.
pub const REUSE_DELAY: u64 = 3;
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
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("arène : indices"),
            size: u64::from(index_capacity) * 4,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
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
        let found = self
            .pages
            .iter_mut()
            .enumerate()
            .filter_map(|(i, page)| Some((i, page.as_mut()?)))
            .find(|(_, page)| {
                page.vertices.can_alloc(vertex_count) && page.indices.can_alloc(index_count)
            })
            .map(|(i, _)| i);
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
    }

    pub fn stats(&self) -> ArenaStats {
        let vertex = size_of::<ColorVertex>() as u64;
        let mut stats = ArenaStats {
            pending_ranges: self.pending.len(),
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
    encoder: Option<wgpu::CommandEncoder>,
    pending: u64,
}

impl Uploads {
    pub fn new(device: &wgpu::Device) -> Self {
        Self {
            belt: wgpu::util::StagingBelt::new(device.clone(), STAGING_CHUNK),
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
}
