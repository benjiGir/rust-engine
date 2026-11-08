//! Les types de blocs.

/// L'identifiant d'un type de bloc.
///
/// C'est un « newtype » : une struct qui enveloppe un `u16`. En mémoire c'est
/// exactement un `u16` (2 octets), mais le compilateur refuse de le confondre
/// avec un nombre quelconque. `Default` donne `BlockId(0)`, c'est-à-dire de l'air.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockId(pub u16);

impl BlockId {
    pub const AIR: Self = Self(0);
    pub const GRASS: Self = Self(1);
    pub const DIRT: Self = Self(2);
    pub const STONE: Self = Self(3);
    pub const SAND: Self = Self(4);
    pub const SNOW: Self = Self(5);

    /// `const fn` : peut être évaluée à la compilation.
    pub const fn is_air(self) -> bool {
        self.0 == Self::AIR.0
    }

    /// Couleur linéaire (r, g, b) du bloc. L'air n'est jamais dessiné.
    pub fn color(self) -> [f32; 3] {
        match self {
            Self::GRASS => [0.36, 0.62, 0.25],
            Self::DIRT => [0.45, 0.32, 0.2],
            Self::STONE => [0.5, 0.5, 0.52],
            Self::SAND => [0.86, 0.78, 0.52],
            Self::SNOW => [0.95, 0.96, 1.0],
            // Bloc inconnu : un magenta bien visible pour repérer l'erreur.
            _ => [1.0, 0.0, 1.0],
        }
    }
}
