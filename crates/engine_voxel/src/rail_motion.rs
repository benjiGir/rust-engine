//! La physique d'un wagon sur rail : une abscisse le long du chemin du rail courant, et une
//! vitesse signée dans le sens des abscisses (voir [`crate::rails`]).
//!
//! Rien ici ne connaît l'ECS ni le monde : le jeu fournit une fonction `rails` (la position d'un
//! bloc donne son [`Rail`]), et reçoit la nouvelle position. C'est ce qui permet de tester
//! la droite, la pente, le virage, l'accélérateur, le butoir et l'attelage sans charger un chunk.
//!
//! # Un pas
//!
//! [`RailMotion::step`] découpe le temps en **sous-pas** : chacun avance d'au plus
//! [`MAX_STEP`] bloc (0,5), donc moins qu'un chemin n'en mesure (0,79 pour un virage). Le wagon ne
//! peut pas sauter par-dessus un rail, même à 30 blocs par seconde. À chaque sous-pas :
//!
//! 1. la gravité le long de la pente (la composante verticale de la tangente) ;
//! 2. le rail : l'accélérateur alimenté pousse dans le sens de la marche (ou du dernier sens
//!    pris, à l'arrêt) ; non alimenté, il retient ; le butoir arrête au milieu du bloc ;
//! 3. la poussée du wagon (locomotive, passager) et le frottement ;
//! 4. la vitesse maximale ;
//! 5. l'avance sur le chemin, en passant d'un rail au suivant ([`follow`]) : la vitesse change de
//!    signe quand le chemin suivant est parcouru à l'envers.
//!
//! Au bout de la voie (ou si le rail disparaît), le wagon **déraille** : [`Derail`] donne sa
//! position et sa vitesse, et le jeu reprend avec la physique d'une boîte.
//!
//! # L'attelage
//!
//! [`couple`] garde deux wagons à une distance donnée (une barre rigide) : les deux avancent
//! sur leur chemin pour corriger l'écart, et partagent leur vitesse (la locomotive tire donc le
//! train, et chaque wagon en freine la moyenne). Sans attelage, [`bump`] repousse deux wagons qui
//! se touchent et échange leur élan.

use glam::{IVec3, Vec3};

use crate::rails::{Rail, RailKind, RailShape, follow};

/// Un sous-pas avance d'au plus ce nombre de blocs.
pub const MAX_STEP: f32 = 0.5;
/// La vitesse maximale par défaut, en blocs par seconde.
pub const DEFAULT_MAX_SPEED: f32 = 8.0;
/// La vitesse maximale qu'on peut régler.
pub const SPEED_LIMIT: f32 = 30.0;
/// L'accélération de la pesanteur le long d'une pente de 45° (blocs par seconde carrée).
const SLOPE_G: f32 = 5.0;
/// L'accélération d'un rail accélérateur alimenté.
const BOOST: f32 = 12.0;
/// La décélération d'un rail accélérateur qui n'est pas alimenté.
const BRAKE: f32 = 40.0;
/// Le frottement de roulement.
const ROLLING: f32 = 0.4;
/// Sous cette vitesse, un wagon sur un frein est tenu à l'arrêt.
const HOLD: f32 = 0.35;

/// Où est un wagon : un rail, l'abscisse sur son chemin, la vitesse signée (blocs par seconde, dans
/// le sens des abscisses croissantes), et le dernier sens pris (±1), pour repartir à l'arrêt.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailMotion {
    pub pos: IVec3,
    pub shape: RailShape,
    pub s: f32,
    pub speed: f32,
    pub heading: f32,
}

/// Le wagon a quitté la voie : sa position (monde) et sa vitesse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Derail {
    pub position: Vec3,
    pub velocity: Vec3,
}

/// Ce qui pousse le wagon en plus des rails.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drive {
    /// La vitesse maximale (blocs par seconde).
    pub max_speed: f32,
    /// Une accélération dans le sens du dernier mouvement (la locomotive qui brûle).
    pub thrust: f32,
    /// Une accélération signée dans le sens des abscisses (un passager qui avance).
    pub push: f32,
}

impl Drive {
    pub fn coasting(max_speed: f32) -> Self {
        Self {
            max_speed,
            thrust: 0.0,
            push: 0.0,
        }
    }
}

fn sign(x: f32) -> f32 {
    if x < 0.0 { -1.0 } else { 1.0 }
}

impl RailMotion {
    /// Un wagon posé sur ce rail, à l'abscisse `s`, immobile.
    pub fn on(pos: IVec3, shape: RailShape, s: f32) -> Self {
        Self {
            pos,
            shape,
            s: s.clamp(0.0, shape.length()),
            speed: 0.0,
            heading: 1.0,
        }
    }

    /// La position du wagon (ses roues) dans le monde.
    pub fn position(&self) -> Vec3 {
        self.pos.as_vec3() + self.shape.point(self.s)
    }

    /// Le sens des abscisses croissantes, dans le monde.
    pub fn tangent(&self) -> Vec3 {
        self.shape.tangent(self.s)
    }

    /// La vitesse dans le monde.
    pub fn velocity(&self) -> Vec3 {
        self.tangent() * self.speed
    }

    /// Pose un wagon qui tombe ou glisse (`position`, `velocity`) sur le rail dessous, s'il y en a
    /// un tout près : sa vitesse est alors la projection de la sienne sur le chemin.
    pub fn snap(
        position: Vec3,
        velocity: Vec3,
        rails: &dyn Fn(IVec3) -> Option<Rail>,
    ) -> Option<Self> {
        let block = position.floor().as_ivec3();
        [block, block - IVec3::Y].into_iter().find_map(|pos| {
            let rail = rails(pos)?;
            let local = position - pos.as_vec3();
            let s = rail.shape.locate(local);
            let on = rail.shape.point(s);
            let near = Vec3::new(local.x - on.x, 0.0, local.z - on.z).length() < 0.6
                && (local.y - on.y) < 0.45
                && (local.y - on.y) > -0.7;
            near.then(|| {
                let tangent = rail.shape.tangent(s);
                let speed = velocity.dot(tangent);
                Self {
                    pos,
                    shape: rail.shape,
                    s,
                    speed,
                    heading: sign(speed),
                }
            })
        })
    }

    fn derail(&self) -> Derail {
        Derail {
            position: self.position(),
            velocity: self.velocity(),
        }
    }

    /// Avance de `ds` le long du chemin (sans force), de rail en rail. `Err` : bout de voie, le
    /// wagon est arrêté à l'extrémité.
    fn advance(
        &mut self,
        mut ds: f32,
        rails: &dyn Fn(IVec3) -> Option<Rail>,
    ) -> Result<(), Derail> {
        for _ in 0..64 {
            let Some(rail) = rails(self.pos) else {
                return Err(self.derail());
            };
            self.shape = rail.shape;
            let len = rail.shape.length();
            let target = self.s + ds;
            if (0.0..=len).contains(&target) {
                self.s = target;
                return Ok(());
            }
            let end = usize::from(target > len);
            let overshoot = if end == 1 { target - len } else { -target };
            let moving = sign(ds);
            let Some((next, rail, entry)) = follow(self.pos, rail.shape, end, rails) else {
                self.s = if end == 1 { len } else { 0.0 };
                return Err(self.derail());
            };
            // Le chemin suivant est parcouru dans son sens, ou à l'envers.
            let forward = if entry == 0 { 1.0 } else { -1.0 };
            if forward != moving {
                self.speed = -self.speed;
                self.heading = -self.heading;
            }
            self.pos = next;
            self.shape = rail.shape;
            self.s = if entry == 0 { 0.0 } else { rail.shape.length() };
            ds = overshoot * forward;
        }
        Ok(())
    }

    /// Un pas de `dt` secondes. `Some` : le wagon a déraillé.
    pub fn step(
        &mut self,
        dt: f32,
        drive: &Drive,
        rails: &dyn Fn(IVec3) -> Option<Rail>,
    ) -> Option<Derail> {
        let mut remaining = dt;
        for _ in 0..256 {
            if remaining <= 1e-6 {
                break;
            }
            let Some(rail) = rails(self.pos) else {
                return Some(self.derail());
            };
            self.shape = rail.shape;
            let sub = if self.speed.abs() > 1e-3 {
                remaining.min(MAX_STEP / self.speed.abs())
            } else {
                remaining
            };
            remaining -= sub;
            let mut v = self.speed;
            let tangent = rail.shape.tangent(self.s);
            v -= SLOPE_G * tangent.y * sub;
            let held = rail.kind == RailKind::Booster && !rail.powered;
            let buffer = rail.kind == RailKind::Stop && !rail.powered;
            match rail.kind {
                RailKind::Booster if rail.powered => {
                    let dir = if v.abs() > 0.05 {
                        sign(v)
                    } else {
                        self.heading
                    };
                    v += dir * BOOST * sub;
                }
                RailKind::Booster => {
                    let brake = BRAKE * sub;
                    v = if v.abs() <= brake {
                        0.0
                    } else {
                        v - sign(v) * brake
                    };
                    if v.abs() < HOLD {
                        v = 0.0;
                    }
                }
                _ => {}
            }
            // La locomotive et le passager poussent, sauf contre un frein ou un butoir.
            if !held && !buffer {
                if drive.thrust > 0.0 && (v.abs() < drive.max_speed || v * self.heading < 0.0) {
                    v += self.heading * drive.thrust * sub;
                }
                v += drive.push * sub;
            }
            let rolling = ROLLING * sub;
            v = if v.abs() <= rolling {
                0.0
            } else {
                v - sign(v) * rolling
            };
            v = v.clamp(-drive.max_speed, drive.max_speed);
            self.speed = v;
            let ds = (v * sub).clamp(-MAX_STEP, MAX_STEP);
            // Le butoir : on s'arrête en passant le milieu du bloc, mais on peut en repartir.
            if buffer {
                let mid = rail.shape.length() * 0.5;
                let (before, after) = (self.s - mid, self.s + ds - mid);
                if before * after < 0.0 || (before.abs() > 1e-3 && after.abs() <= 1e-3) {
                    self.s = mid;
                    self.speed = 0.0;
                    return None;
                }
            }
            if let Err(derail) = self.advance(ds, rails) {
                return Some(derail);
            }
            if self.speed.abs() > 0.2 {
                self.heading = sign(self.speed);
            }
        }
        None
    }

    /// Déplace le wagon de `distance` (signée, dans le sens des abscisses) sans force : une
    /// correction d'attelage.
    pub fn shift(&mut self, distance: f32, rails: &dyn Fn(IVec3) -> Option<Rail>) {
        let _ = self.advance(distance, rails);
    }

    /// La composante de la vitesse du wagon le long de `u` (une direction du monde).
    pub fn along(&self, u: Vec3) -> f32 {
        self.tangent().dot(u) * self.speed
    }

    /// Ajoute `delta` de vitesse le long de `u`, si le rail est assez aligné avec `u`.
    pub fn nudge(&mut self, u: Vec3, delta: f32) {
        let alignment = self.tangent().dot(u);
        if alignment.abs() > 0.2 {
            self.speed += delta * sign(alignment);
        }
    }

    /// Avance de `amount` vers `u` (négatif : s'en éloigne), le long du rail, s'il est assez
    /// aligné avec `u` : une correction d'attelage ou de choc.
    pub fn pull(&mut self, u: Vec3, amount: f32, rails: &dyn Fn(IVec3) -> Option<Rail>) {
        let alignment = self.tangent().dot(u);
        if alignment.abs() > 0.2 {
            self.shift(amount * sign(alignment), rails);
        }
    }
}

/// Attelage : garde `a` et `b` à `length` l'un de l'autre. Les deux avancent sur leur chemin
/// pour corriger l'écart, et partagent la vitesse qu'ils ont l'un vers l'autre.
pub fn couple(
    a: &mut RailMotion,
    b: &mut RailMotion,
    length: f32,
    rails: &dyn Fn(IVec3) -> Option<Rail>,
) {
    let d = b.position() - a.position();
    let dist = d.length();
    if dist < 1e-4 {
        return;
    }
    let u = d / dist;
    let err = dist - length;
    if err.abs() > 1e-4 {
        a.pull(u, err * 0.5, rails);
        b.pull(-u, err * 0.5, rails);
    }
    let va = a.along(u);
    let vb = b.along(u);
    let common = (va + vb) * 0.5;
    a.nudge(u, common - va);
    b.nudge(u, common - vb);
}

/// Deux wagons qui se touchent (à moins de `gap` l'un de l'autre) se repoussent : l'écart est
/// refermé de moitié chacun, et leur élan est partagé avec un rebond amorti.
pub fn bump(
    a: &mut RailMotion,
    b: &mut RailMotion,
    gap: f32,
    rails: &dyn Fn(IVec3) -> Option<Rail>,
) {
    let d = b.position() - a.position();
    let dist = d.length();
    if dist >= gap || dist < 1e-4 {
        return;
    }
    let u = d / dist;
    a.pull(u, (dist - gap) * 0.5, rails);
    b.pull(-u, (dist - gap) * 0.5, rails);
    let va = a.along(u);
    let vb = b.along(u);
    // Ils se rapprochent : on échange l'élan, avec un rebond de 30 %.
    if va > vb {
        let common = (va + vb) * 0.5;
        let spread = (va - vb) * 0.5 * 0.3;
        a.nudge(u, common - spread - va);
        b.nudge(u, common + spread - vb);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    type Track = HashMap<IVec3, Rail>;

    fn rail(kind: RailKind, shape: RailShape, powered: bool) -> Rail {
        Rail {
            kind,
            shape,
            powered,
        }
    }

    /// Une voie est-ouest de `n` rails à partir de l'origine.
    fn straight(n: i32) -> Track {
        (0..n)
            .map(|x| {
                (
                    IVec3::new(x, 0, 0),
                    rail(RailKind::Normal, RailShape::EastWest, false),
                )
            })
            .collect()
    }

    fn run(m: &mut RailMotion, track: &Track, drive: Drive, seconds: f32) -> Option<Derail> {
        let lookup = |p: IVec3| track.get(&p).copied();
        let dt = 1.0 / 60.0;
        for _ in 0..(seconds / dt) as usize {
            if let Some(derail) = m.step(dt, &drive, &lookup) {
                return Some(derail);
            }
        }
        None
    }

    fn cart_at(x: f32, shape: RailShape) -> RailMotion {
        RailMotion::on(IVec3::new(x.floor() as i32, 0, 0), shape, x.fract())
    }

    #[test]
    fn a_cart_rolls_straight_and_friction_slows_it() {
        let track = straight(60);
        let mut m = cart_at(0.5, RailShape::EastWest);
        m.speed = 8.0;
        run(&mut m, &track, Drive::coasting(8.0), 1.0);
        let travelled = m.position().x - 0.5;
        assert!((7.4..8.0).contains(&travelled), "{travelled}");
        assert!(m.speed < 8.0 && m.speed > 7.0);
        assert!((m.position().y - crate::rails::RAIL_HEIGHT).abs() < 1e-4);
        // À l'arrêt, il reste à l'arrêt.
        let mut still = cart_at(3.5, RailShape::EastWest);
        run(&mut still, &track, Drive::coasting(8.0), 1.0);
        assert_eq!(still.speed, 0.0);
        assert!((still.position().x - 3.5).abs() < 1e-4);
    }

    #[test]
    fn a_cart_going_west_comes_out_the_other_way_round() {
        let track = straight(20);
        let mut m = cart_at(10.5, RailShape::EastWest);
        m.speed = -6.0;
        run(&mut m, &track, Drive::coasting(8.0), 1.0);
        assert!(m.position().x < 5.5, "{:?}", m.position());
        assert!(m.speed < 0.0 && m.heading < 0.0);
    }

    #[test]
    fn gravity_pulls_a_cart_down_a_slope_and_slows_it_going_up() {
        // Une montée de quatre rails vers +X : (0,0,0) .. (3,3,0) (chaque rail un cran plus haut).
        let mut track = Track::new();
        for x in 0..4 {
            track.insert(
                IVec3::new(x, x, 0),
                rail(RailKind::Normal, RailShape::AscendEast, false),
            );
        }
        track.insert(
            IVec3::new(4, 4, 0),
            rail(RailKind::Normal, RailShape::EastWest, false),
        );
        // Lâché en haut de la pente, il descend de plus en plus vite.
        let mut m = RailMotion::on(IVec3::new(3, 3, 0), RailShape::AscendEast, 1.0);
        run(&mut m, &track, Drive::coasting(30.0), 1.0);
        assert!(m.speed < -2.0, "il descend : {}", m.speed);
        // Lancé vers le haut, il ralentit puis redescend.
        let mut up = RailMotion::on(IVec3::ZERO, RailShape::AscendEast, 0.2);
        up.speed = 6.0;
        run(&mut up, &track, Drive::coasting(30.0), 0.3);
        assert!(up.speed < 6.0 - 1.0, "freiné par la pente : {}", up.speed);
        assert!(up.position().y > 1.0, "il monte : {}", up.position().y);
        run(&mut up, &track, Drive::coasting(30.0), 3.0);
        assert!(up.speed < 0.0, "puis il redescend : {}", up.speed);
    }

    #[test]
    fn a_cart_follows_a_curve_without_jumping() {
        // Est-ouest, puis un virage vers le sud, puis nord-sud.
        let mut track = Track::new();
        for x in 0..3 {
            track.insert(
                IVec3::new(x, 0, 0),
                rail(RailKind::Normal, RailShape::EastWest, false),
            );
        }
        track.insert(
            IVec3::new(3, 0, 0),
            rail(RailKind::Normal, RailShape::SouthWest, false),
        );
        for z in 1..20 {
            track.insert(
                IVec3::new(3, 0, z),
                rail(RailKind::Normal, RailShape::NorthSouth, false),
            );
        }
        let lookup = |p: IVec3| track.get(&p).copied();
        let mut m = cart_at(0.5, RailShape::EastWest);
        m.speed = 8.0;
        let drive = Drive::coasting(8.0);
        let mut last = m.position();
        for _ in 0..120 {
            m.step(1.0 / 60.0, &drive, &lookup);
            let now = m.position();
            assert!(
                (now - last).length() < 8.0 / 60.0 + 1e-3,
                "saut de {last:?} à {now:?}"
            );
            last = now;
        }
        assert!(m.pos.z >= 1, "il est passé au sud : {:?}", m.pos);
        assert!(m.tangent().z.abs() > 0.99);
        assert!((m.position().x - 3.5).abs() < 1e-3, "{:?}", m.position());
    }

    #[test]
    fn a_powered_booster_pushes_and_an_unpowered_one_holds() {
        let mut track = straight(40);
        for x in 10..20 {
            track.insert(
                IVec3::new(x, 0, 0),
                rail(RailKind::Booster, RailShape::EastWest, true),
            );
        }
        let mut m = cart_at(9.5, RailShape::EastWest);
        m.speed = 3.0;
        run(&mut m, &track, Drive::coasting(8.0), 0.8);
        assert!(m.speed > 5.0, "poussé : {}", m.speed);
        // Jusqu'à la vitesse maximale, pas au-delà.
        run(&mut m, &track, Drive::coasting(8.0), 0.4);
        assert!(m.speed <= 8.0 + 1e-4);
        // Un accélérateur à l'arrêt, alimenté, fait repartir le wagon dans son dernier sens.
        let mut stopped = cart_at(12.5, RailShape::EastWest);
        run(&mut stopped, &track, Drive::coasting(8.0), 0.5);
        assert!(stopped.speed > 1.0, "repart : {}", stopped.speed);

        // Non alimentés : ils arrêtent le wagon et le retiennent.
        for x in 10..20 {
            track.insert(
                IVec3::new(x, 0, 0),
                rail(RailKind::Booster, RailShape::EastWest, false),
            );
        }
        let mut m = cart_at(9.5, RailShape::EastWest);
        m.speed = 8.0;
        run(&mut m, &track, Drive::coasting(8.0), 2.0);
        assert_eq!(m.speed, 0.0);
        assert!(
            m.position().x < 20.0 && m.position().x > 10.0,
            "{:?}",
            m.position()
        );
    }

    #[test]
    fn a_stop_halts_a_cart_in_the_middle_of_its_block_and_lets_it_leave() {
        let mut track = straight(20);
        track.insert(
            IVec3::new(10, 0, 0),
            rail(RailKind::Stop, RailShape::EastWest, false),
        );
        let mut m = cart_at(2.5, RailShape::EastWest);
        m.speed = 8.0;
        let derail = run(&mut m, &track, Drive::coasting(8.0), 3.0);
        assert!(derail.is_none());
        assert_eq!(m.speed, 0.0);
        assert!((m.position().x - 10.5).abs() < 1e-3, "{:?}", m.position());
        // Une poussée le fait repartir (il part du milieu, il n'est pas retenu).
        let drive = Drive {
            push: 3.0,
            ..Drive::coasting(8.0)
        };
        // Retenu tant que le butoir n'est pas alimenté : la poussée ne passe pas.
        run(&mut m, &track, drive, 0.5);
        assert_eq!(m.speed, 0.0);
        track.insert(
            IVec3::new(10, 0, 0),
            rail(RailKind::Stop, RailShape::EastWest, true),
        );
        run(&mut m, &track, drive, 1.0);
        assert!(m.position().x > 11.0, "{:?}", m.position());
    }

    #[test]
    fn the_speed_limit_holds_and_no_rail_is_skipped_at_30_blocks_per_second() {
        // Un circuit : une boucle de rails (virages et droites) sur 12 × 12 blocs.
        let mut track = Track::new();
        for i in 1..11 {
            track.insert(
                IVec3::new(i, 0, 0),
                rail(RailKind::Normal, RailShape::EastWest, false),
            );
            track.insert(
                IVec3::new(i, 0, 11),
                rail(RailKind::Normal, RailShape::EastWest, false),
            );
            track.insert(
                IVec3::new(0, 0, i),
                rail(RailKind::Normal, RailShape::NorthSouth, false),
            );
            track.insert(
                IVec3::new(11, 0, i),
                rail(RailKind::Normal, RailShape::NorthSouth, false),
            );
        }
        track.insert(
            IVec3::ZERO,
            rail(RailKind::Normal, RailShape::SouthEast, false),
        );
        track.insert(
            IVec3::new(11, 0, 0),
            rail(RailKind::Normal, RailShape::SouthWest, false),
        );
        track.insert(
            IVec3::new(0, 0, 11),
            rail(RailKind::Normal, RailShape::NorthEast, false),
        );
        track.insert(
            IVec3::new(11, 0, 11),
            rail(RailKind::Normal, RailShape::NorthWest, false),
        );
        let lookup = |p: IVec3| track.get(&p).copied();
        let mut m = RailMotion::on(IVec3::new(5, 0, 0), RailShape::EastWest, 0.5);
        m.speed = 30.0;
        let drive = Drive {
            thrust: 20.0,
            ..Drive::coasting(30.0)
        };
        let mut visited = vec![m.pos];
        let mut last = m.position();
        for _ in 0..1200 {
            assert!(m.step(1.0 / 60.0, &drive, &lookup).is_none());
            assert!(m.speed.abs() <= 30.0 + 1e-4, "{}", m.speed);
            let now = m.position();
            assert!(
                (now - last).length() <= 0.5 + 1e-3,
                "saut de {last:?} à {now:?}"
            );
            last = now;
            if visited.last() != Some(&m.pos) {
                let step = m.pos - *visited.last().unwrap();
                assert_eq!(
                    step.abs().element_sum(),
                    1,
                    "rail sauté : {visited:?} puis {:?}",
                    m.pos
                );
                visited.push(m.pos);
            }
        }
        // 20 secondes à 30 blocs par seconde : plusieurs tours, tous les rails visités.
        assert!(visited.len() > 100, "{}", visited.len());
        assert!(track.keys().all(|p| visited.contains(p)));
    }

    #[test]
    fn a_cart_derails_at_the_end_of_the_track_and_keeps_its_velocity() {
        let track = straight(6);
        let mut m = cart_at(2.5, RailShape::EastWest);
        m.speed = 8.0;
        let derail = run(&mut m, &track, Drive::coasting(8.0), 2.0).expect("déraillé");
        assert!(
            (derail.position.x - 6.0).abs() < 0.1,
            "{:?}",
            derail.position
        );
        assert!(derail.velocity.x > 5.0 && derail.velocity.y.abs() < 1e-3);
        // Posé de nouveau sur la voie, il reprend le rail.
        let lookup = |p: IVec3| track.get(&p).copied();
        let back =
            RailMotion::snap(Vec3::new(3.2, 0.2, 0.5), Vec3::new(4.0, 0.0, 0.0), &lookup).unwrap();
        assert_eq!(back.pos, IVec3::new(3, 0, 0));
        assert!((back.speed - 4.0).abs() < 1e-4);
        assert!(RailMotion::snap(Vec3::new(3.2, 3.0, 0.5), Vec3::ZERO, &lookup).is_none());
    }

    #[test]
    fn coupled_carts_keep_their_gap_and_the_front_one_pulls_the_other() {
        let track = straight(80);
        let lookup = |p: IVec3| track.get(&p).copied();
        let mut front = cart_at(10.5, RailShape::EastWest);
        let mut back = cart_at(9.3, RailShape::EastWest);
        let drive_front = Drive {
            thrust: 6.0,
            ..Drive::coasting(8.0)
        };
        let coast = Drive::coasting(8.0);
        let mut worst = 0.0f32;
        for _ in 0..360 {
            front.step(1.0 / 60.0, &drive_front, &lookup);
            back.step(1.0 / 60.0, &coast, &lookup);
            for _ in 0..3 {
                couple(&mut front, &mut back, 1.2, &lookup);
            }
            let gap = (front.position() - back.position()).length();
            worst = worst.max((gap - 1.2).abs());
        }
        assert!(worst < 0.1, "écart : {worst}");
        assert!(back.position().x > 25.0, "tiré : {:?}", back.position());
        assert!(back.speed > 4.0, "{}", back.speed);
    }

    #[test]
    fn carts_that_touch_push_each_other_apart() {
        let track = straight(40);
        let lookup = |p: IVec3| track.get(&p).copied();
        let mut moving = cart_at(10.5, RailShape::EastWest);
        moving.speed = 6.0;
        let mut waiting = cart_at(11.9, RailShape::EastWest);
        let drive = Drive::coasting(8.0);
        for _ in 0..120 {
            moving.step(1.0 / 60.0, &drive, &lookup);
            waiting.step(1.0 / 60.0, &drive, &lookup);
            bump(&mut moving, &mut waiting, 1.1, &lookup);
        }
        // Ils ne se traversent pas, et l'élan est passé au second.
        assert!(moving.position().x < waiting.position().x);
        assert!(waiting.speed > 2.0, "{}", waiting.speed);
    }
}
