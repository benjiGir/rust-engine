//! La physique du joueur : une boîte qui tombe, saute et bute contre les blocs.
//!
//! Le joueur est une AABB de 0,6 × 1,8 × 0,6 blocs. À chaque pas de temps fixe :
//! 1. on calcule la vitesse (entrées du joueur, gravité, saut) ;
//! 2. on déplace la boîte **un axe après l'autre** (Y, puis X, puis Z). Après chaque
//!    axe, si la boîte chevauche un bloc plein, on la ramène contre la face de ce bloc.
//!    Traiter les axes séparément permet de glisser le long d'un mur : seul l'axe
//!    bloqué s'arrête, les autres continuent.
//! 3. un grand déplacement est découpé en petits pas (moins d'un bloc et moins que
//!    la largeur du joueur) : même à grande vitesse, on ne peut pas traverser un mur
//!    en « sautant par-dessus » entre deux images.

use glam::{IVec3, Vec3};

use crate::world::World;

pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_HEIGHT: f32 = 1.8;
/// Hauteur des yeux au-dessus des pieds.
pub const PLAYER_EYE_HEIGHT: f32 = 1.62;
/// En blocs par seconde au carré.
pub const GRAVITY: f32 = 32.0;
/// Vitesse verticale au départ d'un saut : environ 1,25 bloc de hauteur (v² / 2g).
pub const JUMP_SPEED: f32 = 9.0;
/// Vitesse de chute maximale.
const MAX_FALL_SPEED: f32 = 60.0;
/// Dans l'eau : vitesses multipliées par ce facteur.
const SWIM_FACTOR: f32 = 0.4;
/// Vitesse de remontée quand on tient Espace dans l'eau.
const SWIM_UP_SPEED: f32 = 4.0;
/// Vitesse de chute maximale dans l'eau.
const SWIM_SINK_SPEED: f32 = 2.5;
/// En dessous de cette part du corps immergée, on marche (une flaque, un ruisseau).
pub const SWIM_DEPTH: f32 = 0.15;
/// La poussée relative d'un corps dans l'eau : un peu plus que son poids une fois
/// immergé aux 9/10 (il flotte, la tête dehors).
const BODY_FLOAT: f32 = 1.1;
/// La vitesse (blocs par seconde) que donne un courant d'une cellule pleine contre une vide.
const CURRENT_PUSH: f32 = 3.0;
/// Le courant le plus fort qu'un corps subit.
const MAX_CURRENT: f32 = 2.5;
/// La hauteur qu'un corps au sol monte sans sauter en marchant contre une marche (une dalle, un
/// escalier de demi-blocs).
const STEP_HEIGHT: f32 = 0.6;
/// La vitesse verticale d'un élan pour sortir de l'eau : de quoi franchir une berge d'un bloc
/// au-dessus de la surface (un peu plus qu'un saut).
const EXIT_SPEED: f32 = 9.3;
/// Ce qu'il reste de la vitesse de chute quand on entre dans l'eau : le plongeon est freiné.
const ENTRY_DAMPING: f32 = 0.35;
/// Plus grand pas de déplacement testé d'un coup.
const MAX_STEP: f32 = 0.4;
/// Petit écart laissé entre le joueur et un bloc, pour qu'il ne soit pas
/// considéré comme « dedans » à cause des arrondis des flottants.
const SKIN: f32 = 1e-3;

/// Le corps physique du joueur.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerBody {
    /// Le centre de la base de la boîte (les pieds).
    pub position: Vec3,
    pub velocity: Vec3,
    /// Vrai si le joueur repose sur un bloc : lui seul permet de sauter.
    pub on_ground: bool,
    /// Vrai si le corps est assez enfoncé dans un liquide pour nager (au-delà de
    /// `SWIM_DEPTH` : marcher dans une flaque n'est pas nager).
    pub in_liquid: bool,
    /// La part de la hauteur du corps sous la surface, de 0 à 1.
    pub submersion: f32,
    /// La vitesse d'entrée (blocs par seconde) si le corps vient de plonger dans un liquide à
    /// ce pas, sinon 0 : de quoi faire des éclaboussures.
    pub splash: f32,
    /// Il nage contre une berge en tenant Espace : un élan l'aide à en sortir (voir `EXIT_SPEED`).
    pub exit_boost: bool,
    /// Largeur (x et z) et hauteur de la boîte : celles du joueur par défaut. Un cochon
    /// ou un slime utilisent la même physique avec une autre taille.
    pub width: f32,
    pub height: f32,
    /// Une poussée horizontale (un coup qui fait reculer), ajoutée à la marche ; elle
    /// s'amortit vite au sol, plus lentement en l'air.
    pub impulse: Vec3,
    /// La gravité, en blocs par seconde au carré : [`GRAVITY`] par défaut, autre dans une dimension
    /// à la gravité différente.
    pub gravity: f32,
}

/// L'amortissement de la poussée, par seconde : au sol, en l'air.
const IMPULSE_DECAY: (f32, f32) = (8.0, 1.5);

/// Les boîtes qui arrêtent un corps dans la cellule `block` (liste vide : un cube plein) ; `None` si
/// rien n'y gêne. `below` : la cellule est sous celles que touche le corps : seules les cases de
/// formes comptent alors, car leurs boîtes (une barrière : 24 seizièmes) peuvent dépasser de la
/// cellule.
fn blockers(
    world: &World,
    block: IVec3,
    below: bool,
) -> Option<std::borrow::Cow<'_, [crate::BlockBox]>> {
    if !world.is_solid(block) {
        return None;
    }
    if below && !crate::shaped::is_shaped_block(world.registry().def(world.block(block))) {
        return None;
    }
    world.collision_boxes(block)
}

impl PlayerBody {
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            velocity: Vec3::ZERO,
            on_ground: false,
            in_liquid: false,
            submersion: 0.0,
            splash: 0.0,
            exit_boost: false,
            width: PLAYER_WIDTH,
            height: PLAYER_HEIGHT,
            impulse: Vec3::ZERO,
            gravity: GRAVITY,
        }
    }

    /// Une boîte d'une autre taille (une créature).
    pub fn with_size(position: Vec3, width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            ..Self::new(position)
        }
    }

    pub fn eye(&self) -> Vec3 {
        self.position + Vec3::Y * PLAYER_EYE_HEIGHT
    }

    /// Coins minimum et maximum de la boîte.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let half = self.width / 2.0;
        let min = self.position - Vec3::new(half, 0.0, half);
        let max = self.position + Vec3::new(half, self.height, half);
        (min, max)
    }

    /// Vrai si le bloc `block` occupe une partie de la boîte (pour interdire d'y poser un bloc).
    pub fn overlaps_block(&self, block: IVec3) -> bool {
        let (min, max) = self.bounds();
        let block_min = block.as_vec3();
        let block_max = block_min + Vec3::ONE;
        min.cmplt(block_max).all() && max.cmpgt(block_min).all()
    }

    /// Avance la simulation de `dt` secondes.
    ///
    /// `walk` est la vitesse horizontale voulue (blocs par seconde, `y` ignoré) :
    /// pas d'inertie, le joueur s'arrête dès qu'on lâche la touche, comme dans Minecraft.
    ///
    /// Dans l'eau : on avance moins vite, on coule lentement, et `jump` (Espace tenu)
    /// fait remonter : c'est une nage très simple.
    pub fn step(&mut self, world: &World, walk: Vec3, jump: bool, dt: f32) {
        let (submersion, fluid) = self.submersion(world);
        self.submersion = submersion;
        let was_in_liquid = self.in_liquid;
        self.in_liquid = submersion > SWIM_DEPTH;
        self.splash = 0.0;
        if self.in_liquid && !was_in_liquid && self.velocity.y < -1.0 {
            // Le plongeon : l'eau encaisse l'essentiel de la chute.
            self.splash = -self.velocity.y;
            self.velocity.y *= ENTRY_DAMPING;
        }
        if self.in_liquid {
            // La nage dépend du fluide : la viscosité freine (la lave est 6 fois plus
            // visqueuse que l'eau), la densité porte (la poussée d'Archimède).
            let (viscosity, density) = fluid;
            let drag = (5.0 / viscosity.max(1.0)).sqrt().min(1.0);
            // Un courant (l'eau qui s'écoule vers une cellule moins remplie) emporte le corps.
            let current = self.current(world);
            self.velocity.x = walk.x * SWIM_FACTOR * drag + current.x;
            self.velocity.z = walk.z * SWIM_FACTOR * drag + current.z;
            // La poussée : proportionnelle au volume immergé et à la densité du fluide
            // (un corps humain flotte à peine dans l'eau : on s'y enfonce jusqu'aux yeux).
            let buoyancy = self.gravity * BODY_FLOAT * (density / 1000.0) * submersion;
            self.velocity.y += (buoyancy - self.gravity) * SWIM_FACTOR * dt;
            if jump {
                self.velocity.y = self.velocity.y.max(SWIM_UP_SPEED * drag);
            }
            // Le frottement du fluide amortit la vitesse verticale.
            self.velocity.y *= 1.0 - (2.0 * dt / drag).min(0.9);
            self.velocity.y = if self.exit_boost {
                // Contre une berge : on ne se retient pas, on monte d'un élan.
                self.velocity.y.max(EXIT_SPEED)
            } else {
                self.velocity.y.clamp(-SWIM_SINK_SPEED, SWIM_UP_SPEED)
            };
        } else {
            self.velocity.x = walk.x + self.impulse.x;
            self.velocity.z = walk.z + self.impulse.z;
            if jump && self.on_ground {
                self.velocity.y = JUMP_SPEED;
            } else if jump && self.exit_boost {
                // À la surface, le dernier élan pour passer par-dessus la berge.
                self.velocity.y = self.velocity.y.max(EXIT_SPEED);
            }
            self.velocity.y = (self.velocity.y - self.gravity * dt).max(-MAX_FALL_SPEED);
        }

        let decay = if self.on_ground {
            IMPULSE_DECAY.0
        } else {
            IMPULSE_DECAY.1
        };
        self.impulse *= 1.0 - (decay * dt).min(1.0);
        if self.impulse.length_squared() < 1e-4 {
            self.impulse = Vec3::ZERO;
        }
        let delta = self.velocity * dt;
        self.on_ground = false;
        // Y d'abord : on sait tout de suite si l'on est posé avant de glisser en X et Z.
        let mut pushing = false;
        for axis in [1, 0, 2] {
            let before = self.position;
            self.move_axis(world, axis, delta[axis]);
            let moved = |body: &Self| (body.position[axis] - before[axis]).abs();
            let mut blocked =
                axis != 1 && delta[axis] != 0.0 && moved(self) < delta[axis].abs() * 0.9;
            // Arrêté par une marche basse, posé ou dans l'eau : on la monte, sans sauter.
            let touching_water = submersion > 0.0;
            if blocked
                && (self.on_ground && self.velocity.y <= 0.0 || touching_water)
                && self.step_up(world, axis, delta[axis], before)
            {
                blocked = false;
            }
            pushing |= blocked;
        }
        // Dans l'eau, contre une berge trop haute pour une marche, Espace donne un élan.
        self.exit_boost = jump && pushing && submersion > 0.0 && !self.on_ground;
    }

    /// Monte une marche de `STEP_HEIGHT` au plus : la même avancée, un peu plus haut, puis on se
    /// repose sur la marche. Sans effet (le corps reste où il était) si rien ne passe, par exemple
    /// contre un mur ou sous un plafond bas.
    fn step_up(&mut self, world: &World, axis: usize, wanted: f32, before: Vec3) -> bool {
        let stuck = self.position;
        for tenth in 1..=6 {
            let lift = tenth as f32 * 0.1;
            if lift > STEP_HEIGHT + 1e-4 {
                break;
            }
            self.position = before + Vec3::Y * lift;
            if self.overlaps_solid(world) {
                continue;
            }
            self.move_axis(world, axis, wanted);
            if (self.position[axis] - before[axis]).abs() >= wanted.abs() * 0.9 {
                // Posé sur la marche : on redescend du surplus.
                self.try_move(world, 1, -lift);
                self.on_ground = true;
                return true;
            }
        }
        self.position = stuck;
        false
    }

    /// La boîte du corps est-elle dans un bloc plein (en tenant compte des formes) ?
    fn overlaps_solid(&self, world: &World) -> bool {
        let (min, max) = self.bounds();
        let (first, last) = (min.floor().as_ivec3(), max.ceil().as_ivec3() - IVec3::ONE);
        for x in first.x..=last.x {
            // Une rangée de plus, dessous : une barrière monte au-delà de sa case.
            for y in first.y - 1..=last.y {
                for z in first.z..=last.z {
                    let block = IVec3::new(x, y, z);
                    let Some(boxes) = blockers(world, block, y < first.y) else {
                        continue;
                    };
                    if boxes.is_empty() {
                        return true;
                    }
                    let origin = block.as_vec3();
                    if boxes
                        .iter()
                        .any(|b| min.cmplt(origin + b.max).all() && max.cmpgt(origin + b.min).all())
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Le courant qui pousse le corps (blocs par seconde, horizontal) : l'eau coule vers les
    /// cellules voisines moins remplies, et le corps suit. Une eau calme (niveaux égaux) ne
    /// pousse pas ; une cascade ou une vague qui s'étale, oui.
    pub fn current(&self, world: &World) -> Vec3 {
        let registry = world.registry();
        let (x, z) = (
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        );
        // La cellule d'eau la plus proche du centre du corps.
        let middle = (self.position.y + self.height * 0.5).floor() as i32;
        let Some(here) = [middle, self.position.y.floor() as i32]
            .into_iter()
            .map(|y| IVec3::new(x, y, z))
            .find(|&p| registry.def(world.block(p)).liquid)
        else {
            return Vec3::ZERO;
        };
        let amount = |p: IVec3| {
            world
                .fluid_amount(p)
                .map_or(crate::fluid_sim::FULL, i32::from)
        };
        let mut push = Vec3::ZERO;
        for side in [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z] {
            let other = here + side;
            // Seul l'écart avec une autre cellule du même fluide compte : un rivage de roche
            // ou de sable ne pousse pas.
            if registry.def(world.block(other)).liquid && world.block(other) == world.block(here) {
                let diff = amount(here) - amount(other);
                push += side.as_vec3() * (diff as f32 / crate::fluid_sim::FULL as f32);
            }
        }
        (push * CURRENT_PUSH).clamp_length_max(MAX_CURRENT)
    }

    /// L'altitude de la surface du liquide dans la colonne de ce point (à quelques blocs
    /// autour de lui), ou `None` s'il n'y a pas de liquide : pour poser des éclaboussures.
    pub fn surface_at(world: &World, point: Vec3) -> Option<f32> {
        let registry = world.registry();
        let (x, z) = (point.x.floor() as i32, point.z.floor() as i32);
        let y0 = point.y.floor() as i32;
        (y0 - 2..=y0 + 3).rev().find_map(|y| {
            let pos = IVec3::new(x, y, z);
            let def = registry.def(world.block(pos));
            if !def.liquid || registry.def(world.block(pos + IVec3::Y)).liquid {
                return None;
            }
            let level = world.fluid_amount(pos).map_or(1.0, |a| {
                (f32::from(a) / crate::fluid_sim::FULL as f32).min(1.0)
            });
            Some(y as f32 + level * 0.875)
        })
    }

    /// La part du corps sous la surface (de 0 à 1), et la viscosité et la densité du
    /// fluide. La surface d'une cellule dépend de sa quantité de fluide : on peut avoir
    /// de l'eau aux chevilles dans une flaque.
    pub fn submersion(&self, world: &World) -> (f32, (f32, f32)) {
        let registry = world.registry();
        let (x, z) = (
            self.position.x.floor() as i32,
            self.position.z.floor() as i32,
        );
        let (bottom, top) = (self.position.y, self.position.y + self.height);
        let mut under = 0.0;
        let mut fluid = (5.0, 1000.0);
        for y in bottom.floor() as i32..=top.floor() as i32 {
            let pos = IVec3::new(x, y, z);
            let def = registry.def(world.block(pos));
            if !def.liquid {
                continue;
            }
            // La hauteur de la surface dans cette cellule.
            let covered = registry.def(world.block(pos + IVec3::Y)).liquid;
            let level = if covered {
                1.0
            } else {
                world
                    .fluid_amount(pos)
                    .map_or(1.0, |a| (a as f32 / crate::fluid_sim::FULL as f32).min(1.0))
                    * 0.875
            };
            let (low, high) = (y as f32, y as f32 + level);
            under += (high.min(top) - low.max(bottom)).max(0.0);
            if let Some(def) = def
                .fluid
                .as_deref()
                .and_then(|key| world.fluids()?.get(key))
            {
                fluid = (def.viscosity as f32, def.density as f32);
            }
        }
        ((under / self.height).clamp(0.0, 1.0), fluid)
    }

    fn move_axis(&mut self, world: &World, axis: usize, amount: f32) {
        let steps = (amount.abs() / MAX_STEP).ceil().max(1.0);
        let part = amount / steps;
        for _ in 0..steps as u32 {
            if !self.try_move(world, axis, part) {
                break; // Bloqué : inutile de continuer sur cet axe.
            }
        }
    }

    /// Déplace la boîte de `distance` sur `axis`. En cas de collision, la recolle
    /// contre la face du bloc et renvoie `false`.
    fn try_move(&mut self, world: &World, axis: usize, distance: f32) -> bool {
        if distance == 0.0 {
            return true;
        }
        self.position[axis] += distance;
        let (min, max) = self.bounds();

        // Tous les blocs que la boîte touche maintenant.
        let first = min.floor().as_ivec3();
        let last = max.ceil().as_ivec3() - IVec3::ONE;
        // La face de bloc la plus proche dans le sens du mouvement.
        let mut wall: Option<f32> = None;
        for x in first.x..=last.x {
            // Une rangée de plus, dessous : une barrière monte au-delà de sa case.
            for y in first.y - 1..=last.y {
                for z in first.z..=last.z {
                    let block = IVec3::new(x, y, z);
                    // Le registre dit si ce bloc arrête le joueur (un bloc de mod peut ne pas le faire).
                    // Un bloc à boîtes (dalle, escalier) ne bloque que là où il est : on cherche la
                    // face de chaque boîte que la nôtre chevauche.
                    let Some(boxes) = blockers(world, block, y < first.y) else {
                        continue;
                    };
                    let mut faces = [f32::NAN; 1];
                    let mut hit_faces = Vec::new();
                    if boxes.is_empty() {
                        faces[0] = if distance > 0.0 {
                            block[axis] as f32
                        } else {
                            block[axis] as f32 + 1.0
                        };
                    } else {
                        let origin = block.as_vec3();
                        for b in boxes.iter() {
                            let (lo, hi) = (origin + b.min, origin + b.max);
                            if min.cmplt(hi).all() && max.cmpgt(lo).all() {
                                hit_faces.push(if distance > 0.0 { lo[axis] } else { hi[axis] });
                            }
                        }
                    }
                    for face in faces.into_iter().filter(|f| !f.is_nan()).chain(hit_faces) {
                        // `Option::map_or` : la première face trouvée, ou la plus proche.
                        wall = Some(wall.map_or(face, |w| {
                            if distance > 0.0 {
                                w.min(face)
                            } else {
                                w.max(face)
                            }
                        }));
                    }
                }
            }
        }

        let Some(face) = wall else {
            return true;
        };
        if distance > 0.0 {
            self.position[axis] -= max[axis] - face + SKIN;
        } else {
            self.position[axis] += face - min[axis] + SKIN;
            if axis == 1 {
                self.on_ground = true;
            }
        }
        self.velocity[axis] = 0.0;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::test_blocks::stone;

    #[test]
    fn an_impulse_pushes_then_fades() {
        let mut world = World::new();
        for x in -5..30 {
            for z in -5..5 {
                world.set_block(IVec3::new(x, 0, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        body.on_ground = true;
        body.impulse = Vec3::new(5.0, 0.0, 0.0);
        for _ in 0..60 {
            body.step(&world, Vec3::ZERO, false, 1.0 / 60.0);
        }
        // Repoussé, sans marcher, puis arrêté : la poussée s'est éteinte.
        assert!(
            body.position.x > 0.9 && body.position.x < 3.0,
            "{}",
            body.position.x
        );
        assert_eq!(body.impulse, Vec3::ZERO);
    }

    const DT: f32 = 1.0 / 60.0;

    /// Un sol plat en y = 0, de -8 à 8 sur x et z.
    fn floor() -> World {
        let mut world = World::new();
        for x in -8..8 {
            for z in -8..8 {
                world.set_block(IVec3::new(x, 0, z), stone());
            }
        }
        world
    }

    fn run(body: &mut PlayerBody, world: &World, walk: Vec3, jump: bool, ticks: u32) {
        for _ in 0..ticks {
            body.step(world, walk, jump, DT);
        }
    }

    #[test]
    fn falls_and_lands_on_the_ground() {
        let world = floor();
        let mut body = PlayerBody::new(Vec3::new(0.5, 10.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 120);
        assert!(body.on_ground);
        assert!(
            (body.position.y - 1.0).abs() < 0.01,
            "y = {}",
            body.position.y
        );
        assert_eq!(body.velocity.y, 0.0);
    }

    #[test]
    fn fast_movement_does_not_go_through_walls() {
        let mut world = floor();
        // Un mur d'un seul bloc d'épaisseur en x = 3.
        for y in 1..4 {
            for z in -2..3 {
                world.set_block(IVec3::new(3, y, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        // 300 blocs par seconde : 5 blocs par pas de temps.
        run(&mut body, &world, Vec3::new(300.0, 0.0, 0.0), false, 10);
        assert!(
            body.position.x < 3.0 - PLAYER_WIDTH / 2.0,
            "x = {}",
            body.position.x
        );
        assert!(body.position.x > 2.5);
    }

    #[test]
    fn slides_along_a_wall() {
        let mut world = floor();
        for y in 1..4 {
            for z in -8..8 {
                world.set_block(IVec3::new(2, y, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        // En diagonale contre le mur : x bloque, z continue.
        run(&mut body, &world, Vec3::new(4.0, 0.0, 4.0), false, 30);
        assert!(body.position.x < 2.0 - PLAYER_WIDTH / 2.0);
        assert!(body.position.z > 2.0);
    }

    #[test]
    fn jump_climbs_one_block() {
        let world = floor();
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 2);
        let mut highest = body.position.y;
        body.step(&world, Vec3::ZERO, true, DT);
        for _ in 0..60 {
            body.step(&world, Vec3::ZERO, false, DT);
            highest = highest.max(body.position.y);
        }
        assert!(highest > 2.0, "le saut monte à {highest}");
        assert!(body.on_ground, "retombe sur le sol");
    }

    #[test]
    fn ceiling_stops_the_jump() {
        let mut world = floor();
        // Plafond en y = 3 : la tête (à 2,8) n'a que 0,2 bloc de marge.
        for x in -2..3 {
            for z in -2..3 {
                world.set_block(IVec3::new(x, 3, z), stone());
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 2);
        body.step(&world, Vec3::ZERO, true, DT);
        let mut highest_head = 0.0f32;
        for _ in 0..60 {
            body.step(&world, Vec3::ZERO, false, DT);
            highest_head = highest_head.max(body.bounds().1.y);
        }
        assert!(
            highest_head <= 3.0,
            "la tête traverse le plafond : {highest_head}"
        );
        assert!(body.on_ground);
    }

    #[test]
    fn water_slows_the_fall_and_space_swims_up() {
        let water = crate::registry::BlockRegistry::builtin()
            .id("core:water")
            .unwrap();
        let mut world = floor();
        for x in -8..8 {
            for z in -8..8 {
                for y in 1..10 {
                    world.set_block(IVec3::new(x, y, z), water);
                }
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 8.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 30);
        assert!(body.in_liquid);
        assert!(body.velocity.y >= -SWIM_SINK_SPEED, "on coule lentement");
        let low = body.position.y;
        run(&mut body, &world, Vec3::ZERO, true, 30);
        assert!(body.position.y > low + 1.0, "Espace fait remonter");
    }

    #[test]
    fn diving_in_is_slowed_and_reported_as_a_splash() {
        let world = pool("core:water", 6);
        // Tombe de 8 blocs au-dessus de la surface.
        let mut body = PlayerBody::new(Vec3::new(0.5, 14.0, 0.5));
        let mut biggest = 0.0f32;
        let mut entry_speed = 0.0;
        for _ in 0..200 {
            let before = body.velocity.y;
            body.step(&world, Vec3::ZERO, false, DT);
            if body.splash > 0.0 {
                entry_speed = before;
            }
            biggest = biggest.max(body.splash);
        }
        assert!(biggest > 8.0, "un plongeon de 8 blocs : {biggest}");
        assert!(entry_speed < -8.0);
        assert!(body.velocity.y.abs() < 3.0, "freiné : {}", body.velocity.y);
        // Une fois dedans, plus d'éclaboussure.
        body.step(&world, Vec3::ZERO, false, DT);
        assert_eq!(body.splash, 0.0);
    }

    #[test]
    fn water_flowing_to_a_lower_cell_carries_a_swimmer_and_still_water_does_not() {
        let mut world = pool("core:water", 4);
        let body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        assert_eq!(body.current(&world), Vec3::ZERO, "eau calme");
        // La cellule voisine (+X) à demi remplie : l'eau s'y écoule.
        let pos = IVec3::new(1, 1, 0);
        let (chunk, [lx, ly, lz]) = World::split(pos);
        world
            .chunk_mut(chunk)
            .set_fluid_amount(lx, ly, lz, Some(1024));
        let here = IVec3::new(0, 1, 0);
        let (chunk, [lx, ly, lz]) = World::split(here);
        world
            .chunk_mut(chunk)
            .set_fluid_amount(lx, ly, lz, Some(4096));
        let current = body.current(&world);
        assert!(current.x > 1.0 && current.z == 0.0, "{current}");
        // Un corps qui nage sans bouger est emporté de ce côté.
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 10);
        assert!(body.position.x > 0.6, "emporté : {}", body.position.x);
    }

    #[test]
    fn the_surface_is_found_from_above_and_below() {
        let world = pool("core:water", 3);
        let top = PlayerBody::surface_at(&world, Vec3::new(0.5, 5.0, 0.5)).unwrap();
        assert!((top - (3.0 + 0.875)).abs() < 1e-4, "{top}");
        assert_eq!(
            PlayerBody::surface_at(&world, Vec3::new(0.5, 2.0, 0.5)),
            Some(top)
        );
        assert_eq!(
            PlayerBody::surface_at(&world, Vec3::new(0.5, 30.0, 0.5)),
            None
        );
    }

    /// Un monde au sol de pierre (y = 0) avec une dalle (demi-bloc) de test en `x = 2`, y = 1.
    fn world_with_a_slab_at_x2() -> World {
        let mut registry = (*crate::registry::BlockRegistry::builtin()).clone();
        registry
            .add_source(
                "test",
                r#"[(id: "test:slab", name: "D", color: (1,1,1), boxes: [(from: (0,0,0), to: (16,8,16))])]"#,
                "test",
            )
            .unwrap();
        let registry = std::sync::Arc::new(registry);
        let (stone, slab) = (
            registry.id("core:stone").unwrap(),
            registry.id("test:slab").unwrap(),
        );
        let mut world = World::with_registry(registry);
        for x in -8..8 {
            for z in -8..8 {
                world.set_block(IVec3::new(x, 0, z), stone);
            }
        }
        for x in 2..6 {
            for z in -8..8 {
                world.set_block(IVec3::new(x, 1, z), slab);
            }
        }
        world
    }

    #[test]
    fn a_walking_body_climbs_a_half_block_without_jumping_but_not_a_full_one() {
        let world = world_with_a_slab_at_x2();
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::new(4.0, 0.0, 0.0), false, 8);
        run(&mut body, &world, Vec3::new(4.0, 0.0, 0.0), false, 80);
        assert!(body.position.x > 4.0, "il est passé : {}", body.position.x);
        assert!(
            (body.position.y - 1.5).abs() < 0.05 && body.on_ground,
            "posé sur la dalle : {}",
            body.position.y
        );
        // Un bloc plein (1 de haut) l'arrête : il faut sauter.
        let stone = world.registry().id("core:stone").unwrap();
        let mut wall = world_with_a_slab_at_x2();
        for z in -8..8 {
            wall.set_block(IVec3::new(2, 1, z), stone);
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &wall, Vec3::new(4.0, 0.0, 0.0), false, 120);
        assert!(body.position.x < 2.0, "arrêté : {}", body.position.x);
    }

    #[test]
    fn a_low_ceiling_stops_a_step_up() {
        let mut world = world_with_a_slab_at_x2();
        let stone = world.registry().id("core:stone").unwrap();
        // Un plafond juste au-dessus de la dalle : la tête ne passe pas.
        for z in -8..8 {
            world.set_block(IVec3::new(2, 3, z), stone);
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::new(4.0, 0.0, 0.0), false, 120);
        assert!(body.position.x < 2.0, "{}", body.position.x);
    }

    /// Un bassin d'eau de 4 blocs de profondeur (x < 2) bordé d'une berge de pierre (x >= 2) dont le
    /// dessus est à `bank` blocs au-dessus de la surface de l'eau (arrondi : 0 = à fleur).
    fn pool_with_a_bank(bank_top: i32) -> World {
        let mut world = pool("core:water", 4);
        let stone = crate::registry::BlockRegistry::builtin()
            .id("core:stone")
            .unwrap();
        for x in 2..8 {
            for z in -8..8 {
                for y in 1..bank_top {
                    world.set_block(IVec3::new(x, y, z), stone);
                }
            }
        }
        world
    }

    fn swim_to_the_bank(world: &World) -> PlayerBody {
        let mut body = PlayerBody::new(Vec3::new(0.5, 2.0, 0.5));
        run(&mut body, world, Vec3::ZERO, false, 90);
        // Il nage vers la berge (+X) en tenant Espace, comme un joueur qui veut sortir.
        for _ in 0..240 {
            body.step(world, Vec3::X * 4.0, true, DT);
            // Posé sur la berge : on s'arrête là (au-delà, le petit monde de test s'arrête).
            if body.position.x > 2.6 && body.on_ground {
                break;
            }
        }
        body
    }

    #[test]
    fn a_swimmer_climbs_out_onto_a_bank_level_with_the_water_or_one_block_higher() {
        // Berge dont le dessus est à 5 (la surface de l'eau est à 4,875) : à fleur.
        let body = swim_to_the_bank(&pool_with_a_bank(5));
        assert!(
            body.position.x > 2.6 && body.position.y >= 4.99,
            "reste dans l'eau : {:?}",
            body.position
        );
        // Un bloc de plus (dessus à 6) : il faut monter d'un bloc entier.
        let body = swim_to_the_bank(&pool_with_a_bank(6));
        assert!(
            body.position.x > 2.6 && body.position.y >= 5.99,
            "n'arrive pas à sortir : {:?}",
            body.position
        );
    }

    #[test]
    fn a_swimmer_cannot_climb_a_cliff_two_blocks_above_the_water() {
        // Dessus à 7 : 2,1 blocs au-dessus de la surface, hors de portée de l'élan.
        let mut body = PlayerBody::new(Vec3::new(0.5, 2.0, 0.5));
        let world = pool_with_a_bank(7);
        run(&mut body, &world, Vec3::ZERO, false, 90);
        let mut highest = body.position.y;
        for _ in 0..300 {
            body.step(&world, Vec3::X * 4.0, true, DT);
            highest = highest.max(body.position.y);
        }
        assert!(
            body.position.x < 2.0,
            "sur la falaise ? {:?}",
            body.position
        );
        // L'élan ne fait pas voler : au plus un bloc et demi au-dessus de l'eau.
        assert!(highest < 4.875 + 1.6, "monté jusqu'à {highest}");
    }

    #[test]
    fn overlap_test_for_block_placement() {
        let body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        assert!(body.overlaps_block(IVec3::new(0, 1, 0)), "les pieds");
        assert!(body.overlaps_block(IVec3::new(0, 2, 0)), "la tête");
        assert!(
            !body.overlaps_block(IVec3::new(0, 0, 0)),
            "le sol sous les pieds"
        );
        assert!(!body.overlaps_block(IVec3::new(1, 1, 0)), "à côté");
        assert!(
            !body.overlaps_block(IVec3::new(0, 3, 0)),
            "au-dessus de la tête"
        );
    }

    /// Un bassin de `depth` blocs d'un fluide, sur le sol de `floor()`.
    fn pool(key: &str, depth: i32) -> World {
        let block = crate::registry::BlockRegistry::builtin().id(key).unwrap();
        let mut world = floor();
        world.set_fluids(std::sync::Arc::new(crate::fluids::FluidRegistry::builtin()));
        for x in -8..8 {
            for z in -8..8 {
                for y in 1..1 + depth {
                    world.set_block(IVec3::new(x, y, z), block);
                }
            }
        }
        world
    }

    #[test]
    fn a_body_floats_with_its_head_out_of_deep_water() {
        let world = pool("core:water", 8);
        let mut body = PlayerBody::new(Vec3::new(0.5, 2.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 600);
        // La surface est à y = 8 + 0,875 ; les yeux (à 1,62 des pieds) dépassent presque.
        let eyes = body.position.y + PLAYER_EYE_HEIGHT;
        assert!((7.5..10.0).contains(&eyes), "yeux à {eyes}");
        assert!(body.in_liquid);
    }

    #[test]
    fn a_thin_puddle_is_walked_through_and_lava_is_slow() {
        // De l'eau jusqu'aux chevilles : on marche, on ne nage pas.
        let mut world = floor();
        let water = crate::registry::BlockRegistry::builtin()
            .id("core:water")
            .unwrap();
        for x in -8..8 {
            for z in -8..8 {
                let pos = IVec3::new(x, 1, z);
                world.set_block(pos, water);
                let (chunk, [lx, ly, lz]) = World::split(pos);
                world
                    .chunk_mut(chunk)
                    .set_fluid_amount(lx, ly, lz, Some(512));
            }
        }
        let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 10);
        assert!(!body.in_liquid, "immersion {}", body.submersion);
        assert!(body.submersion > 0.0);
        // Dans la lave, on avance bien moins vite que dans l'eau.
        let speed = |key: &str| {
            let world = pool(key, 4);
            let mut body = PlayerBody::new(Vec3::new(0.5, 1.0, 0.5));
            run(&mut body, &world, Vec3::X * 4.0, false, 60);
            body.position.x - 0.5
        };
        assert!(speed("core:lava") < speed("core:water") * 0.6);
    }

    #[test]
    fn a_body_lands_on_a_slab_at_half_height_and_walks_under_a_top_slab() {
        let registry = std::sync::Arc::new(crate::registry::tests::with_shapes());
        let bottom = registry.id("test:slab").unwrap();
        let top = registry.id("test:slab[half=top]").unwrap();
        let mut world = World::with_registry(registry);
        for x in -8..8 {
            for z in -8..8 {
                world.set_block(IVec3::new(x, 0, z), stone());
            }
        }
        world.set_block(IVec3::new(0, 1, 0), bottom);
        let mut body = PlayerBody::new(Vec3::new(0.5, 10.0, 0.5));
        run(&mut body, &world, Vec3::ZERO, false, 120);
        assert!(body.on_ground);
        assert!(
            (body.position.y - 1.5).abs() < 0.01,
            "y = {}",
            body.position.y
        );
        // Une dalle haute à y = 3 : à 1.8 de haut, on passe dessous (sol à 1, plafond à 3.5).
        world.set_block(IVec3::new(4, 3, 0), top);
        let mut walker = PlayerBody::new(Vec3::new(2.5, 1.0, 0.5));
        run(&mut walker, &world, Vec3::new(1.0, 0.0, 0.0), false, 240);
        assert!(walker.position.x > 5.0, "x = {}", walker.position.x);
    }
}
