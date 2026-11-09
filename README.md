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
crates/engine_ecs       # ECS « sparse set » fait main : entités, requêtes, systèmes, événements
crates/engine_inspect   # trait Inspect (egui) + crates/engine_inspect_derive : #[derive(Inspect)]
crates/engine_render    # rendu wgpu : chunks texturés (opaque + translucide), frustum culling, jour/nuit, captures
crates/engine_voxel     # blocs et fluides (registres moddables), chunks 32³, terrain, lumière, maillage, chargement,
                        # sauvegarde, machines (block entities, capacités, ticks programmés), grilles
crates/engine_mods      # mods : manifestes mod.ron, ordre de chargement, scripts WebAssembly (wasmtime, bac à sable)
crates/mod_sdk          # le kit pour écrire le script d'un mod en Rust (compilé en wasm32)
tools/xtask             # tâches du projet : cargo xtask build-mods
docs/                   # modding.md (écrire un mod), architecture.md (réseaux, fluides, structures mobiles)
games/voxel             # le jeu (E : inventaire et artisanat, clic droit : établi/four/coffre, F3 : inspecteur,
                        # F : vol, T : temps, F1/F2 : rendu, F4 : bras) ; clic gauche maintenu : miner
assets/core/            # blocks.ron (les blocs de base), biomes.ron (15 biomes), ores.ron (minerais par profondeur),
                        # fluids.ron, items.ron (objets, outils, armures), recipes.ron, smelting.ron, loot.ron,
                        # textures/ et items/ (PNG 16×16), sounds/ (WAV)
tools/assetgen          # génère les textures et les sons : cargo run -p assetgen
saves/                  # les mondes sauvegardés (ignoré par git)
assets/core/entities.ron # les créatures (taille, vie, vitesse, boîtes qui les dessinent)
mods/                   # un dossier par mod : mod.ron, blocks.ron, fluids.ron, entities.ron, textures/, sounds/,
                        # script/ (sources) et son .wasm (ex. mods/example : casseur de blocs, rubis)
games/                  # à venir : railway (city builder)
.github/workflows/      # CI sur macOS et Windows : fmt, clippy, test
```

## Mesurer

```sh
cargo run --release -p voxel                             # le jeu, optimisé
RUST_LOG=info,engine_core=debug cargo run -p voxel       # FPS et triangles dans les logs
VOXEL_AUTOFLY=1 cargo run --release -p voxel             # la caméra avance seule : mesure du chargement
cargo bench -p engine_voxel                              # génération d'un chunk, culling contre greedy
cargo bench -p engine_ecs                                # notre ECS contre hecs, objets sur tapis
cargo bench -p engine_voxel --bench machines             # 10k/50k/100k machines, 50k câbles et tuyaux
cargo bench -p engine_mods --bench wasm                  # coût des appels WebAssembly, machines scriptées
cargo bench -p engine_voxel --bench fluids               # un lac qui se vide, une grotte inondée (deux modèles)
VOXEL_STRESS=50000 cargo run --release -p voxel          # test de charge en jeu (machines + entités ECS)
```

Mesures de référence (Apple Silicon, `--release`) :

| | 10 000 | 50 000 | 100 000 |
|---|---|---|---|
| tick de machines, toutes actives (budget 10 ms) | 1,0 ms | 7,0 ms | 16,6 ms |
| pas de simulation de l'ECS, objets sur tapis | 44 µs | 232 µs | |

Végétation : génération d'un chunk de forêt dense 0,39 ms, maillage 0,92 ms ; rendu hors
écran 2560 × 1440 : 9,5 ms par image. Ticks aléatoires (herbe, pousses, feuilles) à moins de
4 chunks du joueur, 24 blocs tirés par chunk et par tick.

Génération du terrain : 0,66 ms pour un chunk souterrain (grottes, minerais), 0,52 ms en
surface, 0,29 ms en moyenne sur une grande zone (les chunks de ciel sont presque gratuits).

50 000 câbles et tuyaux (blocs avec capacités) : ~200 octets chacun en mémoire (~10 Mio),
234 Kio sur disque, sauvegardés en 40 ms.

Variables du jeu :
- `VOXEL_SEED` (graine, 42 par défaut), `VOXEL_RADIUS` (rayon de vue en chunks, 16) ;
- `VOXEL_TIME` (heure de départ : 0 minuit, 0.5 midi, 0.75 coucher), `VOXEL_DAY_LENGTH` (secondes, 600) ;
- `VOXEL_AUTOFLY` (`1` = vitesse du sprint, ou une vitesse en blocs par seconde) ;
- `VOXEL_DEMO=1` (le joueur marche, casse et pose seul) ;
- `VOXEL_SCREENSHOTS=dossier` (enregistre les captures PNG, dont une par biome, puis quitte, même écran éteint) ;
  `VOXEL_SHOTS=biome` ne garde que les captures dont le nom contient ce texte ;
- `VOXEL_HIDE_ARM=1` (cache le bras à la première personne, aussi avec F4) ;
- sans aucune de ces variables, le jeu s'ouvre sur le menu principal (Jouer, Nouveau monde,
  Options, Quitter) ; Échap ouvre la pause. Les options sont dans `settings.ron` ;
- `VOXEL_MENU_DEMO=1` (scénario automatique : menu, un monde, retour au menu, quitter) ;
- `VOXEL_WORLD=nom` (le monde à ouvrir dans `saves/`, « world » par défaut ; sauvegarde toutes les 30 s et à la fermeture) ;
- `VOXEL_MUTE=1`, `VOXEL_VOLUME_EFFECTS`, `VOXEL_VOLUME_AMBIENCE` (volumes de 0 à 1) ;
- `VOXEL_QUIT_AFTER=secondes` (quitter proprement, avec sauvegarde) ;
- `VOXEL_STRESS=N` (N horloges au rythme maximal et N objets sur tapis ; temps dans les logs).

En jeu, modifier un shader, une texture, un son, un `blocks.ron` ou le `.wasm` d'un mod le recharge aussitôt (F5 pour forcer).

Mods : `cargo xtask build-mods` compile les scripts ; voir `docs/modding.md`.

## Avant chaque commit

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
