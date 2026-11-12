# Brief : passe visuelle sur les blocs

À donner à Codex. Tout ce qu'il faut savoir est ici ou dans les documents cités ; en cas de doute
sur une règle du jeu, lire le code plutôt que deviner.

## Objectif

Rapprocher les blocs de ce qu'ils représentent. Aujourd'hui presque tous les blocs « objets » du jeu
de base sont des **cubes pleins** avec une texture : le coffre, le four, le lit, la torche, le levier,
le bouton, le répéteur, le tuyau, le câble, la table d'enchantement, l'alambic, la terre labourée, le
cactus, le piston... Il s'agit de leur donner **une vraie forme** (boîtes, orientation) et **de
meilleures textures**, sans rien changer au gameplay.

Deux leviers :
1. **Les formes** : `boxes` (en seizièmes de bloc) et `states` dans `blocks.ron`. Documentation :
   `docs/modding.md`, sections « Les formes et les états de bloc » et « Les animations de blocs ».
2. **Les textures** : PNG RGBA 16 × 16, dessinés par `tools/assetgen`. Direction artistique :
   `docs/art-direction.md` (« terre, ardoise et cuivre », dessins originaux, rien venant de Minecraft).

## Règles à ne pas enfreindre

- **L'ordre des blocs dans `assets/core/blocks.ron` fixe les identifiants numériques** : ne jamais
  réordonner, supprimer ni insérer au milieu. Ajouter à la fin seulement, et seulement si c'est
  nécessaire. Ne pas renommer un `id` (les sauvegardes, les recettes, `loot.ron`, les mods et les
  scripts s'y réfèrent).
- Ne pas toucher au gameplay : `hardness`, `tool`, `tier`, `behavior`, `ports`, `conduit`, `channel`,
  `support`, `gravity`, `loot.ron`, les recettes, les sons.
- **Ne jamais modifier `saves/`** (des mondes du propriétaire y sont) ni `.idea/`.
- Pas de PNG écrit à la main sans passer par le générateur : les textures sont **versionnées mais
  produites par `tools/assetgen`** (déterministe). Modifier le générateur (`tools/assetgen/src/textures.rs`
  et `textures/items.rs`), puis relancer :
  ```sh
  cargo run -p assetgen -- --textures-only
  ```
  Sinon la prochaine régénération écrase le travail.
- Les champs `color` des blocs et objets sont des **moyennes linéaires** des PNG (sRGB, pixels
  transparents exclus) : les recalculer quand une texture change sensiblement (ils servent au terrain
  lointain, aux objets au sol et au repli sans texture).
- Le four garde sa bouche aux pixels `x = 4..11`, `y = 8..13` de `furnace_front.png` (les flammes
  animées s'y affichent).
- Dessin original uniquement. Palette ramenée à 32 niveaux par canal, grain périodique pour l'herbe et
  la terre (voir `art-direction.md`).

## Ce qu'une forme change vraiment

Une forme à `boxes` sert au **dessin, à la collision et à la sélection** : le joueur marche sur une
dalle, passe sous une dalle haute, et le rayon passe à côté. Conséquences à vérifier pour chaque bloc
transformé :
- un coffre, un lit ou une table plus bas qu'un cube changent la façon dont on marche dessus et dont on
  les vise ;
- les voisins d'un bloc à boîtes gardent leurs faces (il est « transparent » pour eux) ;
- sa lumière est arrêtée s'il est `solid`, sinon elle passe (`light_opacity` pour régler) ;
- les **créatures le traitent encore comme un cube plein** (limite connue, ne pas la « corriger » ici) ;
- la texture est **coupée**, pas étirée : une boîte de 8/16 de haut montre la moitié de sa texture.
  Adapter les textures (ou découper en plusieurs `textures`) pour que le résultat soit juste.

### Points délicats (lire le code avant)

- **Les paires qui se remplacent** (`lever`/`lever_on`, `button`/`button_on`, `gate`/`gate_open`,
  `repeater`/`repeater_on`, `piston`/`piston_on`, `lamp`/`lamp_lit`, `power_lamp`/`power_lamp_on`...) :
  le jeu échange un bloc contre l'autre. Si on ajoute `states` (orientation) à l'un, vérifier que
  l'échange conserve l'orientation. Si ce n'est pas le cas, **ne pas ajouter d'états** à ces blocs :
  se limiter à une forme fixe, et le signaler plutôt que de modifier la logique.
- **Les blocs en `Shape::Cross`** (37 plantes, déjà en croix) : améliorer les textures seulement.
- **Eau, lave, verre, glace** : rendu translucide (`alpha`, passe séparée) ; ne pas changer le mode de
  rendu.
- **Les blocs à machine** (`core:chest`, `core:furnace`, `core:brewing_stand`...) : l'inventaire et le
  clic droit ne dépendent pas de la forme, mais vérifier en jeu qu'on peut toujours les viser et les
  ouvrir.
- Au plus 64 variantes par bloc à états ; seule la base est dans la barre de blocs.
- Les blocs de mods font partie du travail (voir plus bas), mais un mod ne doit utiliser que des
  champs documentés de `blocks.ron`.

## Où est quoi

| Quoi | Où |
|---|---|
| Blocs du jeu de base | `assets/core/blocks.ron`, textures dans `assets/core/textures/` (139 PNG) |
| Icônes des objets | `assets/core/items/` (91 PNG, 16 × 16) ; icône d'un bloc = miniature isométrique 32 × 32 calculée (`games/voxel/src/item_art.rs`) |
| Générateur des textures | `tools/assetgen/src/textures.rs`, `tools/assetgen/src/textures/items.rs` |
| Outils en main | `games/voxel/src/hand.rs` (points de prise) |
| Thème de l'interface | `games/voxel/src/skin.rs` |
| Blocs de mods | `mods/<mod>/blocks.ron`, `mods/<mod>/textures/`, `mods/<mod>/items/` |

Mods : `example` (5 blocs), `mecanique` (7), `magie` (1), `donjons` (4), `entrepot` (4), `forge` (2).
Les textures de `example` et `mecanique` sont déjà dans `assetgen`. Celles de **`donjons`, `magie`,
`entrepot` et `forge` ont été dessinées par de petits scripts jetables** (et sont à refaire proprement,
dans `assetgen`, avec le même style que le reste). Elles sont dans `mods/<mod>/textures/` et `items/`.

## Candidats évidents (cubes à transformer)

Priorité haute (la silhouette change tout) : `chest`, `furnace`, `workbench`, `bed`, `torch`, `lever`,
`button`, `repeater`, `enchanting_table`, `brewing_stand`, `farmland`/`farmland_wet`, `cactus`,
`pipe`, `cable`, `shaft`, `piston`/`sticky_piston`/`piston_head`, `tank`, `battery`, `solar_panel`,
`pump`, `motor`, `fan`, `propeller`, `helm`, `balloon`.
Mods : `entrepot:vault`, `forge:anvil` (déjà en boîtes, à peaufiner), `donjons:spike_trap` (déjà en
boîtes), `magie:rune_table`, `mecanique:*` (déjà en boîtes pour arbres et engrenages).

Priorité moyenne : textures plus fines pour les minerais, les bûches, les feuilles, la pierre, le
sable, la neige, la glace ; cohérence entre les 5 essences de bois.

## Vérifier en jeu

Le jeu se pilote par commandes (`cargo xtask ctl help`). Lancer avec un monde de test **à son nom**
(les mondes de test sont à supprimer par le propriétaire, ne pas en réutiliser un existant d'un autre) :

```sh
VOXEL_MUTE=1 VOXEL_CONTROL=1 VOXEL_SEED=7 VOXEL_WORLD=visuel cargo run -p voxel
```

puis, dans un autre terminal (la commande entière dans **un seul** argument entre guillemets) :

```sh
cargo xtask ctl "mode fly"
cargo xtask ctl "place 2 71 16 core:chest"        # poser un bloc
cargo xtask ctl "goto 2.5 74 12"                  # se placer
cargo xtask ctl "lookat 2.5 71.5 16.5"            # viser
cargo xtask ctl "screenshot shots/chest.png"      # capture (1280 × 720 par défaut)
cargo xtask ctl "use"                             # clic droit (ouvre un coffre, bascule un levier)
cargo xtask ctl "break 2 71 16"                   # casser
cargo xtask ctl quit
```

Notes : le sol de la zone de départ est à `y = 70`, les arbres gênent parfois la visée (changer de
colonne). `place` n'oriente pas : une variante orientée se pose avec `use` en visant. `texts` /
`clicktext` / `click` / `type` / `key` pilotent les menus. Modifier une texture ou un `blocks.ron` en
cours de partie le recharge aussitôt (F5 pour forcer). Lire les captures avec un outil d'image, ne
pas se fier aux seuls tests. **Supprimer le dossier `shots/` à la fin.**

Planche de contrôle sans fenêtre (thème, objets, blocs) :

```sh
cargo run -p voxel --example skin_preview -- docs/images/skin-preview.png
```

Pour chaque bloc transformé, regarder au minimum : de face, de côté, de dessus, posé contre un mur, à
côté d'un bloc identique (raccord des textures), de nuit, et vérifier qu'on peut marcher dessus, le
viser, le casser, et (pour une machine) l'ouvrir.

## Contrôles avant de rendre

Tous doivent rester verts :

```sh
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo xtask check-arch        # aucun .rs au-dessus de 2500 lignes
cargo xtask hooks --check
cargo xtask docs --check
```

Si un test attend l'ancien aspect (couleur moyenne, taille, forme), le mettre à jour **en expliquant
pourquoi** plutôt que de l'assouplir. Si la passe rend `docs/art-direction.md` faux, le corriger.
Garder `mods/*/*.wasm` à jour avec `cargo xtask build-mods` seulement si un script a changé (ce brief
n'en demande pas).

## Façon de rendre

- Un lot cohérent à la fois (par exemple : « conteneurs et machines », puis « redstone », puis « bois
  et pierre »), chacun avec ses captures avant/après et les contrôles verts.
- Message de commit en français, une ligne de titre puis un court corps. Ne pas committer
  `.idea/rust-game-engine.iml`.
- Signaler tout ce qui n'a pas pu être fait ou vérifié (une paire de blocs dont l'orientation ne se
  conserve pas, une collision étrange, une texture qui ne se raccorde pas) au lieu de le contourner.
