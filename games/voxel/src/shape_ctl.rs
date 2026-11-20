//! Les commandes `ctl` des formes, pour les essais sans souris (voir `shaping_ui.rs`, `shaped_play.rs`).
//!
//! - `shape forme [n|all]` : taille `n` blocs (un par défaut, `all` : toute la pile) du bloc tenu en
//!   pièces. Les formes : `layer`, `slab`, `slope_layer`, `slope`, `micro`, `nano` ;
//! - `rebuild [all]` : refait un bloc plein (ou tous ceux que la pile permet) avec les pièces tenues ;
//! - `shaping` : ouvre l'écran de l'établi de taille, le bloc tenu dans sa case d'entrée ;
//! - `pieces x y z` : le contenu de la case de formes en x y z, en JSON.
//!
//! Pour poser une pièce, on la tient et on se sert de `use` (ou `place`) sur un bloc visé, comme au clic droit.

use anyhow::{Context as _, Result, bail};
use engine_voxel::shaped::{Piece, PieceKind};
use glam::IVec3;

use crate::VoxelGame;
use crate::control::json_str;
use crate::inventory::Slot;
use crate::ui::Screen;

/// Une commande des formes, lue par `control.rs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Shape(PieceKind, bool, u32),
    Rebuild(bool),
    Open,
    Pieces(IVec3),
}

/// Lit `shape`, `rebuild`, `shaping`, `pieces` ; `None` si ce n'est pas une commande des formes.
pub fn parse(name: &str, args: &[&str]) -> Option<Result<Command>> {
    Some(match name {
        "shape" => (|| {
            let kind = match args.first().copied() {
                Some("layer") => PieceKind::Layer,
                Some("slab") => PieceKind::Slab,
                Some("slope_layer") => PieceKind::SlopeLayer,
                Some("slope") => PieceKind::Slope,
                Some("micro") => PieceKind::Micro,
                Some("nano") => PieceKind::Nano,
                _ => bail!("shape : layer|slab|slope_layer|slope|micro|nano [n|all]"),
            };
            Ok(match args.get(1).copied() {
                None => Command::Shape(kind, false, 1),
                Some("all") => Command::Shape(kind, true, 0),
                Some(n) => Command::Shape(
                    kind,
                    false,
                    n.parse()
                        .ok()
                        .filter(|n| *n > 0)
                        .context("shape : le nombre de blocs doit être un entier positif")?,
                ),
            })
        })(),
        "rebuild" => match args {
            [] => Ok(Command::Rebuild(false)),
            ["all"] => Ok(Command::Rebuild(true)),
            _ => Err(anyhow::anyhow!("rebuild : [all]")),
        },
        "shaping" if args.is_empty() => Ok(Command::Open),
        "pieces" => (|| {
            let n = |i: usize| -> Result<i32> {
                args.get(i)
                    .context("pieces : x y z")?
                    .parse()
                    .with_context(|| format!("pieces : « {} » n'est pas un entier", args[i]))
            };
            Ok(Command::Pieces(IVec3::new(n(0)?, n(1)?, n(2)?)))
        })(),
        _ => return None,
    })
}

impl VoxelGame {
    /// Exécute une commande des formes ; renvoie la réponse JSON.
    pub(crate) fn shape_command(&mut self, command: Command) -> Result<String> {
        let slot = Slot::Main(self.inventory.selected);
        match command {
            Command::Shape(kind, all, blocks) => {
                let mut pieces = 0;
                // `n` blocs : on taille un bloc à la fois, comme `n` clics.
                for _ in 0..if all { 1 } else { blocks } {
                    pieces += self.carve(slot, kind, all).map_err(anyhow::Error::msg)?;
                }
                Ok(format!("{{\"ok\":true,\"pieces\":{pieces}}}"))
            }
            Command::Rebuild(all) => {
                let blocks = self.rebuild(slot, all).map_err(anyhow::Error::msg)?;
                Ok(format!("{{\"ok\":true,\"blocks\":{blocks}}}"))
            }
            Command::Open => {
                // Le bloc tenu passe dans la case d'entrée (le reste de la grille revient à la fermeture).
                if let Some(stack) = self.inventory.slots[self.inventory.selected].take() {
                    let before = self.inventory.craft[0].replace(stack);
                    if let Some(old) = before {
                        self.give_or_drop(old);
                    }
                }
                self.ui.screen = Screen::Shaping;
                self.ui.inventory_open = true;
                self.mouse_captured = false;
                Ok("{\"ok\":true}".to_string())
            }
            Command::Pieces(pos) => {
                let shaped = self
                    .shaped_at(pos)
                    .context("pieces : ce n'est pas une case de formes")?;
                let parts: Vec<String> = shaped
                    .parts
                    .iter()
                    .map(|p| {
                        let shape = match p.piece {
                            Piece::Layer {
                                face,
                                offset,
                                thickness,
                            } => format!(
                                "{{\"layer\":\"{face:?}\",\"offset\":{offset},\"thickness\":{thickness}"
                            ),
                            Piece::Slope {
                                facing,
                                ceiling,
                                level,
                            } => format!(
                                "{{\"slope\":{facing},\"ceiling\":{ceiling},\"level\":{level}"
                            ),
                            Piece::Micro { cells } => format!("{{\"micro\":{cells}"),
                        };
                        format!("{shape},\"material\":{}}}", json_str(shaped.material_of(p)))
                    })
                    .collect();
                Ok(format!(
                    "{{\"ok\":true,\"parts\":[{}],\"full\":{}}}",
                    parts.join(","),
                    shaped.is_full()
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_commands_are_read() {
        assert_eq!(
            parse("shape", &["slab"]).unwrap().unwrap(),
            Command::Shape(PieceKind::Slab, false, 1)
        );
        assert_eq!(
            parse("shape", &["micro", "all"]).unwrap().unwrap(),
            Command::Shape(PieceKind::Micro, true, 0)
        );
        assert_eq!(
            parse("shape", &["nano", "3"]).unwrap().unwrap(),
            Command::Shape(PieceKind::Nano, false, 3)
        );
        assert!(parse("shape", &["cube"]).unwrap().is_err());
        assert!(parse("shape", &["slab", "0"]).unwrap().is_err());
        assert_eq!(
            parse("pieces", &["1", "2", "3"]).unwrap().unwrap(),
            Command::Pieces(IVec3::new(1, 2, 3))
        );
        assert!(parse("give", &[]).is_none());
    }
}
