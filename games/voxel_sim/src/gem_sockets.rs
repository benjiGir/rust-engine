//! Les gestes des gemmes dans l'inventaire : sertir, retirer, ajouter un emplacement. Les règles
//! (emplacements, bonus, noms) sont dans `gems.rs` ; la doc de ce module-là décrit les gestes.

use engine_voxel::ItemRegistry;

use crate::adventure_data::AdventureData;
use crate::affixes;
use crate::enchanting;
use crate::gems::{SEAL_SOCKET, SEAL_UNSOCKET, gem_label, gems_of, socket_count};
use crate::inventory::{Click, ItemStack, Slot};
use crate::item_data;

/// Ce qu'a donné un clic avec un objet au curseur.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Rien à faire : le clic reste un clic ordinaire.
    Ignored,
    /// La gemme est posée.
    Socketed(String),
    /// Les gemmes sorties de l'objet (clés).
    Removed(Vec<String>),
    /// Un emplacement de plus.
    SocketAdded,
    /// Le geste est refusé (le message dit pourquoi) ; rien n'a changé.
    Refused(&'static str),
}

/// Un clic avec `cursor` sur `target` (voir la doc du module). Consomme la gemme ou le sceau.
pub fn use_on(
    data: &AdventureData,
    items: &ItemRegistry,
    cursor: &mut Option<ItemStack>,
    target: &mut Option<ItemStack>,
) -> Outcome {
    let (Some(held), Some(piece)) = (*cursor, target.as_mut()) else {
        return Outcome::Ignored;
    };
    let held_key = items.get(held.item).key.clone();
    let is_gem = data.gem_of(&held_key).is_some();
    let is_seal = held_key == SEAL_UNSOCKET || held_key == SEAL_SOCKET;
    if !is_gem && !is_seal {
        return Outcome::Ignored;
    }
    let def = items.get(piece.item);
    if !affixes::can_carry(def) {
        return Outcome::Ignored;
    }
    let sockets = socket_count(piece.damage);
    let outcome = if held_key == SEAL_SOCKET {
        let max = data.max_sockets as usize;
        if sockets >= max {
            return Outcome::Refused("Cet objet a déjà le maximum d'emplacements");
        }
        if !enchanting::edit_sheet(&mut piece.damage, |s| s.sockets.push(None)) {
            return Outcome::Refused("Plus de place pour les fiches d'objets");
        }
        Outcome::SocketAdded
    } else if sockets == 0 {
        // Pas d'emplacement : un clic ordinaire (on peut poser la gemme dans la case).
        return Outcome::Ignored;
    } else if held_key == SEAL_UNSOCKET {
        let gems = gems_of(piece.damage);
        if gems.is_empty() {
            return Outcome::Refused("Aucune gemme à retirer");
        }
        enchanting::edit_sheet(&mut piece.damage, |s| {
            s.sockets.fill(None);
        });
        Outcome::Removed(gems)
    } else {
        let free =
            item_data::sheet(piece.damage).and_then(|s| s.sockets.iter().position(Option::is_none));
        let Some(free) = free else {
            return Outcome::Refused("Tous les emplacements sont pleins");
        };
        enchanting::edit_sheet(&mut piece.damage, |s| {
            s.sockets[free] = Some(held_key.clone())
        });
        Outcome::Socketed(held_key)
    };
    // Le sceau ou la gemme est consommé.
    if let Some(held) = cursor {
        held.count -= 1;
        if held.count == 0 {
            *cursor = None;
        }
    }
    outcome
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    fn play_ui_sound(&mut self) {
        self.sound("core:enchant");
    }

    /// Un clic gauche sur une case de l'inventaire avec un objet au curseur : pose une gemme, retire
    /// les gemmes ou ajoute un emplacement. `true` si le clic est traité.
    pub fn try_gem_click(&mut self, slot: Slot, click: Click) -> bool {
        if click != Click::Primary {
            return false;
        }
        let data = crate::adventure_data::current();
        let items = self.content.items.clone();
        let outcome = {
            let mut cursor = self.players.me_mut().inventory.cursor;
            let target = self.players.me_mut().inventory.slot_mut(slot);
            let outcome = use_on(&data, &items, &mut cursor, target);
            if outcome != Outcome::Ignored {
                self.players.me_mut().inventory.cursor = cursor;
            }
            outcome
        };
        match outcome {
            Outcome::Ignored => return false,
            Outcome::Refused(why) => self.toast(why),
            Outcome::Socketed(key) => {
                let name = gem_label(&data, &key).map_or(key, |(n, _)| n);
                self.toast(&format!("{name} sertie"));
                self.play_ui_sound();
            }
            Outcome::SocketAdded => {
                self.toast("Un emplacement de plus");
                self.play_ui_sound();
            }
            Outcome::Removed(gems) => {
                self.toast(&format!("{} gemme(s) retirée(s)", gems.len()));
                for key in gems {
                    if let Some(id) = items.id(&key) {
                        let left = self.collect_stack(ItemStack::new(id, 1));
                        if left > 0 {
                            let at = crate::player::body(&self.ecs, self.players.entity())
                                .position
                                .floor()
                                .as_ivec3();
                            self.drop_stack(at, ItemStack::new(id, left));
                        }
                    }
                }
                self.play_ui_sound();
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> ItemRegistry {
        ItemRegistry::builtin(&crate::test_registry()).unwrap()
    }

    fn stack(items: &ItemRegistry, key: &str, count: u32, damage: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: items.id(key).unwrap(),
            count,
            damage,
        })
    }

    /// Une épée à deux emplacements, dont un plein.
    fn socketed_sword(items: &ItemRegistry, sockets: usize) -> Option<ItemStack> {
        let mut sword = stack(items, "core:iron_sword", 1, 0);
        enchanting::edit_sheet(&mut sword.as_mut().unwrap().damage, |s| {
            s.sockets = vec![None; sockets]
        });
        sword
    }

    #[test]
    fn a_gem_goes_into_a_free_socket_and_is_consumed() {
        let data = AdventureData::builtin();
        let items = items();
        let mut target = socketed_sword(&items, 2);
        let mut cursor = stack(&items, "core:gem_ruby_3", 2, 0);
        let out = use_on(&data, &items, &mut cursor, &mut target);
        assert_eq!(out, Outcome::Socketed("core:gem_ruby_3".into()));
        assert_eq!(
            cursor.unwrap().count,
            1,
            "une gemme de la pile est consommée"
        );
        assert_eq!(gems_of(target.unwrap().damage), ["core:gem_ruby_3"]);
        // La deuxième remplit l'autre emplacement ; la dernière du curseur disparaît.
        let mut cursor = stack(&items, "core:gem_opal_1", 1, 0);
        let out = use_on(&data, &items, &mut cursor, &mut target);
        assert!(matches!(out, Outcome::Socketed(_)));
        assert!(cursor.is_none());
        assert_eq!(gems_of(target.unwrap().damage).len(), 2);
        // Plus d'emplacement libre : le geste est refusé, la gemme reste en main.
        let mut cursor = stack(&items, "core:gem_ruby_1", 1, 0);
        let out = use_on(&data, &items, &mut cursor, &mut target);
        assert!(matches!(out, Outcome::Refused(_)));
        assert!(cursor.is_some());
    }

    #[test]
    fn clicks_that_are_not_gem_gestures_stay_ordinary() {
        let data = AdventureData::builtin();
        let items = items();
        let mut cursor = stack(&items, "core:gem_ruby_1", 1, 0);
        // Un objet sans emplacement, une case vide, un objet qui ne porte rien.
        let mut plain = stack(&items, "core:iron_sword", 1, 0);
        assert_eq!(
            use_on(&data, &items, &mut cursor, &mut plain),
            Outcome::Ignored
        );
        let mut empty = None;
        assert_eq!(
            use_on(&data, &items, &mut cursor, &mut empty),
            Outcome::Ignored
        );
        let mut coal = stack(&items, "core:coal", 5, 0);
        assert_eq!(
            use_on(&data, &items, &mut cursor, &mut coal),
            Outcome::Ignored
        );
        // Un objet quelconque au curseur n'est ni gemme ni sceau.
        let mut stick = stack(&items, "core:stick", 1, 0);
        let mut sword = socketed_sword(&items, 1);
        assert_eq!(
            use_on(&data, &items, &mut stick, &mut sword),
            Outcome::Ignored
        );
        assert_eq!(cursor.unwrap().count, 1);
    }

    #[test]
    fn the_unsocket_seal_returns_every_gem() {
        let data = AdventureData::builtin();
        let items = items();
        let mut target = socketed_sword(&items, 3);
        for key in ["core:gem_ruby_2", "core:gem_topaz_4"] {
            let mut cursor = stack(&items, key, 1, 0);
            use_on(&data, &items, &mut cursor, &mut target);
        }
        let mut seal = stack(&items, SEAL_UNSOCKET, 2, 0);
        let out = use_on(&data, &items, &mut seal, &mut target);
        assert_eq!(
            out,
            Outcome::Removed(vec!["core:gem_ruby_2".into(), "core:gem_topaz_4".into()])
        );
        assert_eq!(seal.unwrap().count, 1, "le sceau est consommé");
        assert_eq!(
            socket_count(target.unwrap().damage),
            3,
            "les emplacements restent"
        );
        assert!(gems_of(target.unwrap().damage).is_empty());
        // Plus rien à retirer : refusé, le sceau est gardé.
        let out = use_on(&data, &items, &mut seal, &mut target);
        assert_eq!(out, Outcome::Refused("Aucune gemme à retirer"));
        assert_eq!(seal.unwrap().count, 1);
    }

    #[test]
    fn the_socket_seal_adds_one_up_to_the_maximum() {
        let data = AdventureData::builtin();
        let items = items();
        // Un objet sans fiche : le sceau lui en crée une avec un emplacement.
        let mut target = stack(&items, "core:iron_helmet", 1, 5);
        let mut seal = stack(&items, SEAL_SOCKET, 10, 0);
        assert_eq!(
            use_on(&data, &items, &mut seal, &mut target),
            Outcome::SocketAdded
        );
        assert_eq!(socket_count(target.unwrap().damage), 1);
        assert_eq!(enchanting::wear(target.unwrap().damage), 5, "l'usure suit");
        for _ in 0..3 {
            assert_eq!(
                use_on(&data, &items, &mut seal, &mut target),
                Outcome::SocketAdded
            );
        }
        assert_eq!(
            socket_count(target.unwrap().damage),
            data.max_sockets as usize
        );
        let out = use_on(&data, &items, &mut seal, &mut target);
        assert!(matches!(out, Outcome::Refused(_)));
        assert_eq!(seal.unwrap().count, 6, "quatre sceaux consommés");
        // Une gemme tient maintenant dans l'un d'eux.
        let mut cursor = stack(&items, "core:gem_onyx_5", 1, 0);
        assert!(matches!(
            use_on(&data, &items, &mut cursor, &mut target),
            Outcome::Socketed(_)
        ));
        // Un objet qui ne porte rien ne reçoit pas de sceau.
        let mut coal = stack(&items, "core:coal", 1, 0);
        assert_eq!(
            use_on(&data, &items, &mut seal, &mut coal),
            Outcome::Ignored
        );
    }
}
