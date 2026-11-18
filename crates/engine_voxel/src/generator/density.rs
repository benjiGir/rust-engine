//! La densité 3D du générateur v2 : solide ou vide, bloc par bloc.
//!
//! # Le relief et ses ressauts
//!
//! Une colonne a un relief continu `top` (le dernier bloc plein du relief, quantifié : voir
//! `v2_context.rs`). Presque partout, le bloc d'altitude `y` est solide si et seulement si
//! `y <= top` : le relief est la **seule** règle des plaines, plateaux, côtes, mers, vallées, lits
//! et zones réservées. Aucun bruit ne peut y ouvrir ni y fermer quoi que ce soit.
//!
//! Dans les massifs et leurs contreforts, la géographie ajoute des **escarpements** : le relief
//! saute de 10 à 44 blocs sur une paroi posée sur la structure macro (`scarp.rs`), en quelques blocs de
//! large. Une colonne d'escarpement connaît donc
//!
//! - `base` : le pied de la paroi à cet endroit (le relief sans le ressaut) ;
//! - `top` : le sommet (pied plus la part du ressaut déjà franchie) ;
//! - `jump` : la hauteur totale du ressaut, en blocs ;
//! - `dist` : sa distance horizontale à la ligne de rupture, positive du côté haut (la ligne est le
//!   bord de la paroi ; `dist` est une estimation lisse, quasi linéaire près de la ligne) ;
//! - `reach` : la profondeur maximale (blocs) de l'**entaille** de son pied, 0 presque partout ;
//! - `shape` et `lid` : l'exposant du profil de l'entaille et la fraction de la paroi qu'elle occupe.
//!
//! # L'entaille du pied
//!
//! Là où `reach > 0` (une partie des parois seulement, par plages le long de la ligne), le pied de
//! la paroi est évidé : le bloc `(x, y, z)` de la colonne, à la hauteur relative `t = (y - base) /
//! jump` comprise entre 0 et `lid`, est de l'air quand
//!
//! ```text
//! dist  <  reach × (1 - (t / lid)^shape)  -  0,4 × saillie(y)
//! ```
//!
//! L'entaille est la plus profonde au pied (`reach` blocs derrière la paroi) et se referme jusqu'à
//! `t = lid`. L'exposant `shape` varie lentement le long de la paroi : sous 1, une voûte basse et
//! longue qui se resserre vite ; vers 2, un porche haut qui reste profond avant de se fermer. Au-dessus, le **toit** reste plein : la partie haute de la paroi avance de plusieurs
//! blocs au-dessus du pied, jusqu'à `reach`. La *saillie* des couches (de -1 à 1, par tranche de 3
//! blocs d'altitude : voir `surface.rs`) fait saillir ou reculer certaines couches : des marches
//! d'une couche sur la voûte, la résistance apparente de la roche.
//!
//! # Pourquoi rien ne flotte
//!
//! - l'entaille ne touche jamais le sol ni les blocs sous `base`, ni le niveau de la mer ;
//! - dans une colonne, le vide est un intervalle d'un seul tenant `(base, base + c]` ; comme la
//!   profondeur d'entaille décroît quand `dist` croît, une colonne plus à l'intérieur a un intervalle
//!   plus court : chaque bloc du toit est relié, de proche en proche vers l'intérieur de la masse, à
//!   une colonne dont le vide est plus bas, jusqu'au sol. Le toit (au-dessus de `lid`) est plein sur
//!   toute la masse haute, et relie entre elles les colonnes voisines le long de la paroi ;
//! - `reach` est plafonné par l'épaisseur locale de la masse haute (voir `scarp` : le plus grand
//!   `dist` du voisinage moins une marge) : une dalle étroite n'est pas évidée de part en part, et le
//!   toit garde toujours une colonne pleine pour s'appuyer. Une **arche** n'apparaît que sur une
//!   crête fine reliée à des masses plus épaisses (un pont naturel), jamais sur un pilier isolé ;
//! - le test `relief_v2.rs` le vérifie sur les blocs finals (chaque composante solide touche le sol).

use super::surface::StrataColumn;
use super::v2::CoreCell;
use crate::chunk::CHUNK_SIZE;

/// Profondeur maximale de l'entaille d'un pied de paroi, en blocs (jusqu'à 1,7 fois sous une crête
/// fine : une arche).
pub(super) const REACH_MAX: f32 = 10.0;
/// Amplitude (blocs) de la saillie des couches dans l'entaille.
const LEDGE: f32 = 0.4;
/// Lignes lues sous et au-dessus du chunk : le sable ne doit pas tomber (bloc dessous) et la
/// profondeur de sol se compte depuis l'air (jusqu'à 8 blocs au-dessus).
pub(super) const BELOW: i32 = 1;
pub(super) const ABOVE: i32 = 8;
pub(super) const SPAN: usize = CHUNK_SIZE + (BELOW + ABOVE) as usize;

/// Le bloc d'altitude `y` d'une colonne d'escarpement est-il vidé par l'entaille ? (Sous `top`.)
fn notched(cell: &CoreCell, y: i32, ledge: f32) -> bool {
    if cell.reach <= 0.0 || y <= cell.base as i32 {
        return false;
    }
    let t = (y - cell.base as i32) as f32 / cell.jump.max(1.0);
    if t >= cell.lid {
        return false;
    }
    let depth = cell.reach * (1.0 - (t / cell.lid).powf(cell.shape)) - LEDGE * ledge;
    cell.dist < depth
}

/// Solide ou vide, pour `SPAN` blocs de la colonne `(lx, lz)` à partir de `origin_y - BELOW`.
pub(super) fn solid_column(core: &CoreCell, strata: &StrataColumn, origin_y: i32) -> [bool; SPAN] {
    let top = core.top as i32;
    let mut out = [false; SPAN];
    let base = origin_y - BELOW;
    for (i, cell) in out.iter_mut().enumerate() {
        let y = base + i as i32;
        *cell = y <= top
            && (y <= super::SEA_LEVEL + 1
                || core.reach <= 0.0
                || !notched(core, y, strata.ledge(y)));
    }
    out
}

/// Le nombre de blocs solides consécutifs au-dessus de l'indice `idx` : la profondeur sous l'air.
pub(super) fn depth_below_air(solid: &[bool; SPAN], idx: usize) -> i32 {
    solid[idx + 1..].iter().take_while(|&&s| s).count() as i32
}

/// Le bloc solide le plus haut d'une colonne d'escarpement : on descend du relief jusqu'au premier
/// solide (une paroi à demi franchie peut avoir toute sa partie haute évidée, jamais le pied).
pub(super) fn real_top(strata: &StrataColumn, cell: &CoreCell) -> i32 {
    let top = cell.top as i32;
    if cell.reach <= 0.0 {
        return top;
    }
    for y in (cell.base as i32 + 1..=top).rev() {
        if y <= super::SEA_LEVEL + 1 || !notched(cell, y, strata.ledge(y)) {
            return y;
        }
    }
    cell.base as i32
}
