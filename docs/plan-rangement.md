# Plan : ranger les fichiers par domaine

Trois dossiers `src/` sont devenus des listes à plat difficiles à parcourir :

| Dossier | Fichiers `.rs` à plat | Sous-dossiers |
|---|---|---|
| `games/voxel_sim/src` | 112 | 1 (`bin/`) |
| `games/voxel/src` | 74 | 1 (`quest_ui/`) |
| `crates/engine_voxel/src` | 55 | 6 |
| les autres crates | 12 au plus | — |

Les noms portent aujourd'hui le rangement qui manque : des préfixes répétés (`enchant_books`,
`enchant_data`, `enchant_effects`, `enchant_library`, `enchant_table`, `enchant_ui_server`) et des
suffixes de côté (`ghosts.rs` dans la simulation, `ghosts_window.rs` dans le jeu ; `storage_ui.rs` dans
le jeu, `storage_ui_server.rs` dans la simulation). Le jeu réexporte aussi les 112 modules de la
simulation, un par un, en tête de `main.rs`.

## Les règles

1. **Un dossier par domaine du jeu**, avec **les mêmes noms dans la simulation et dans le jeu** :
   `enchanting/` côté `voxel_sim` contient les règles, `enchanting/` côté `voxel` contient les écrans.
   On trouve l'autre moitié d'un fichier au même chemin dans l'autre crate.
2. **Le fichier parent porte le cœur du domaine** (`enchanting.rs` à côté de `enchanting/`, comme
   `quest_ui.rs` et `quest_ui/` aujourd'hui) et déclare ses sous-modules. Pas de `mod.rs`, et **pas de
   réexportation** : on écrit le chemin complet (`crate::enchanting::table::TableStats`), qui dit d'où
   vient chaque chose.
3. **Le dossier remplace le préfixe, la crate remplace le suffixe** : `enchant_table.rs` devient
   `enchanting/table.rs` ; `storage_ui_server.rs` (simulation) et `storage_ui.rs` (jeu) deviennent tous
   deux `storage/ui.rs`, chacun dans sa crate.
4. **Pas plus de 20 fichiers par dossier**, vérifié par `cargo xtask check-arch` comme la limite de
   2 500 lignes par fichier.
5. **Le déplacement ne change pas le code** : `git mv` (l'historique suit), les chemins mis à jour, rien
   d'autre. Chaque lot compile, passe les tests et se commite seul.

## La simulation (`games/voxel_sim/src`, 112 fichiers)

À la racine, ce qui sert à tout le monde (10 fichiers) : `lib.rs`, `paths.rs`, `logs.rs`, `launch.rs`,
`settings.rs`, `common.rs`, `content.rs`, `components.rs`, `events.rs`, `shared.rs`.

| Dossier | Fichiers (ancien nom → nouveau) | Nombre |
|---|---|---|
| `server/` (avec `server.rs`, l'ancien `server_side.rs`) | `server_boot` → `boot`, `server_step` → `step`, `server_sends` → `sends`, `server_commands` → `commands`, `intents`, `interact_server` → `interact`, `console_server` → `console`, `death_server` → `death`, `ui_server` → `ui`, `screens`, `screen_actions`, `world_edit`, `hot_reload`, `stress`, `stats`, `json_text` | 16 |
| `net/` | `protocol`, `wire`, `net_host` → `host`, `net_join` → `join`, `local_link`, `dedicated`, `predicted`, `player_view`, `entities`, `sky_view`, `bot`, `bot_swarm`, `ghosts`, `multiplayer_tests` → `tests` | 14 |
| `world/` | `dimensions`, `daytime`, `weather`, `persistence`, `soundscape`, `sound_ids`, `texture_pack_config` → `texture_pack` | 7 |
| `player/` (avec `player.rs`) | `players`, `movement`, `inventory`, `gear`, `attributes`, `survival`, `potions`, `drops`, `item_data`, `bucket_play` → `bucket`, `paired_play` → `paired`, `tone` | 12 |
| `mobs/` (avec `mobs.rs`) | `mob_brains` → `brains`, `mob_model` → `model`, `aquatic`, `breeding`, `elites`, `villagers`, `trading`, `sheep_colors`, `projectiles`, `spawning`, `navigation` | 11 |
| `enchanting/` (avec `enchanting.rs`) | `enchant_books` → `books`, `enchant_data` → `data`, `enchant_effects` → `effects`, `enchant_library` → `library`, `enchant_table` → `table`, `enchant_ui_server` → `ui`, `book_play` | 7 |
| `adventure/` (avec `adventure.rs`) | `adventure_data` → `data`, `affixes`, `gems`, `gem_sockets`, `charms`, `charm_play`, `tiers`, `tiers_ui_server` → `tiers_ui`, `forge_tables`, `forge_ui_server` → `forge_ui` | 10 |
| `quests/` (avec `quests.rs`) | `quest_play` → `play`, `quest_progress` → `progress`, `quest_ui_server` → `ui`, `guide`, `guide_ui_server` → `guide_ui` | 5 |
| `storage/` (avec `storage.rs`, neuf, qui déclare) | `backpacks`, `backpack_block`, `storage_ui_server` → `ui`, `transfer`, `container_view` | 5 |
| `vehicles/` (avec `vehicles.rs`, neuf) | `carts`, `cart_play`, `ships` | 3 |
| `shaping/` (avec `shaping.rs`, neuf) | `shape_materials` → `materials`, `shaped_play` → `play`, `shaping_ui_server` → `ui` | 3 |
| `modding/` (avec `modding.rs`) | `mod_menu` → `menu`, `player_api_server` → `player_api` | 2 |

Le binaire `bin/voxel-server.rs` ne bouge pas.

## Le jeu (`games/voxel/src`, 74 fichiers)

À la racine (4 fichiers) : `main.rs`, `assets.rs`, `audio.rs`, `key_codes.rs`.

| Dossier | Fichiers (ancien nom → nouveau) | Nombre |
|---|---|---|
| `client/` (avec `client.rs`, l'ancien `client_side.rs`) | `client_world` → `world`, `client_entities` → `entities`, `net_client` → `join`, `stream_upload`, `predicted_window` → `predicted`, `player_view_window` → `player_view`, `sky_view_window` → `sky_view`, `intents_window` → `intents`, `interact`, `play_input` → `input`, `death`, `culling`, `screen_actions_window` → `screen_actions` | 13 |
| `render/` | `entity_render` → `entities`, `mob_render` → `mobs`, `mob_render_tests` → `mobs_tests`, `item_art`, `item_models`, `hand`, `hand_model`, `chest_lids`, `conduit_fx`, `conduit_render` → `conduits`, `water_fx`, `weather_render` → `weather`, `far_terrain`, `shader_packs`, `texture_packs` | 15 |
| `ui/` (avec `ui.rs`) | `ui_info` → `info`, `ui_input` → `input`, `ui_inspect` → `inspect`, `ui_layout` → `layout`, `ui_options` → `options`, `ui_target` → `target`, `ui_widgets` → `widgets`, `skin`, `menus`, `console`, `recipe_browser`, `chest_preview` | 12 |
| `control/` (avec `control.rs`) | `adventure_ctl` → `adventure`, `enchant_ctl` → `enchanting`, `item_ctl` → `items`, `net_ctl` → `net`, `quest_ctl` → `quests`, `shape_ctl` → `shaping`, `storage_ctl` → `storage`, `ghosts_window` → `ghosts`, `bench`, `screenshots`, `shapes_scene` | 11 |
| `adventure/` | `tiers_ui`, `forge_ui`, `attributes_ui` → `attributes` | 3 |
| `enchanting/` | `enchant_ui` → `ui` | 1 |
| `quests/` | `quest_ui` → `ui` (et son dossier `ui/`), `guide_ui` | 2 |
| `storage/` | `storage_ui` → `ui`, `backpacks_window` → `backpacks`, `container_view_window` → `container_view` | 3 |
| `vehicles/` | `cart_play_window` → `cart_play`, `ships_window` → `ships` | 2 |
| `shaping/` | `shaping_ui` → `ui`, `shaped_play_window` → `play` | 2 |
| `modding/` | `mod_menu_window` → `menu`, `player_api` | 2 |
| `world/` | `dimensions_window` → `dimensions` | 1 |

Les petits dossiers de domaine (1 à 3 fichiers) sont voulus : ils sont le miroir de ceux de la
simulation, et ils grandiront avec le jeu.

## Le moteur (`crates/engine_voxel/src`, 55 fichiers)

Son interface publique ne change pas : `lib.rs` exporte déjà les types à la racine de la crate
(`engine_voxel::BlockRegistry`), et seuls `shaped`, `double_chest`, `paired_block`, `torch`, `door`,
`bed` et `density_graph` sont appelés par leur chemin de module depuis les autres crates. Le rangement
reste interne.

| Dossier | Fichiers | Nombre |
|---|---|---|
| racine | `lib.rs`, `block.rs`, `registry.rs`, `items.rs`, `chunk.rs`, `world.rs`, `grid.rs`, `light.rs`, `raycast.rs`, `physics.rs`, `edited_surface.rs`, `visibility.rs`, `save.rs` | 13 |
| `streaming/` (existe) | + `chunk_sender`, `pipeline_timing`, `transfer` | +3 |
| `worldgen/` | `generation`, `generator` (et son dossier), `density_graph` (et son dossier), `biome`, `dimension`, `trees`, `plants`, `far`, `far_props`, `worldmap`, `worldmap_blocks` | 11 |
| `blocks/` | `bed`, `door`, `paired_block`, `double_chest`, `block_rules`, `farming`, `connections` | 7 |
| `tech/` (avec `tech.rs`) | `machines`, `networks`, `network_visual`, `circuits`, `conduit_shell`, `containers`, `storage`, `capabilities`, `crafting`, `recipe_index` | 10 |
| `motion/` | `subgrid`, `ship`, `rails`, `rail_motion`, `rail_save`, `pathfind` | 6 |
| `fluids/` (avec `fluids.rs`) | `fluid_sim` → `sim` (et son dossier) | 1 |
| `mesh/`, `shaped/` (existent) | inchangés | — |

## Découpage

| Lot | Contenu | Vérification |
|---|---|---|
| **R1, la simulation (réalisé)** | les 112 fichiers de `voxel_sim` dans leurs dossiers, renommés ; les chemins mis à jour dans `voxel_sim` et dans `voxel` ; la liste des réexportations de `main.rs` remplacée | fmt, clippy, les 1 199 tests, `check-arch` ; le jeu et `voxel-server` démarrent |
| **R2, le jeu** | les 74 fichiers de `voxel` dans leurs dossiers, renommés | idem, et un coup d'œil en jeu (menus, écrans, autre joueur) |
| **R3, le moteur** | les 55 fichiers d'`engine_voxel` dans leurs dossiers, interface publique inchangée | idem, et `cargo xtask build-mods` |
| **R4, la règle et la doc** | `check-arch` refuse plus de 20 fichiers `.rs` par dossier ; `docs/architecture.md` et les docs qui citent des chemins mis à jour | `check-arch` passe, et échoue sur un dossier de 21 fichiers |

Chaque lot se fait sur un arbre propre (pas de travail en cours dans les fichiers déplacés) : un fichier
déplacé pendant qu'on l'édite ailleurs crée un conflit. Les plans déjà réalisés (`docs/plan-*.md`)
gardent leurs anciens chemins : ils racontent l'histoire.

## Réalisé

- **R1** : les 112 fichiers de `voxel_sim` sont rangés en 12 dossiers ; la racine garde 11 fichiers
  (`lib.rs` et les 10 communs), le plus gros dossier (`server/`) en a 16. `git mv` partout : l'historique
  suit. Les chemins ont été réécrits par un script (les `use`, les chemins complets, les usages nus des
  modules renommés), puis `cargo fix` a retiré les imports devenus inutiles. Le jeu n'a plus la liste des
  112 réexportations : il écrit `voxel_sim::player::inventory`. Les exemples de `games/voxel/examples` ne
  recompilent plus de fichiers de la simulation par `#[path]` : ils passent par `voxel_sim::`. Un écart au
  plan : `players.rs` garde son nom (`player::players`), `player::table` aurait été obscur. Les
  `include_str!` des fichiers descendus d'un niveau ont pris un `../` de plus. Vérifié : fmt, clippy, les
  1 199 tests, `check-arch`, le jeu et `voxel-server` démarrent.

## Choix à valider

| Question | Proposition | Pourquoi |
|---|---|---|
| Renommer en même temps que déplacer | oui (`enchant_table` → `enchanting/table`, sans `_server` ni `_window`) | sinon `enchanting/enchant_table.rs` répète le dossier ; un seul passage sur les chemins au lieu de deux |
| Comment le jeu appelle la simulation | par `voxel_sim::` directement (`voxel_sim::enchanting::table`), plus de réexportation | on voit tout de suite si un code vient de la simulation ou de la fenêtre ; les 112 noms de `main.rs` disparaissent |
| La limite de fichiers par dossier | 20, vérifiée par `check-arch` | assez pour les gros domaines (`server/` en a 16), assez bas pour ne pas revenir à une liste à plat |
| Le moteur | dans ce plan (R3), après le jeu | 55 fichiers, mais le risque est faible : son interface publique ne bouge pas |
