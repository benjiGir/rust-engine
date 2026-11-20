//! Les commandes `ctl` du stockage, pour les essais sans clavier ni souris.
//!
//! - `backpack` : ouvre le premier sac de l'inventaire (comme la touche B) ;
//! - `deposit` / `topup` / `takeall` / `sort` : le geste du même nom sur le conteneur ouvert ;
//! - `quickmove n` : un Maj + clic sur la case `n` de l'inventaire (1 à 36) ;
//! - `container x y z` : le contenu d'un conteneur (coffre, four, machine) en JSON ;
//! - `bag [numéro]` : le contenu d'un sac (par défaut celui de l'écran ouvert, sinon le premier
//!   sac porté).
//!
//! - `drops` : les objets au sol (objet, nombre, position, secondes), pour voir l'aimant agir ;
//! - `capture on|off` : fait comme si la souris était capturée (sans toucher au vrai curseur), ce
//!   qui affiche la fiche de visée et son aperçu dans une capture d'écran ;
//!
//! Les écrans de conteneur eux-mêmes se pilotent avec `click`/`clicktext`.

use anyhow::{Context as _, Result, bail};
use glam::IVec3;

use crate::VoxelGame;
use crate::storage_ui::Op;
use crate::ui::Screen;

/// Une commande du stockage, lue par `control.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Backpack,
    /// Maj + clic droit avec le sac en main, sur le bloc visé.
    PlaceBag,
    Op(Op),
    Container(IVec3),
    Bag(Option<u32>),
    Capture(bool),
    Drops,
}

/// Lit `backpack`, `deposit`, `quickmove`... ; `None` si ce n'est pas une commande du stockage.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    let no_args = |command: Command| {
        if args.is_empty() {
            Ok(command)
        } else {
            Err(anyhow::anyhow!("{name} : pas d'argument"))
        }
    };
    Some(match name {
        "backpack" => no_args(Command::Backpack),
        "placebag" => no_args(Command::PlaceBag),
        "deposit" => no_args(Command::Op(Op::Deposit)),
        "topup" => no_args(Command::Op(Op::TopUp)),
        "takeall" => no_args(Command::Op(Op::TakeAll)),
        "sort" => no_args(Command::Op(Op::Sort)),
        "quickmove" => args
            .first()
            .and_then(|a| a.parse::<usize>().ok())
            .filter(|n| args.len() == 1 && (1..=36).contains(n))
            .map(|n| Command::Op(Op::QuickIn(n - 1)))
            .context("quickmove : une case de 1 à 36"),
        "container" => (|| {
            if args.len() != 3 {
                bail!("container : x y z");
            }
            let n = |i: usize| -> Result<i32> {
                args[i]
                    .parse()
                    .with_context(|| format!("« {} » n'est pas un entier", args[i]))
            };
            Ok(Command::Container(IVec3::new(n(0)?, n(1)?, n(2)?)))
        })(),
        "drops" => no_args(Command::Drops),
        "capture" => match args {
            ["on"] => Ok(Command::Capture(true)),
            ["off"] => Ok(Command::Capture(false)),
            _ => Err(anyhow::anyhow!("capture : on ou off")),
        },
        "bag" => match args {
            [] => Ok(Command::Bag(None)),
            [n] => n
                .parse()
                .map(|n| Command::Bag(Some(n)))
                .context("bag : un numéro de sac"),
            _ => Err(anyhow::anyhow!("bag : [numéro]")),
        },
        _ => return None,
    })
}

fn slots_json(
    items: &engine_voxel::ItemRegistry,
    slots: &[Option<engine_voxel::ItemStack>],
) -> String {
    let mut out = Vec::new();
    for (i, s) in slots.iter().enumerate() {
        if let Some(s) = s {
            let _ = items;
            out.push(format!(
                "[{i},{},{},{}]",
                crate::control::json_str(&s.item),
                s.count,
                s.damage
            ));
        }
    }
    format!("[{}]", out.join(","))
}

impl VoxelGame {
    /// Exécute une commande du stockage ; renvoie la réponse JSON.
    pub(crate) fn storage_command(&mut self, command: Command) -> Result<String> {
        match command {
            Command::Backpack => {
                if !self.open_first_bag() {
                    bail!("backpack : aucun sac à dos dans l'inventaire");
                }
                let Screen::Bag(number) = self.ui.screen else {
                    bail!("backpack : le sac ne s'est pas ouvert");
                };
                Ok(format!("{{\"ok\":true,\"bag\":{number}}}"))
            }
            Command::PlaceBag => {
                self.aimed()?;
                if !self.place_backpack() {
                    bail!("placebag : il faut un sac à dos en main");
                }
                Ok("{\"ok\":true}".to_string())
            }
            Command::Op(op) => {
                // Le contrôleur n'a pas de cases : seuls Déposer et Maj + clic y ont un sens.
                if let Screen::Controller(pos) = self.ui.screen {
                    match op {
                        Op::Deposit => self.controller_deposit(pos, None),
                        Op::QuickIn(slot) => self.controller_deposit(pos, Some(slot)),
                        _ => bail!("le contrôleur ne connaît que deposit et quickmove"),
                    }
                    return Ok("{\"ok\":true}".to_string());
                }
                let target = self
                    .open_target()
                    .context("aucun conteneur ouvert (coffre, wagon ou sac)")?;
                self.storage_transfer(target, op);
                Ok("{\"ok\":true}".to_string())
            }
            Command::Drops => {
                let mut list = Vec::new();
                self.ecs
                    .query::<(&crate::components::Transform, &crate::drops::DroppedItem)>()
                    .for_each(|_, (t, d)| {
                        list.push(format!(
                            "{{\"item\":{},\"count\":{},\"damage\":{},\"pos\":[{:.2},{:.2},{:.2}],\"age\":{:.1}}}",
                            crate::control::json_str(&self.content.items.get(d.stack.item).key),
                            d.stack.count,
                            d.stack.damage,
                            t.position.x,
                            t.position.y,
                            t.position.z,
                            d.age
                        ));
                    });
                Ok(format!("{{\"ok\":true,\"drops\":[{}]}}", list.join(",")))
            }
            Command::Capture(on) => {
                self.mouse_captured = on;
                Ok("{\"ok\":true}".to_string())
            }
            Command::Container(pos) => {
                let streamer = self.streamer();
                let inventory = streamer
                    .block_entity(pos)
                    .and_then(|e| e.caps.as_ref()?.inventory.as_ref())
                    .with_context(|| format!("container : pas d'inventaire en {pos}"))?;
                let block = streamer.world().block(pos);
                let layout = engine_voxel::layout_of(&self.registry, block);
                Ok(format!(
                    "{{\"ok\":true,\"block\":{},\"slots\":{},\"main\":{},\"upgrades\":{},\"items\":{}}}",
                    crate::control::json_str(&self.registry.def(block).key),
                    inventory.slots.len(),
                    layout.map_or(inventory.slots.len(), |l| l.main),
                    layout.map_or(0, |l| l.upgrades),
                    slots_json(&self.content.items, &inventory.slots),
                ))
            }
            Command::Bag(number) => {
                let number = match (number, self.ui.screen) {
                    (Some(n), _) | (None, Screen::Bag(n)) => n,
                    (None, _) => {
                        let items = self.content.items.clone();
                        crate::backpacks::carried(
                            &self.players.me_mut().inventory,
                            &items,
                            &mut self.backpacks,
                        )
                        .first()
                        .map(|c| c.number)
                        .context("bag : aucun sac numéroté (ouvrir un sac avec `backpack`)")?
                    }
                };
                let bag = self
                    .backpacks
                    .bag(number)
                    .with_context(|| format!("bag : pas de sac numéro {number}"))?;
                Ok(format!(
                    "{{\"ok\":true,\"bag\":{number},\"tier\":{},\"slots\":{},\"main\":{},\"disabled\":{},\"items\":{}}}",
                    bag.tier,
                    bag.slots.len(),
                    bag.layout().main,
                    bag.disabled,
                    slots_json(&self.content.items, &bag.slots),
                ))
            }
        }
    }
}
