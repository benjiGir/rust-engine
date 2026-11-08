//! L'identifiant numérique d'un bloc.

/// Le numéro d'un type de bloc, tel que stocké dans les chunks.
///
/// C'est un « newtype » : une struct qui enveloppe un `u16`. En mémoire c'est
/// exactement un `u16` (2 octets), mais le compilateur refuse de le confondre
/// avec un nombre quelconque. `Default` donne `BlockId(0)`, c'est-à-dire de l'air.
///
/// Seul l'air a un numéro fixe. Les autres sont attribués au chargement par le
/// [`BlockRegistry`](crate::BlockRegistry), qui connaît aussi leurs propriétés
/// (couleur, solide, transparent...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockId(pub u16);

impl BlockId {
    /// L'air : réservé, toujours 0. Un chunk neuf (rempli de zéros) est donc vide.
    pub const AIR: Self = Self(0);

    /// `const fn` : peut être évaluée à la compilation.
    pub const fn is_air(self) -> bool {
        self.0 == Self::AIR.0
    }
}
