# rust-game-engine

Un moteur de jeu en Rust, écrit pour apprendre le langage : d'abord un moteur voxel façon Minecraft, puis un city builder ferroviaire.

Plan complet : https://claude.ai/artifact/8Sw3uqhzc4MZboC8xzEF25

## Démarrer

```sh
xcode-select --install                                   # une seule fois
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
cargo run -p game_of_life                                # lance l'exercice de la phase 0
cargo test                                               # lance les tests
```

`rust-toolchain.toml` installe automatiquement `rustfmt`, `clippy` et la cible Windows.

## Phase 0 : le jeu de la vie

Le code de départ est dans `exercises/game_of_life/src/main.rs`.
Écris `live_neighbours` puis `step`, retire les `#[ignore]` des tests correspondants, et relance `cargo test`.

## Structure

```
Cargo.toml              # workspace : ajoute chaque nouvelle crate dans `members`
exercises/              # exercices d'apprentissage (phase 0)
crates/engine_core      # fenêtre, boucle à pas fixe, entrées
crates/engine_render    # rendu wgpu : cubes instanciés, maillages de chunks, frustum culling
crates/engine_voxel     # blocs, chunks 32³, terrain procédural, maillage, chargement en tâche de fond
games/voxel             # le mini-Minecraft (F1 : culling / greedy, F2 : frustum culling)
games/                  # à venir : railway (city builder)
.github/workflows/      # CI sur macOS et Windows : fmt, clippy, test
```

## Mesurer

```sh
cargo run --release -p voxel                             # le jeu, optimisé
RUST_LOG=info,engine_core=debug cargo run -p voxel       # FPS et triangles dans les logs
VOXEL_AUTOFLY=1 cargo run --release -p voxel             # la caméra avance seule : mesure du chargement
cargo bench -p engine_voxel                              # génération d'un chunk, culling contre greedy
```

Variables du jeu : `VOXEL_SEED` (graine, 42 par défaut), `VOXEL_RADIUS` (rayon de vue en chunks, 16),
`VOXEL_AUTOFLY` (`1` = vitesse du sprint, ou une vitesse en blocs par seconde).

## Avant chaque commit

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
