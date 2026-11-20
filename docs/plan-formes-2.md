# Plan : formes de blocs, deuxième vague

Suite de `docs/plan-formes.md` (couches, dalles, pentes, microblocs et grains, déjà faits : `core:shaped`,
`engine_voxel::shaped`, `mesh/shaped.rs`, l'établi de taille). Idées reprises de **Create: Copycats+**
(<https://github.com/copycats-plus/copycats>) et de son extension **Copycats+ Additions**. Leur licence
est « tous droits réservés » : on reprend **les idées de formes**, jamais le code, les modèles ni les
textures. Les dimensions ci-dessous sont les nôtres.

## Ce qu'on ajoute (choisi le 20 nov. 2026)

1. **Les pièces en boîtes** : escaliers (et verticaux), marche verticale, poutre, tranches redimensionnables
   (d'arête et de coin), demi-couches, demi-panneaux, panneaux d'un quart, planchettes.
2. **Les pentes avancées** : pentes en coin (sortant et rentrant), pentes posées sur un mur.
3. **Les formes qui se raccordent** : muret, barrière, portillon, vitre, vitre à plat, taillés dans
   n'importe quel matériau.

Hors sujet : bloc fantôme, portes, trappes, boutons, plaques, échelles taillés.

Tout reste dans le modèle existant : une case `core:shaped`, des pièces au seizième, chacune son
matériau, plusieurs matériaux par case, la grille d'occupation 16³ pour les recouvrements et les faces
cachées. Le format monte d'une version (`VERSION = 2`) et relit la version 1.

## 1. Les pièces en boîtes

### Le modèle

Une nouvelle pièce générique :

```rust
/// Un pavé au seizième, avec la sorte d'objet dont il vient et combien d'objets il vaut
/// (ce qu'il rend quand on le casse).
Piece::Cuboid { min: [u8; 3], max: [u8; 3], kind: PieceKind, count: u8 }
/// Un escalier : une dalle et une demi-dalle, ou sa version verticale (un coin de 3/4).
Piece::Stairs { facing: u8, ceiling: bool, vertical: bool }
```

Les formes « boîtes » sont des **préréglages** qui fabriquent un `Cuboid` selon le point visé, la face et le
regard (`plan_inner` / `plan_outer`), comme les couches aujourd'hui :

| Pièce (objet) | Géométrie (en seizièmes) | Orientation | Redimensionnable | Rendement par bloc |
|---|---|---|---|---|
| Escalier `core:shape_stairs` | dalle 16 × 8 × 16 + marche 16 × 8 × 8 | 4 directions × haut / bas | non | 1 (on garde 3/4 de la matière, comme un escalier) |
| Escalier vertical `core:shape_vertical_stairs` | trois quarts d'une colonne (un coin vide) | 4 coins | non | 1 |
| Marche verticale `core:shape_step` | colonne 8 × 16 × 8 dans un coin | 4 coins (et couchée sur X ou Z) | non | 4 |
| Poutre `core:shape_beam` | colonne 8 × 16 × 8 **centrée** | axe X, Y ou Z | non | 4 |
| Tranche `core:shape_slice` | barre de n × n le long d'une arête, n = 1 à 8 | 12 arêtes | oui : clic droit avec une tranche sur la tranche, +1 | 64 (une tranche de 1 × 1 vaut 1 ; une de n × n en vaut n²... voir la règle) |
| Tranche de coin `core:shape_corner_slice` | cube n × n × n dans un coin, n = 1 à 8 | 8 coins | oui | même règle |
| Demi-couche `core:shape_half_layer` | moitié d'une couche (16 × e × 8), e = 1 à 16 | 6 faces × 2 moitiés | oui, s'empile comme une couche | 32 |
| Demi-panneau `core:shape_half_panel` | 16 × 8 × 2 sur une face, une moitié | 6 faces × 4 moitiés | non | 16 |
| Panneau d'un quart `core:shape_quarter_panel` | 8 × 8 × 2 dans un coin d'une face | 6 faces × 4 coins | non | 32 |
| Planchette `core:shape_board` | 16 × 16 × 1 collée à une face ; plusieurs dans la même case | 6 faces | non | 16 |

**La règle des redimensionnables** (tranches) : un objet « tranche » vaut un volume de 1 × 1 × 16 (ou 1³ × 8
pour le coin). Grossir une tranche de n à n + 1 consomme (n + 1)² - n² objets (le jeu les retire de la pile
tenue ; il refuse s'il n'y en a pas assez). Casser rend `count` objets. Ainsi rien ne se perd ni ne se crée.

La planchette existe déjà presque : c'est une couche de 1 ; on l'expose comme objet à part pour que
plusieurs faces d'une même case en reçoivent une (le modèle le permet déjà : pas de recouvrement entre
deux couches de 1 sur des faces opposées ; aux arêtes, la seconde est rognée de 1 pour ne pas chevaucher,
ou refusée — choisir et tester).

### Ce qu'il faut toucher

- `shaped.rs` : les nouvelles pièces (occupation, boîtes, validation, fusion des tranches et demi-couches
  de même matériau, `turned` pour les vaisseaux, `drops`, format v2 avec relecture de la v1), les sortes
  d'objets (`PieceKind`) et leurs rendements.
- `mesh/shaped.rs` : un `Cuboid` et un `Stairs` se dessinent comme des boîtes (la routine existe).
- `items.ron`, icônes (`item_art.rs`), textures d'objets (`tools/assetgen`, dans le style existant), noms
  (« Escalier de chêne »...).

## 2. Les pentes avancées

### Le modèle

Généraliser la pente :

```rust
Piece::Slope { base: Face, up: Face, level: u8, corner: Corner }   // Corner::{None, Outer, Inner}
```

- `base` : la face d'appui (le sol `NegY`, le plafond `PosY`, ou un mur) ; `up` : la face vers laquelle la
  pente monte (perpendiculaire à `base`). 6 × 4 = 24 orientations ; les pentes actuelles sont
  `base = NegY` (ou `PosY` au plafond) et `up` = leur `facing`.
- **Pente en coin sortant** : la hauteur en un point vaut `level × min(u, v)` (u, v : les deux distances
  vers les bords hauts) ; **rentrant** : `level × max(u, v)`. Mêmes 16 niveaux, même empilement (une couche
  de pente en coin de même orientation et de même matériau monte d'un niveau).
- Relire les pentes v1 dans la nouvelle forme.

### Ce qu'il faut toucher

- `shaped.rs` : occupation (colonne par colonne, au seizième), boîtes en escalier, `turned`, placement :
  viser un mur pose une pente murale ; deux nouveaux objets `core:shape_corner_slope_layer` (coin sortant)
  et `core:shape_inner_slope_layer` (coin rentrant), rendement 16 ; le coin à poser se choisit selon le
  quart de la face visé.
- `mesh/shaped.rs` : le plan incliné d'un coin est fait de **deux triangles** (sortant : une pyramide ;
  rentrant : deux plans qui se rejoignent) ; les côtés sont des triangles ou des quads selon le cas.
  Généraliser l'orientation existante (`orient`) aux 24 orientations.
- Collision : l'escalier de seizièmes suit la nouvelle hauteur (le joueur monte une pente en coin).

## 3. Les formes qui se raccordent

### Le modèle

```rust
Piece::Connected { kind: Link, open: bool }   // Link::{Wall, Fence, Gate, Pane, FlatPane}
```

- Une forme raccordée **occupe seule sa case** (pas d'autre pièce avec elle) : c'est plus simple pour les
  raccords et c'est l'usage normal.
- Sa géométrie dépend des **voisins** (comme les conduits, voir `connections.rs`) :
  - muret : poteau central 8 × 16 × 8 et bras de 6 de large, 14 de haut vers chaque voisin raccordé ; le
    poteau monte à 16 seulement s'il y a un muret au-dessus ou un coin ;
  - barrière : poteau 4 × 16 × 4 et deux traverses vers chaque voisin ; la collision monte à 24 seizièmes
    (on ne saute pas par-dessus, comme Minecraft) ;
  - portillon : se raccorde aux barrières et murets ; un clic droit l'ouvre (`open`) : plus de collision, le
    dessin tourne à 90° ;
  - vitre : panneau de 2 d'épaisseur, bras vers les voisins ; vitre à plat : la même posée horizontalement
    (raccords dans le plan).
- **Qui se raccorde à qui** : une forme de la même famille (muret ↔ muret, barrière ↔ barrière et
  portillon, vitre ↔ vitre), et les blocs cubes pleins opaques (muret, barrière, vitre).
- Les boîtes (collision, visée) et le maillage lisent les voisins : `connections::boxes_at` et le
  `Neighbourhood` ont déjà accès aux blocs voisins et à leurs entités.
- Quand un voisin change, la case doit être **remaillée** (le streamer remaille déjà les voisins d'un bloc
  modifié ; vérifier que c'est vrai pour les cases de formes).

### Objets

`core:shape_wall` (Muret, rendement 1), `core:shape_fence` (Barrière, 2), `core:shape_gate` (Portillon, 1),
`core:shape_pane` (Vitre, 8), `core:shape_flat_pane` (Vitre à plat, 8). Une vitre d'un matériau opaque est
une simple grille pleine : c'est voulu (une vitre de verre reste transparente).

## 4. L'établi de taille

La grille des formes passe de 6 à environ 23 entrées : la ranger par familles (couches et dalles ; pentes ;
blocs et escaliers ; tranches et panneaux ; raccordées), avec une barre de défilement si la fenêtre est
petite (320 px de large doit rester utilisable). Chaque entrée garde son icône au matériau et son rendement.
`ctl` : `shape <forme>` accepte les nouvelles clés.

## 5. Tests et vérifications

- Moteur : occupation et boîtes de chaque nouvelle pièce (et de chaque orientation, au moins par symétrie) ;
  fusion et refus ; tranches qui grandissent sans perte ni création d'objets ; format v1 relu en v2 ;
  `turned` pour chaque pièce ; pentes en coin (hauteurs aux coins, collision montable) ; raccords d'un
  muret selon ses voisins (0, 1, 2 en ligne, 2 en coin, 4) ; portillon ouvert sans collision.
- Maillage : nombre de faces d'un escalier, d'une pente en coin (triangles), d'une vitre seule et raccordée ;
  faces cachées entre deux murets voisins.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` (zéro avertissement), `cargo test --workspace`,
  `cargo xtask check-arch` (aucun fichier au-delà de 2 500 lignes : `shaped.rs` fait déjà ~1 750 lignes,
  **le découper** en modules, par exemple `shaped/{mod,piece,slope,connected,format,place}.rs`).
- En jeu (`VOXEL_WORLD=formestest`) : un escalier de chêne, un muret de pierre qui se raccorde en L, une
  barrière avec un portillon qu'on ouvre, une vitre de verre, une pente murale, une pente en coin montée
  jusqu'à 16, une tranche qui grandit ; compléter la capture « formes » (`shapes_scene.rs`).
- Docs : `docs/moteur/03-monde.md` et `05-rendu.md` (sections des formes), la page du guide.
