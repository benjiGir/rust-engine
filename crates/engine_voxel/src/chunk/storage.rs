//! Le stockage compact des blocs et de la lumière d'un chunk.
//!
//! Un chunk de 32³ blocs en `u16` pèse 64 Kio, sa lumière 32 Kio : avec un rayon de vue de 32,
//! les 25 000 chunks chargés prenaient 2,4 Gio. Or un chunk généré contient presque toujours
//! moins de 16 sortes de blocs (pierre, terre, quelques minerais, de l'air), et un chunk profond
//! est entièrement noir. On range donc :
//!
//! - les blocs dans une **palette** : un indice de 4 bits (16 Kio) ou de 8 bits (32 Kio) par bloc,
//!   ou un seul numéro si le chunk est uniforme. Le tableau plein (64 Kio) ne sert plus qu'à la
//!   génération (écrire vite, bloc par bloc) et aux chunks de plus de 256 sortes de blocs ;
//! - la lumière en un seul octet quand elle est la même partout.
//!
//! Le streamer appelle [`super::Chunk::compact`] une fois le chunk généré, sur le thread de fond.
//! Une modification ensuite (un bloc posé, l'eau qui coule) écrit directement dans la palette, et
//! l'agrandit (4 → 8 → 16 bits) seulement quand une nouvelle sorte de bloc ne tient plus.

use std::borrow::Cow;

use rustc_hash::FxHashMap as HashMap;

use super::VOLUME;
use crate::block::BlockId;

/// Les blocs d'un chunk.
#[derive(Debug, Clone)]
pub(super) enum Blocks {
    /// Le même bloc partout.
    Uniform(BlockId),
    /// Au plus 16 sortes : deux indices par octet (le bloc pair dans les 4 bits du bas).
    Nibbles {
        palette: Vec<BlockId>,
        data: Box<[u8]>,
    },
    /// Au plus 256 sortes : un indice par octet.
    Bytes {
        palette: Vec<BlockId>,
        data: Box<[u8]>,
    },
    /// Le tableau plein : rapide à écrire (la génération), sans limite de sortes.
    Full(Box<[BlockId]>),
}

impl Blocks {
    pub fn full(fill: BlockId) -> Self {
        Self::Full(vec![fill; VOLUME].into_boxed_slice())
    }

    #[inline]
    pub fn get(&self, i: usize) -> BlockId {
        match self {
            Self::Uniform(b) => *b,
            Self::Nibbles { palette, data } => {
                let byte = data[i >> 1];
                let n = if i & 1 == 0 { byte & 0x0F } else { byte >> 4 };
                palette[usize::from(n)]
            }
            Self::Bytes { palette, data } => palette[usize::from(data[i])],
            Self::Full(blocks) => blocks[i],
        }
    }

    pub fn set(&mut self, i: usize, block: BlockId) {
        loop {
            match self {
                Self::Full(blocks) => {
                    blocks[i] = block;
                    return;
                }
                Self::Uniform(fill) => {
                    if *fill == block {
                        return;
                    }
                    *self = Self::Nibbles {
                        palette: vec![*fill],
                        data: vec![0; VOLUME / 2].into_boxed_slice(),
                    };
                }
                Self::Nibbles { palette, data } => {
                    let Some(n) = palette_index(palette, block, 16) else {
                        self.widen();
                        continue;
                    };
                    let byte = &mut data[i >> 1];
                    *byte = if i & 1 == 0 {
                        (*byte & 0xF0) | n
                    } else {
                        (*byte & 0x0F) | (n << 4)
                    };
                    return;
                }
                Self::Bytes { palette, data } => {
                    let Some(n) = palette_index(palette, block, 256) else {
                        self.widen();
                        continue;
                    };
                    data[i] = n;
                    return;
                }
            }
        }
    }

    /// Passe à la représentation plus large suivante (la palette est pleine).
    fn widen(&mut self) {
        let wider = match &*self {
            // Les indices de 4 bits deviennent des octets : la palette ne change pas.
            Self::Nibbles { palette, data } => Self::Bytes {
                palette: palette.clone(),
                data: data
                    .iter()
                    .flat_map(|&byte| [byte & 0x0F, byte >> 4])
                    .collect(),
            },
            other => Self::Full(other.to_vec().into_boxed_slice()),
        };
        *self = wider;
    }

    /// La plus petite représentation de ces blocs.
    pub fn compact(&mut self) {
        let mut palette: Vec<BlockId> = Vec::new();
        let mut lookup: HashMap<BlockId, u8> = HashMap::default();
        let mut indices = vec![0u8; VOLUME];
        // Le dernier bloc vu : les blocs voisins se ressemblent, on évite la table.
        let mut last: Option<(BlockId, u8)> = None;
        for (i, slot) in indices.iter_mut().enumerate() {
            let b = self.get(i);
            let n = match last {
                Some((lb, ln)) if lb == b => ln,
                _ => {
                    let n = match lookup.get(&b) {
                        Some(&n) => n,
                        None => {
                            if palette.len() == 256 {
                                // Trop de sortes de blocs : le tableau plein.
                                if !matches!(self, Self::Full(_)) {
                                    *self = Self::Full(self.to_vec().into_boxed_slice());
                                }
                                return;
                            }
                            let n = palette.len() as u8;
                            palette.push(b);
                            lookup.insert(b, n);
                            n
                        }
                    };
                    last = Some((b, n));
                    n
                }
            };
            *slot = n;
        }
        *self = match palette.len() {
            0 | 1 => Self::Uniform(palette.first().copied().unwrap_or(BlockId::AIR)),
            2..=16 => Self::Nibbles {
                palette,
                data: indices
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|&[even, odd]| even | (odd << 4))
                    .collect(),
            },
            _ => {
                palette.shrink_to_fit();
                Self::Bytes {
                    palette,
                    data: indices.into_boxed_slice(),
                }
            }
        };
    }

    /// Copie les blocs `start..start + out.len()` dans `out`.
    #[inline]
    pub fn copy_to(&self, start: usize, out: &mut [BlockId]) {
        match self {
            Self::Uniform(b) => out.fill(*b),
            Self::Full(blocks) => out.copy_from_slice(&blocks[start..start + out.len()]),
            Self::Bytes { palette, data } => {
                let len = out.len();
                for (o, &n) in out.iter_mut().zip(&data[start..start + len]) {
                    *o = palette[usize::from(n)];
                }
            }
            Self::Nibbles { .. } => {
                for (k, o) in out.iter_mut().enumerate() {
                    *o = self.get(start + k);
                }
            }
        }
    }

    pub fn to_vec(&self) -> Vec<BlockId> {
        let mut out = vec![BlockId::AIR; VOLUME];
        self.copy_to(0, &mut out);
        out
    }

    pub fn as_slice(&self) -> Cow<'_, [BlockId]> {
        match self {
            Self::Full(blocks) => Cow::Borrowed(blocks),
            _ => Cow::Owned(self.to_vec()),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = BlockId> + '_ {
        (0..VOLUME).map(move |i| self.get(i))
    }

    /// Vrai si tous les blocs vérifient `f` (sans décoder un chunk uniforme).
    pub fn all(&self, f: impl Fn(BlockId) -> bool) -> bool {
        match self {
            Self::Uniform(b) => f(*b),
            Self::Full(blocks) => blocks.iter().all(|&b| f(b)),
            Self::Nibbles { palette, .. } | Self::Bytes { palette, .. } => {
                // Une palette peut garder une sorte de bloc qui n'est plus nulle part : on ne
                // conclut vite que si toute la palette vérifie `f`.
                palette.iter().all(|&b| f(b)) || self.iter().all(f)
            }
        }
    }

    /// Octets occupés sur le tas.
    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Nibbles { palette, data } | Self::Bytes { palette, data } => {
                data.len() + palette.capacity() * size_of::<BlockId>()
            }
            Self::Full(blocks) => blocks.len() * size_of::<BlockId>(),
        }
    }
}

/// L'indice de `block` dans la palette, ajouté s'il manque et qu'il reste de la place.
#[inline]
fn palette_index(palette: &mut Vec<BlockId>, block: BlockId, max: usize) -> Option<u8> {
    if let Some(n) = palette.iter().position(|&p| p == block) {
        return Some(n as u8);
    }
    if palette.len() < max {
        palette.push(block);
        return Some((palette.len() - 1) as u8);
    }
    None
}

impl PartialEq for Blocks {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (Self::Full(a), Self::Full(b)) => a == b,
            _ => self.iter().eq(other.iter()),
        }
    }
}

/// La lumière d'un chunk (voir `DEFAULT_LIGHT`).
#[derive(Debug, Clone)]
pub(super) enum Light {
    /// La même partout : plein ciel (un chunk au-dessus du relief), ou le noir (sous terre).
    Uniform(u8),
    Full(Box<[u8]>),
}

impl Light {
    /// `data` gardé tel quel, ou réduit à un octet s'il est uniforme.
    pub fn from_data(data: Box<[u8]>) -> Self {
        match data.first() {
            Some(&first) if data.iter().all(|&v| v == first) => Self::Uniform(first),
            _ => Self::Full(data),
        }
    }

    #[inline]
    pub fn get(&self, i: usize) -> u8 {
        match self {
            Self::Uniform(v) => *v,
            Self::Full(data) => data[i],
        }
    }

    /// La cellule `i`, à modifier (une lumière uniforme devient un tableau).
    #[inline]
    pub fn cell_mut(&mut self, i: usize) -> &mut u8 {
        if let Self::Uniform(v) = *self {
            *self = Self::Full(vec![v; VOLUME].into_boxed_slice());
        }
        match self {
            Self::Full(data) => &mut data[i],
            Self::Uniform(_) => unreachable!("rendue pleine juste au-dessus"),
        }
    }

    #[inline]
    pub fn copy_to(&self, start: usize, out: &mut [u8]) {
        match self {
            Self::Uniform(v) => out.fill(*v),
            Self::Full(data) => out.copy_from_slice(&data[start..start + out.len()]),
        }
    }

    pub fn as_slice(&self) -> Cow<'_, [u8]> {
        match self {
            Self::Uniform(v) => Cow::Owned(vec![*v; VOLUME]),
            Self::Full(data) => Cow::Borrowed(data),
        }
    }

    pub fn all(&self, f: impl Fn(u8) -> bool) -> bool {
        match self {
            Self::Uniform(v) => f(*v),
            Self::Full(data) => data.iter().all(|&v| f(v)),
        }
    }

    pub fn heap_bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Full(data) => data.len(),
        }
    }
}

impl PartialEq for Light {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (Self::Full(a), Self::Full(b)) => a == b,
            _ => (0..VOLUME).all(|i| self.get(i) == other.get(i)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(kinds: u16) -> Vec<BlockId> {
        (0..VOLUME)
            .map(|i| BlockId((i as u16 * 7 + i as u16 / 3) % kinds))
            .collect()
    }

    #[test]
    fn compaction_keeps_every_block_and_picks_the_smallest_form() {
        for (kinds, bytes) in [
            (1, 0),
            (2, VOLUME / 2),
            (16, VOLUME / 2),
            (17, VOLUME),
            (300, VOLUME * 2),
        ] {
            let blocks = pattern(kinds);
            let mut storage = Blocks::Full(blocks.clone().into_boxed_slice());
            storage.compact();
            assert_eq!(storage.to_vec(), blocks, "{kinds} sortes");
            let heap = storage.heap_bytes();
            assert!(
                heap >= bytes && heap <= bytes + 1024,
                "{kinds} sortes : {heap} octets"
            );
        }
    }

    #[test]
    fn writes_widen_the_palette_only_when_needed() {
        let mut storage = Blocks::Uniform(BlockId::AIR);
        let mut expected = vec![BlockId::AIR; VOLUME];
        for k in 0..400u16 {
            let i = (usize::from(k) * 977) % VOLUME;
            storage.set(i, BlockId(k + 1));
            expected[i] = BlockId(k + 1);
            if k == 10 {
                assert!(matches!(storage, Blocks::Nibbles { .. }));
            }
            if k == 100 {
                assert!(matches!(storage, Blocks::Bytes { .. }));
            }
        }
        assert!(matches!(storage, Blocks::Full(_)));
        assert_eq!(storage.to_vec(), expected);
        // Réécrire un bloc déjà dans la palette ne l'agrandit pas.
        let mut small = Blocks::Uniform(BlockId(3));
        small.set(5, BlockId(4));
        small.set(5, BlockId(3));
        small.set(6, BlockId(4));
        assert!(matches!(&small, Blocks::Nibbles { palette, .. } if palette.len() == 2));
        assert_eq!(small.get(6), BlockId(4));
        assert_eq!(small.get(5), BlockId(3));
    }

    #[test]
    fn copies_match_reads_in_every_form() {
        let blocks = pattern(12);
        let mut forms = vec![Blocks::Full(blocks.clone().into_boxed_slice())];
        let mut nibbles = forms[0].clone();
        nibbles.compact();
        forms.push(nibbles);
        let mut bytes = Blocks::Full(pattern(40).into_boxed_slice());
        bytes.compact();
        forms.push(bytes);
        for form in &forms {
            let mut out = vec![BlockId::AIR; 45];
            form.copy_to(1001, &mut out);
            let read: Vec<_> = (1001..1046).map(|i| form.get(i)).collect();
            assert_eq!(out, read);
        }
    }

    #[test]
    fn uniform_light_expands_on_write() {
        let mut light = Light::from_data(vec![0xF0; VOLUME].into_boxed_slice());
        assert!(matches!(light, Light::Uniform(0xF0)));
        *light.cell_mut(9) = 0x3;
        assert_eq!((light.get(9), light.get(10)), (0x3, 0xF0));
        assert_eq!(light.heap_bytes(), VOLUME);
    }
}
