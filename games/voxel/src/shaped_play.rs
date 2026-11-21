//! Les formes en jeu : poser une pièce, l'empiler, la casser pièce par pièce, et la fiche de visée.
//! (Le modèle de données est dans `engine_voxel::shaped` ; la fabrication des pièces, dans
//! `shaping_ui.rs`.)
//!
//! # Poser
//!
//! Un clic droit avec une pièce en main vise un point d'un bloc :
//!
//! - sur une **case de formes**, la pièce s'ajoute **dans cette case** quand la face visée est à l'intérieur
//!   (le dessus d'une dalle : la dalle de chêne se pose sur la dalle de pierre ; une pente qui monte d'un niveau ;
//!   un microbloc à côté d'un autre). Si elle n'y tient pas, elle va dans la case voisine ;
//! - sinon la pièce naît dans la **case voisine** de la face visée (couche collée à cette face, pente
//!   tournée vers le joueur, microbloc sur le point visé). Une case de formes y naît d'un bloc d'air, ou
//!   s'enrichit si elle y est déjà.
//!
//! Le matériau vient de la pile (son `damage`, voir `shape_materials.rs`).
//!
//! Quelques cas particuliers :
//!
//! - une **tranche** (d'arête ou de coin) posée sur une tranche de même sorte et de même matière
//!   l'agrandit d'un cran ; l'objet coûte la matière ajoutée (voir `engine_voxel::shaped::slice_cost`) ;
//! - un **portillon** s'ouvre et se ferme d'un clic droit (sauf si l'on tient une pièce, qui se pose) ;
//! - une **forme raccordée** (muret, barrière, vitre...) va toujours seule dans une case.
//!
//! # Casser
//!
//! Le clic gauche retire **la pièce visée** (le rayon touche une de ses boîtes) et la rend en objets
//! (une couche de 5 seizièmes rend 5 couches). Le temps de minage est celui de son matériau, au prorata
//! de son volume. La case se simplifie : vide, elle redevient de l'air ; pleine d'un seul matériau, le
//! vrai bloc.

use std::borrow::Cow;

use engine_voxel::shaped::{self, Link, Part, Piece, PieceKind, Shaped};
use engine_voxel::{BlockDef, BlockId, RayHit};
use glam::{IVec3, Vec3};

use crate::VoxelGame;
use crate::events::{self, BlockBroken, BlockPlaced};
use crate::inventory::ItemStack;
use crate::shape_materials;
use crate::ui_info::Card;

impl VoxelGame {
    /// La fiche de visée d'une case de formes : ses matériaux, et la pièce visée.
    pub(crate) fn describe_shaped(&self, card: &mut Card, hit: &RayHit) {
        let Some(shaped) = self.server.shaped_at(hit.block_pos) else {
            return;
        };
        let names: Vec<String> = shaped
            .materials
            .iter()
            .map(|m| shape_materials::material_name(&self.client.registry, m).to_lowercase())
            .collect();
        card.name = format!("Case de formes : {}", names.join(", "));
        if let Some(index) = self.server.aimed_part(hit.block_pos, &shaped) {
            let part = shaped.parts[index];
            let material =
                shape_materials::material_name(&self.client.registry, shaped.material_of(&part));
            let (what, size) = describe_piece(&part.piece);
            card.lines.insert(
                0,
                (
                    format!(
                        "Pièce visée : {what} de {} ({size})",
                        material.to_lowercase()
                    ),
                    crate::skin::TEXT,
                ),
            );
        }
    }
}

/// Le nom d'une pièce et sa taille, pour la fiche de visée.
fn describe_piece(piece: &shaped::Piece) -> (String, String) {
    let text = |a: &str, b: String| (a.to_string(), b);
    match *piece {
        Piece::Layer { thickness, .. } if thickness >= 16 => text("bloc", "16/16".to_string()),
        Piece::Layer { thickness: 8, .. } => text("dalle", "8/16".to_string()),
        Piece::Layer { thickness, .. } => text("couche", format!("{thickness}/16")),
        Piece::Slope {
            level,
            corner,
            base,
            ..
        } => {
            let name = match corner {
                shaped::Corner::None => "pente",
                shaped::Corner::Outer => "pente en coin sortant",
                shaped::Corner::Inner => "pente en coin rentrant",
            };
            let wall = if matches!(base, engine_voxel::Face::NegY | engine_voxel::Face::PosY) {
                ""
            } else {
                ", murale"
            };
            text(name, format!("niveau {level}/16{wall}"))
        }
        Piece::Micro { cells } => text(
            "microblocs",
            format!("{} cellule(s) de 1/64", cells.count_ones()),
        ),
        Piece::Cuboid {
            kind,
            count,
            min,
            max,
        } => {
            let size = (0..3)
                .map(|a| (max[a] - min[a]).to_string())
                .collect::<Vec<_>>();
            (
                kind.name().to_lowercase(),
                format!("{} ({count} objet(s))", size.join(" × ")),
            )
        }
        Piece::Stairs { vertical, .. } => text(
            if vertical {
                "escalier vertical"
            } else {
                "escalier"
            },
            "3/4 de bloc".to_string(),
        ),
        Piece::Connected { kind, open } => text(
            match kind {
                Link::Wall => "muret",
                Link::Fence => "barrière",
                Link::Gate => "portillon",
                Link::Pane => "vitre",
                Link::FlatPane => "vitre à plat",
            },
            if open { "ouvert" } else { "se raccorde" }.to_string(),
        ),
    }
}

/// La boîte du joueur (min, max).
fn player_bounds(game: &crate::server_side::VoxelServer) -> (Vec3, Vec3) {
    crate::player::body(&game.ecs, game.players.entity()).bounds()
}

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Un clic droit sur un portillon : il s'ouvre ou se ferme. `false` : ce n'est pas un portillon
    /// (ou l'on tient une pièce, qui se pose).
    pub(crate) fn use_gate(&mut self, pos: IVec3) -> bool {
        let holds_piece = self
            .players
            .me_mut()
            .inventory
            .selected_stack()
            .is_some_and(|stack| {
                PieceKind::from_key(&self.content.items.get(stack.item).key).is_some()
            });
        if holds_piece {
            return false;
        }
        let Some(mut shaped) = self.shaped_at(pos) else {
            return false;
        };
        let Some(Part {
            piece:
                Piece::Connected {
                    kind: Link::Gate,
                    open,
                },
            ..
        }) = shaped.parts.first().copied()
        else {
            return false;
        };
        shaped.parts[0].piece = Piece::Connected {
            kind: Link::Gate,
            open: !open,
        };
        if shaped::store(&mut self.streamer_mut(), pos, &shaped) {
            self.sound("core:place_hard");
            self.punch();
            self.after_edit(pos);
        }
        true
    }

    /// Les objets que rend une pièce cassée, au pied de la case.
    fn drop_pieces(&mut self, pos: IVec3, piece: &shaped::Piece, material: &str) {
        let number = shape_materials::number_of(material);
        for (kind, count) in piece.drops() {
            let Some(item) = self.content.items.id(kind.key()) else {
                continue;
            };
            self.drop_stack(
                pos,
                ItemStack {
                    item,
                    count,
                    damage: number,
                },
            );
        }
    }

    /// Les nouvelles boîtes de la case gênent-elles le joueur ou une créature ?
    fn blocks_the_player(&self, pos: IVec3, before: &Shaped, after: &Shaped) -> bool {
        let (min, max) = player_bounds(self);
        let origin = pos.as_vec3();
        let old = before.boxes();
        let fresh = after
            .boxes()
            .into_iter()
            .filter(|b| !old.contains(b))
            .any(|b| min.cmplt(origin + b.max).all() && max.cmpgt(origin + b.min).all());
        fresh || (before.is_empty() && self.mob_overlaps(pos))
    }

    /// Ajoute `piece` à la case `pos` (qui contient `existing`, ou rien). `Some(pos)` si c'est fait.
    fn try_place(
        &mut self,
        pos: IVec3,
        existing: Option<Shaped>,
        piece: shaped::Piece,
        material: &str,
    ) -> Option<IVec3> {
        let mut shaped = existing.unwrap_or_default();
        // Une pièce taillée dans un matériau retiré ou non taillable ne se pose pas.
        let allowed = self
            .registry
            .id(material)
            .is_some_and(|id| shaped::is_shapeable(self.registry.def(id)));
        if !allowed {
            self.toast("Ce matériau n'existe plus : la pièce ne peut pas être posée");
            return None;
        }
        let before = shaped.clone();
        shaped.try_add(piece, material).ok()?;
        // Le joueur ne doit pas être emmuré par la pièce ajoutée.
        if self.blocks_the_player(pos, &before, &shaped) {
            log::debug!("impossible de poser une pièce dans une entité");
            return None;
        }
        if !shaped::store(&mut self.streamer_mut(), pos, &shaped) {
            return None;
        }
        let block = self.streamer().world().block(pos);
        events::send(
            &self.ecs,
            BlockPlaced {
                pos,
                block,
                player: Some(self.players.current_id()),
            },
        );
        self.after_edit(pos);
        Some(pos)
    }

    /// Agrandit la tranche `index` de la case `pos` d'un cran, avec `available` objets en main.
    fn grow_slice(
        &mut self,
        pos: IVec3,
        existing: Shaped,
        index: usize,
        available: u32,
    ) -> Option<IVec3> {
        let mut shaped = existing.clone();
        let extra = match shaped.try_grow(index, available) {
            Ok(extra) => extra,
            Err(shaped::AddError::NotEnough) => {
                self.toast("Il n'y a pas assez de tranches en main pour l'agrandir");
                return None;
            }
            Err(_) => {
                self.toast("Cette tranche ne peut pas grandir ici");
                return None;
            }
        };
        if self.blocks_the_player(pos, &existing, &shaped)
            || !shaped::store(&mut self.streamer_mut(), pos, &shaped)
        {
            return None;
        }
        for _ in 0..extra {
            self.players.me_mut().inventory.consume_selected();
        }
        self.punch();
        self.after_edit(pos);
        Some(pos)
    }

    /// Le clic gauche sur une case de formes : retire la pièce visée (ou toute la case si la visée
    /// ne la désigne pas). `None` : ce bloc n'est pas une case de formes.
    pub(crate) fn break_shaped_piece(&mut self, pos: IVec3) -> Option<bool> {
        let mut shaped = self.shaped_at(pos)?;
        let aimed = self
            .aimed_part(pos, &shaped)
            .or_else(|| shaped.parts.len().checked_sub(1));
        let tool = self.held_tool();
        let mut removed = Vec::new();
        match aimed {
            // La visée d'une pièce : une seule. Sans visée (une commande, une explosion) : tout.
            Some(index) if self.aim().hit.is_some_and(|t| t.block_pos == pos) => {
                removed.extend(shaped.remove_part(index));
            }
            _ => {
                while let Some(item) = shaped.remove_part(0) {
                    removed.push(item);
                }
            }
        }
        if removed.is_empty() || !shaped::store(&mut self.streamer_mut(), pos, &shaped) {
            return Some(false);
        }
        let mut wear = false;
        for (piece, material) in removed {
            let Some(id) = self.registry.id(&material) else {
                continue;
            };
            let def = self.registry.def(id).clone();
            events::send(
                &self.ecs,
                BlockBroken {
                    pos,
                    block: id,
                    player: Some(self.players.current_id()),
                },
            );
            if engine_voxel::can_harvest(&def, tool.as_ref()) {
                self.drop_pieces(pos, &piece, &material);
            }
            wear |= def.hardness > 0.0;
        }
        if tool.is_some() && wear && self.wear_held() {
            log::info!("l'outil s'est cassé");
        }
        self.exert(self.hunger_rules.mine);
        self.after_edit(pos);
        Some(true)
    }

    /// Le point visé, relatif à la case visée.
    fn aimed_point(&self, hit: &RayHit) -> Vec3 {
        let aim = self.aim();
        aim.eye + aim.forward * hit.distance - hit.block_pos.as_vec3()
    }

    /// Le contenu de la case de formes en `pos` (`None` si ce n'en est pas une).
    pub(crate) fn shaped_at(&self, pos: IVec3) -> Option<Shaped> {
        let streamer = self.streamer();
        let block = streamer.world().block(pos);
        shaped::is_shaped_block(self.registry.def(block))
            .then(|| shaped::read(streamer.block_entity(pos)))
    }

    /// Un clic droit avec une pièce en main. `None` : ce n'est pas une pièce (la pose ordinaire
    /// continue) ; `Some(pos)` : la case modifiée, ou `Some(None)` si la pièce n'a pas pu se poser.
    pub(crate) fn place_shaped_piece(&mut self) -> Option<Option<IVec3>> {
        let stack = self.players.me_mut().inventory.selected_stack()?;
        let kind = PieceKind::from_key(&self.content.items.get(stack.item).key)?;
        let Some(material) = shape_materials::key_of(stack.damage) else {
            self.toast("Cette pièce n'a pas de matière : taille-la à l'établi");
            return Some(None);
        };
        let aim = self.aim();
        let hit = aim.hit?;
        if hit.normal == IVec3::ZERO {
            return Some(None);
        }
        let forward = aim.forward;
        let point = self.aimed_point(&hit);
        // Une tranche sur une tranche : elle grandit.
        if kind.is_resizable()
            && let Some(existing) = self.shaped_at(hit.block_pos)
            && let Some(index) = existing.resizable_hit(point, hit.normal.as_vec3(), kind)
            && existing.material_of(&existing.parts[index]) == material
        {
            return Some(self.grow_slice(hit.block_pos, existing, index, stack.count));
        }
        // D'abord dans la case visée, si c'en est une et que la face visée est à l'intérieur.
        let mut placed = None;
        if let Some(existing) = self.shaped_at(hit.block_pos)
            && let Some(piece) = shaped::plan_inner(kind, &existing, point, hit.normal, forward)
        {
            placed = self.try_place(hit.block_pos, Some(existing), piece, &material);
        }
        // Sinon dans la case voisine.
        if placed.is_none() {
            let target = hit.block_pos + hit.normal;
            let piece = shaped::plan_outer(kind, point, hit.normal, forward);
            let (block, def_liquid) = {
                let streamer = self.streamer();
                let block = streamer.world().block(target);
                (block, self.registry.def(block).liquid)
            };
            let existing = self.shaped_at(target);
            if block.is_air() || def_liquid || existing.is_some() {
                placed = self.try_place(target, existing, piece, &material);
            }
        }
        let pos = placed?;
        self.players.me_mut().inventory.consume_selected();
        self.punch();
        Some(Some(pos))
    }

    /// L'indice de la pièce visée dans la case `pos`.
    fn aimed_part(&self, pos: IVec3, shaped: &Shaped) -> Option<usize> {
        // Une forme raccordée est seule dans sa case : c'est elle, bras compris.
        if shaped.link().is_some() {
            return Some(0);
        }
        let hit = self.aim().hit.filter(|t| t.block_pos == pos)?;
        shaped.part_hit(self.aimed_point(&hit), hit.normal.as_vec3())
    }

    /// Ce que le minage regarde : le bloc lui-même, ou, pour une case de formes, le matériau de la
    /// pièce visée (sa dureté et son outil), au prorata du volume de la pièce.
    pub(crate) fn mining_def(&self, pos: IVec3, block: BlockId) -> Cow<'_, BlockDef> {
        let def = self.registry.def(block);
        if !shaped::is_shaped_block(def) {
            return Cow::Borrowed(def);
        }
        let Some(shaped) = self.shaped_at(pos) else {
            return Cow::Borrowed(def);
        };
        let Some(index) = self
            .aimed_part(pos, &shaped)
            .or_else(|| shaped.parts.len().checked_sub(1))
        else {
            return Cow::Borrowed(def);
        };
        let part = shaped.parts[index];
        let Some(material) = self.registry.id(shaped.material_of(&part)) else {
            return Cow::Borrowed(def);
        };
        let mut out = self.registry.def(material).clone();
        // Le volume de la pièce, en part du bloc plein (au moins 1/16 : un grain ne se mine pas
        // instantanément).
        let fraction = part
            .piece
            .occupancy()
            .iter()
            .map(|r| r.count_ones())
            .sum::<u32>() as f32
            / 4096.0;
        if out.hardness > 0.0 {
            out.hardness *= fraction.clamp(1.0 / 16.0, 1.0);
        }
        Cow::Owned(out)
    }
}
