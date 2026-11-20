//! Les messages entre ce qui lit les entrées (le client) et ce qui fait vivre le monde (le
//! simulateur).
//!
//! Le client n'agit jamais sur le monde : il fabrique des **intentions** ([`ClientMsg`]) que le
//! simulateur applique dans `VoxelGame::handle` (voir `intents.rs`), joueur par joueur. Une
//! intention ne dit jamais un résultat : « je casse le bloc que je regarde », pas « le bloc en
//! (3, 60, 8) est cassé ». Le simulateur refait lui-même le lancer de rayon à partir de la
//! [`Aim`] du message et ne croit jamais la cible que le client dit viser.
//!
//! Ses réponses sont des [`ServerMsg`]. Tous ces types sont `Serialize`/`Deserialize` : ils
//! sont prêts pour le réseau (postcard), même si en solo les messages restent en mémoire et sont
//! appliqués dans la même image.

use std::sync::Arc;

use engine_ecs::Entity;
use engine_voxel::Chunk;
use glam::{IVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::inventory::{Click, Slot};
use crate::player::MoveMode;

/// Ce que le joueur regarde : l'œil et la direction du regard, au moment de l'action.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aim {
    pub eye: Vec3,
    pub forward: Vec3,
}

/// Une entité désignée dans un message. Un identifiant réseau stable viendra avec l'envoi des
/// entités au client : d'ici là, c'est l'entité de l'ECS (numéro et génération).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRef(pub u64);

impl EntityRef {
    pub fn new(entity: Entity) -> Self {
        Self(u64::from(entity.generation) << 32 | u64::from(entity.index))
    }

    pub fn entity(self) -> Entity {
        Entity {
            index: self.0 as u32,
            generation: (self.0 >> 32) as u32,
        }
    }
}

/// Où en est le minage d'un bloc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigPhase {
    /// Le bouton est pressé sur un bloc : le simulateur commence à compter le temps.
    Start,
    /// Le bouton est lâché, ou la visée a changé de bloc, avant la fin.
    Abort,
    /// La fissure est complète côté client : le simulateur vérifie que le temps a pu s'écouler.
    Finish,
}

/// Une case que `SlotClick` désigne : l'inventaire du joueur, ou une case du conteneur qu'il a
/// ouvert (le simulateur refuse un conteneur que ce joueur n'a pas ouvert).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotTarget {
    Player(Slot),
    /// Une case d'un coffre, d'un four, d'un alambic : par le bloc qui porte l'inventaire.
    Block {
        pos: IVec3,
        index: usize,
    },
    Cart {
        cart: EntityRef,
        index: usize,
    },
    Bag {
        number: u32,
        index: usize,
    },
}

/// Le bouton pressé sur une case (Maj est à part : [`ClientMsg::SlotClick::shift`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClickButton {
    Primary,
    Secondary,
    /// Prendre `n` objets (la fenêtre « Diviser la pile »).
    Take(u32),
}

impl From<Click> for ClickButton {
    fn from(click: Click) -> Self {
        match click {
            Click::Primary => Self::Primary,
            Click::Secondary => Self::Secondary,
            Click::Take(n) => Self::Take(n),
        }
    }
}

impl From<ClickButton> for Click {
    fn from(button: ClickButton) -> Self {
        match button {
            ClickButton::Primary => Self::Primary,
            ClickButton::Secondary => Self::Secondary,
            ClickButton::Take(n) => Self::Take(n),
        }
    }
}

/// Ce que le client demande au simulateur.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMsg {
    /// Où le joueur est allé : le mouvement est prédit par la physique du client, le simulateur
    /// ne fait que le contrôler (voir `movement.rs`). `step` est le numéro du pas fixe.
    Move {
        position: Vec3,
        velocity: Vec3,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
        mode: MoveMode,
        step: u64,
    },
    /// Le minage : `pos` et `face` sont ce que le client croit viser (le simulateur contrôle).
    Dig {
        aim: Aim,
        phase: DigPhase,
        pos: IVec3,
        face: IVec3,
    },
    /// Un coup de poing ou d'arme sur une créature ou un wagon.
    Attack {
        aim: Aim,
        entity: EntityRef,
    },
    /// Le clic droit : le simulateur enchaîne les usages (wagon, porte, lit, conteneur, machine,
    /// habitant, houe, charme...) puis pose le bloc tenu. `sneaking` : Maj est tenu.
    Use {
        aim: Aim,
        sneaking: bool,
    },
    /// Le bouton de repas est tenu ou lâché (envoyé seulement quand il change).
    Eat {
        held: bool,
    },
    /// La case de la barre d'objets en main.
    SelectSlot(u8),
    /// Jeter un objet de la main (`all` : toute la pile).
    Drop {
        aim: Aim,
        all: bool,
    },
    /// Un clic sur une case.
    SlotClick {
        slot: SlotTarget,
        button: ClickButton,
        shift: bool,
    },
    /// L'écran est fermé : la main et la grille d'artisanat retournent dans l'inventaire.
    CloseMenu,
    Chat(String),
    /// Une ligne de la console (commande du jeu ou d'un mod).
    Command(String),
    Respawn,
}

/// Ce que le simulateur répond au client.
///
/// Les messages de chunks portent un `Arc<Chunk>` : en mémoire il passe sans copie, et la réplique du
/// client tient les mêmes blocs que le serveur. Ils ne sont donc pas sérialisables pour l'instant
/// (`#[serde(skip)]`) : le transport réseau les encodera avec le format des sauvegardes (lot 5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMsg {
    /// Un mouvement refusé (ou un déplacement imposé) : le client place le joueur ici.
    Correct { position: Vec3 },
    /// Un chunk hors de portée : le client l'oublie.
    Unload(IVec3),
    // Les variantes `serde(skip)` sont à la fin : elles décalent les numéros de variantes qui les suivent.
    /// Un chunk que le client n'avait pas, avec sa lumière et ses blocs actifs. Le serveur envoie des
    /// colonnes entières, éclairées (voir `engine_voxel::ChunkSender`).
    #[serde(skip)]
    Chunk { pos: IVec3, chunk: Arc<Chunk> },
    /// Un chunk que le client a déjà et qui a changé : le chunk d'aujourd'hui, avec les positions des
    /// blocs posés ou cassés (`edits`, remaillés dans l'image), ou `remesh` pour un changement qui
    /// peut attendre (lumière, machine). Ni l'un ni l'autre : des données seules, sans remaillage.
    #[serde(skip)]
    ChunkChanged {
        pos: IVec3,
        chunk: Arc<Chunk>,
        edits: Vec<IVec3>,
        remesh: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_ref_round_trips() {
        let entity = Entity {
            index: 7,
            generation: 3,
        };
        assert_eq!(EntityRef::new(entity).entity(), entity);
    }

    #[test]
    fn messages_survive_postcard() {
        let aim = Aim {
            eye: Vec3::new(1.0, 2.0, 3.0),
            forward: Vec3::Z,
        };
        let messages = vec![
            ClientMsg::Dig {
                aim,
                phase: DigPhase::Finish,
                pos: IVec3::new(4, 5, 6),
                face: IVec3::Y,
            },
            ClientMsg::SlotClick {
                slot: SlotTarget::Block {
                    pos: IVec3::ONE,
                    index: 4,
                },
                button: ClickButton::Take(5),
                shift: false,
            },
            ClientMsg::Move {
                position: Vec3::ONE,
                velocity: Vec3::ZERO,
                yaw: 0.5,
                pitch: -0.2,
                on_ground: true,
                mode: MoveMode::Walk,
                step: 12,
            },
            ClientMsg::Command("tp 0 70 0".to_string()),
        ];
        for message in messages {
            let bytes = postcard::to_stdvec(&message).unwrap();
            let back: ClientMsg = postcard::from_bytes(&bytes).unwrap();
            assert_eq!(back, message);
        }
        let reply = ServerMsg::Correct {
            position: Vec3::new(0.0, 64.0, 0.0),
        };
        let bytes = postcard::to_stdvec(&reply).unwrap();
        assert_eq!(postcard::from_bytes::<ServerMsg>(&bytes).unwrap(), reply);
        let unload = ServerMsg::Unload(IVec3::new(1, -2, 3));
        let bytes = postcard::to_stdvec(&unload).unwrap();
        assert_eq!(postcard::from_bytes::<ServerMsg>(&bytes).unwrap(), unload);
    }

    #[test]
    fn chunk_messages_share_the_chunk_in_memory() {
        let chunk = Arc::new(Chunk::new());
        let msg = ServerMsg::Chunk {
            pos: IVec3::ZERO,
            chunk: chunk.clone(),
        };
        // Copier le message ne copie pas les 96 Kio du chunk.
        let ServerMsg::Chunk { chunk: copy, .. } = msg.clone() else {
            unreachable!()
        };
        assert!(Arc::ptr_eq(&copy, &chunk));
        // Le transport réseau (lot 5) les encodera à part : pour l'instant, `postcard` les refuse.
        assert!(postcard::to_stdvec(&msg).is_err());
    }
}
