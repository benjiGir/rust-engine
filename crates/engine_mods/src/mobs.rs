//! Les créatures à comportement : le jeu confie au script, par lot, ce que ses créatures
//! perçoivent, et le script répond par des ordres (marcher, sauter, frapper). Ce module fixe
//! le format du lot (le SDK le relit) ; le jeu et le banc d'essai l'écrivent avec [`encode`].
//!
//! Un lot est une suite d'enregistrements de 12 mots de 4 octets (petit-boutiste) :
//! `id, type, x, y, z, vie, vie max, vers_joueur_x, vers_joueur_y, vers_joueur_z, drapeaux, mémoire`.

/// La taille d'un enregistrement, en octets.
pub const RECORD_BYTES: usize = 12 * 4;
/// Au plus ce nombre de créatures par appel (le jeu en envoie davantage en plusieurs fois).
pub const MAX_PER_CALL: usize = 64;

/// Les drapeaux d'un enregistrement.
pub const FLAG_ON_GROUND: u32 = 1;
pub const FLAG_IN_LIQUID: u32 = 1 << 1;
pub const FLAG_HURT: u32 = 1 << 2;

/// Ce que le jeu montre d'une créature au script.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MobSnapshot {
    /// Son numéro, pour les ordres (valable le temps de l'appel).
    pub id: u32,
    /// Son type (voir `mob_kind`).
    pub kind: u32,
    pub position: [f32; 3],
    pub health: f32,
    pub max_health: f32,
    /// Du pied de la créature vers le joueur.
    pub to_player: [f32; 3],
    pub flags: u32,
    /// L'entier que le script a rangé avec `mob_state`.
    pub state: u32,
}

/// Écrit un lot d'enregistrements.
pub fn encode(mobs: &[MobSnapshot]) -> Vec<u8> {
    let mut out = Vec::with_capacity(mobs.len() * RECORD_BYTES);
    for m in mobs {
        out.extend(m.id.to_le_bytes());
        out.extend(m.kind.to_le_bytes());
        for v in m.position {
            out.extend(v.to_le_bytes());
        }
        out.extend(m.health.to_le_bytes());
        out.extend(m.max_health.to_le_bytes());
        for v in m.to_player {
            out.extend(v.to_le_bytes());
        }
        out.extend(m.flags.to_le_bytes());
        out.extend(m.state.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sdk_reads_what_the_game_writes() {
        let snapshot = MobSnapshot {
            id: 7,
            kind: 3,
            position: [1.0, 2.0, 3.0],
            health: 4.0,
            max_health: 10.0,
            to_player: [-1.5, 0.0, 6.0],
            flags: FLAG_ON_GROUND | FLAG_HURT,
            state: 9,
        };
        let mobs = mod_sdk::__private::decode_mobs(&encode(&[snapshot, snapshot]), 2);
        assert_eq!(mobs.len(), 2);
        let m = &mobs[0];
        assert_eq!((m.id.0, m.kind, m.state), (7, 3, 9));
        assert_eq!(m.position, [1.0, 2.0, 3.0]);
        assert_eq!((m.health, m.max_health), (4.0, 10.0));
        assert_eq!(m.to_player, [-1.5, 0.0, 6.0]);
        assert!(m.on_ground && m.hurt && !m.in_liquid);
        assert_eq!(RECORD_BYTES, mod_sdk::mobs::RECORD_BYTES);
        assert_eq!(
            [FLAG_ON_GROUND, FLAG_IN_LIQUID, FLAG_HURT],
            [
                mod_sdk::mobs::FLAG_ON_GROUND,
                mod_sdk::mobs::FLAG_IN_LIQUID,
                mod_sdk::mobs::FLAG_HURT
            ]
        );
    }
}
