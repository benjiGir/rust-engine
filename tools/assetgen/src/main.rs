//! `cargo run -p assetgen` : génère les textures et les sons de base du jeu.
//!
//! Cet outil dessine les textures originales « terre, ardoise et cuivre » en pixel-art
//! 16 × 16 et prépare la banque audio à partir de sources créditées et versionnées.
//! Tout est déterministe ; la génération ne demande aucun téléchargement.
//! Les fichiers produits sont versionnés dans le dépôt ; on ne relance l'outil que
//! pour les modifier.

mod creature_skins;
mod doors;
mod fauna_items;
mod magic_items;
mod sounds;
mod texture_pack32;
mod textures;
mod wool_palette;

use std::path::{Path, PathBuf};

use anyhow::Result;

fn main() -> Result<()> {
    // La racine du dépôt, connue à la compilation (`tools/assetgen/../..`).
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.is_empty()
            || args == ["--textures-only"]
            || args == ["--sounds-only"]
            || args == ["--pack32-only"],
        "usage : cargo run -p assetgen -- [--textures-only|--sounds-only|--pack32-only]"
    );
    let mut written: Vec<PathBuf> = Vec::new();
    if args != ["--sounds-only"] && args != ["--pack32-only"] {
        written.extend(textures::generate(&root)?);
    }
    if args != ["--sounds-only"] {
        written.extend(texture_pack32::generate(&root)?);
    }
    if args.is_empty() || args == ["--sounds-only"] {
        written.extend(sounds::generate(&root)?);
    }
    for path in &written {
        println!(
            "écrit : {}",
            path.strip_prefix(&root).unwrap_or(path).display()
        );
    }
    println!("{} fichiers générés", written.len());
    Ok(())
}

/// Un générateur pseudo-aléatoire minuscule (xorshift) : rapide et reproductible.
/// Pas besoin de la crate `rand` pour du bruit décoratif.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // Une graine nulle bloquerait xorshift sur 0 pour toujours.
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Un nombre entre 0 et 1.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Un nombre entre -1 et 1.
    pub fn signed(&mut self) -> f32 {
        self.next_f32() * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(5), Rng::new(5));
        for _ in 0..1000 {
            let x = a.next_f32();
            assert_eq!(x, b.next_f32());
            assert!((0.0..1.0).contains(&x));
        }
    }
}
