# Plan : les formes de blocs (couches, dalles, pentes, microblocs)

Inspirations : **Create: Copycats+** (blocs « copycat » qui prennent l'apparence de n'importe quel bloc ;
couches empilables jusqu'au bloc plein ; *bytes* de 1/8 dont 8 font un bloc ; l'extension *Copycats+
Additions* ajoute des « slope layers » empilables de 1 à 8), **Carpenter's Blocks** (un cadre qui prend
la matière qu'on lui donne ; pentes, dalles, escaliers ; marteau pour tourner) et **Chisel** (un outil
où l'on met un bloc et où l'on choisit l'apparence de sortie). Aucun code ni donnée de ces mods n'est
repris : seulement les idées.

## Les décisions (validées le 20 nov. 2026)

| Question | Choix |
|---|---|
| D'où vient l'apparence ? | **Comme Copycat** : quelques blocs de forme génériques ; le matériau (n'importe quel bloc plein, du jeu ou d'un mod) est rangé **dans** la case. Aucun bloc à déclarer par matériau. |
| Finesse | **1/16** : couches de 1 à 16 seizièmes, pentes à 16 niveaux, microblocs de 1/8 (2 × 2 × 2) et de 1/64 (4 × 4 × 4). |
| Fabrication | **Une table seule** : l'*établi de taille*. On y met un bloc, on choisit la forme dans un écran, on récupère les pièces. Les pièces posées s'empilent à la main. |
| Plusieurs matériaux par case | **Dès maintenant** : chaque couche, pente ou microbloc garde son matériau (une dalle de chêne sur une dalle de pierre). |

## Vue d'ensemble

```
bloc plein (chêne) ──► établi de taille ──► pièces (objets) ──► posées dans le monde
                         (écran : forme)      « Dalle de chêne »     une case « core:shaped »
                                              « Pente de pierre »    qui contient une liste de
                                              ...                    pièces, chacune son matériau
```

- **Un seul bloc** du registre, `core:shaped` : une case de formes. Son contenu est dans son entité de
  bloc (`BlockEntity::data`), pas dans des variantes : aucune explosion du nombre de blocs.
- **Des objets de pièces** : un objet par sorte de pièce (couche, dalle, pente...), le matériau étant
  désigné par la pile (voir « Les objets »).
- **Le rendu, la collision, la visée et la lumière** viennent du contenu de la case.

## 1. Le modèle de données (moteur, `crates/engine_voxel/src/shaped.rs`)

### Les pièces

```rust
pub enum Piece {
    /// Une couche posée sur une face (6 directions), épaisse de 1 à 16 seizièmes.
    /// 8 = une dalle, 16 = un bloc plein. Elle grandit vers l'intérieur de la case.
    Layer { face: Face, thickness: u8 },
    /// Une pente : un plan incliné qui monte de 0 (bord bas) à `level` seizièmes (bord haut)
    /// sur toute la case. `facing` : le côté haut (4 directions) ; `ceiling` : retournée
    /// (accrochée au plafond). level 16 = la pente pleine (un demi-bloc en volume, 45°).
    Slope { facing: Facing4, ceiling: bool, level: u8 },
    /// Des microblocs sur une grille 4 × 4 × 4 (1/64 chacun) : un bit par cellule.
    /// Un microbloc de 1/8 occupe 2 × 2 × 2 cellules alignées.
    Micro { cells: u64 },
}
pub struct Part { pub piece: Piece, pub material: u8 }   // indice dans `materials`
pub struct Shaped { pub materials: Vec<String>, pub parts: Vec<Part> }
```

- **Occupation** : `occupancy() -> [u16; 256]` (16³ bits, 512 octets) calculée à la demande. Une pente
  occupe, colonne par colonne, les seizièmes sous son plan. Sert à refuser un recouvrement, à savoir si
  la case est pleine, et à cacher les faces intérieures au maillage.
- **Ajouter** (`try_add(part) -> Result`) : refuse ce qui chevauche. Fusionne quand c'est le même
  matériau : une couche sur la même face s'épaissit ; une pente de même orientation monte de niveau ;
  des microblocs se réunissent dans le même `Micro`. Plusieurs matériaux : des pièces séparées.
- **Retirer** (`remove_at(point, normal) -> Option<Part>`) : la pièce que vise le rayon (voir 3).
  Une couche perd un seizième ? Non : on retire **la pièce entière visée** et elle tombe en objets
  (une couche de 5 seizièmes rend 5 objets « couche », voir 4).
- **Simplifier** : une case vide redevient de l'air ; une case **pleine d'un seul matériau** redevient le
  vrai bloc de ce matériau (16 couches de chêne = un bloc de chêne).
- **Format** (`encode` / `decode`) : un octet de version, la liste des matériaux en **identifiants texte**
  (la case reste lisible quand la liste des mods change ; un matériau inconnu s'affiche en
  `core:unknown` et garde son nom), puis les pièces. Au plus 16 matériaux et 32 pièces par case.
- **Matériau valide** (`is_shapeable(def)`) : un cube (`Shape::Cube`), solide ou transparent (verre),
  pas liquide, sans comportement ni entité de bloc, placeable. Bois, pierres, verre, laine, minerais,
  blocs de mods : oui. Coffre, four, herbe haute, eau : non.

### Les boîtes

`Shaped::boxes() -> Vec<BlockBox>` pour la collision et la visée : couches et microblocs en boîtes
(fusionnées), pentes en **escalier de seizièmes** (au plus 16 boîtes, fusionnées par rangée). La marche
automatique du joueur (0,6) permet de monter une pente en marchant.

Tests (dans le module) : ajout et fusion, recouvrement refusé, empilement jusqu'au plein puis
simplification, format aller-retour (et un matériau inconnu), occupation d'une pente à chaque niveau,
boîtes d'une pente.

## 2. Le bloc `core:shaped` (registre, monde, physique, visée)

- `blocks.ron` : `core:shaped`, `shape: Model` avec une boîte pleine par défaut (l'objet ne se pose pas
  tel quel : `placeable: false`), `transparent: true`, `light_opacity` 0, `behavior: "core:shaped"`.
  Dureté et outil : ceux du matériau majoritaire (le jeu le lit à la casse).
- **Boîtes dynamiques** : `connections::boxes_at` (utilisé par `World::block_boxes`, donc la physique et
  le lancer de rayon) reçoit aussi l'entité de bloc ; pour `core:shaped`, les boîtes viennent de
  `Shaped::decode(data).boxes()`. Garder un cache si le décodage se révèle cher (mesurer).
- **Lumière** : la case ne bloque pas la lumière (comme un modèle) ; elle bloque celle du ciel si elle est
  pleine (cas qui se simplifie de toute façon en vrai bloc).
- **Grilles mobiles** : une case de formes est un bloc avec entité : elle voyage avec un vaisseau
  (`turned` doit tourner les pièces : faces, orientations, bits des microblocs).

## 3. Le maillage (`mesh.rs`, nouveau `mesh/shaped.rs`)

Pour une case `core:shaped`, le mailleur lit son entité (le `Neighbourhood` a déjà accès aux entités
des chunks) et produit, pièce par pièce, des faces avec **les textures du matériau** (`face_texture` du
matériau selon l'axe, sa teinte de biome pour l'herbe et les feuilles, la découpe pour le verre) :

- couches et microblocs : des boîtes, comme `model_boxes` ;
- pentes : la face inclinée (un quad dont la normale est inclinée ; le shader choisit déjà ses UV selon
  l'axe dominant), deux triangles de côté (des quads dégénérés, un sommet doublé), le dos et le dessous ;
- **faces cachées** : une face contre une autre pièce de la case (occupation 16³) ou contre un voisin
  opaque est omise ;
- lumière : celle de la cellule d'air voisine, sans occlusion ambiante fine au début (comme les modèles).

Le maillage binaire passe les cases `core:shaped` à cette routine (comme les autres modèles).
Le shader n'a pas besoin de changer (vérifier les UV de la face inclinée et des triangles).

## 4. Les objets et les matériaux

- **Six objets de pièces** (`items.ron`, catégorie construction) :

  | Objet | Pièce posée | Rendement par bloc |
  |---|---|---|
  | `core:shape_layer` — Couche | couche de 1/16 | 16 |
  | `core:shape_slab` — Dalle | couche de 8/16 | 2 |
  | `core:shape_slope_layer` — Couche de pente | pente +1 niveau | 16 |
  | `core:shape_slope` — Pente | pente de niveau 16 | 2 |
  | `core:shape_micro` — Microbloc | cube de 1/8 (2 × 2 × 2) | 8 |
  | `core:shape_nano` — Grain | cube de 1/64 | 64 |

- **Le matériau dans la pile** : `damage` = numéro du matériau dans la **palette de matériaux du monde**
  (`materials.ron` à la racine du monde : la liste des identifiants texte, un numéro attribué au premier
  usage, jamais réutilisé). Deux piles de même pièce et même matériau s'empilent donc normalement.
  Le bit 31 (fiches d'objets) et le marqueur d'affixes ne sont jamais utilisés par ces objets.
- **Nom et icône** : « Dalle de chêne », « Pente de pierre » ; l'icône est la miniature de la forme avec
  les textures du matériau (`item_art.rs`), mise en cache par (pièce, matériau).
- **Retour en arrière** : à l'établi, 16 couches (ou 2 dalles, 8 microblocs...) d'un même matériau
  redonnent le bloc plein.

## 5. L'établi de taille (`core:shaping_table`)

- Un bloc à modèle (dans l'esprit de l'établi), avec une recette (`recipes.ron` : planches et lingot de
  fer, par exemple).
- **L'écran** (`games/voxel/src/shaping_ui.rs`) : une case d'entrée (le bloc à tailler, n'importe quel
  matériau valide ; un bloc refusé affiche pourquoi), puis la grille des **six formes** dessinées avec le
  matériau, chacune avec son rendement ; un clic donne les pièces dans l'inventaire et consomme un bloc,
  Maj + clic taille toute la pile. Une seconde rangée « Reconstituer » propose le bloc plein quand
  l'entrée contient assez de pièces d'un même matériau.
- `ctl` : `shape <forme> [n]` pour tailler sans souris (essais et captures).

## 6. Poser, empiler, casser (jeu)

- **Poser une pièce** (clic droit avec un objet de pièce) :
  - sur la face d'un bloc ordinaire : une case `core:shaped` naît dans la case voisine, avec la pièce
    orientée selon la face visée (couche collée à cette face) ou le regard (pente : le côté haut face au
    joueur ; retournée si l'on vise le dessous d'un bloc ou le haut d'une face) ;
  - **sur une case de formes** : la pièce s'y ajoute si elle y tient (couche qui s'épaissit, pente qui
    monte, microbloc dans la cellule 2 × 2 × 2 (ou 4 × 4 × 4) visée ou voisine du point visé) ; sinon dans
    la case suivante.
- **Casser** : le clic gauche retire **la pièce visée** (le rayon touche une de ses boîtes) et la rend en
  objets ; la case se simplifie. Le temps de minage est celui du matériau, au prorata du volume.
- **Fiche de visée** : « Case de formes : chêne, pierre » et la pièce visée.

## 7. Le reste

- **Sauvegarde** : rien de neuf dans les chunks (l'entité de bloc est déjà sauvegardée) ;
  `materials.ron` à la racine du monde.
- **Terrain lointain** : une case de formes compte comme un sol si elle est pleine à plus de moitié, avec
  la couleur de son matériau principal (`edited_surface.rs`).
- **Mods** : un mod peut lire et écrire une case de formes par les fonctions de bloc existantes ; rien de
  plus dans ce lot. `docs/modding.md` explique ce qui est taillable.
- **Guide et quêtes** : une page « Tailler les blocs » dans `guide.ron` ; une quête facultative.
- **Documentation** : `docs/moteur/03-monde.md` (le bloc et son format), `05-rendu.md` (les pentes),
  `06-simulation.md` si besoin.

## Lots

1. **Modèle de données** (`shaped.rs`) et ses tests.
2. **Bloc, boîtes dynamiques, physique, visée** (+ rotation sur les grilles mobiles), tests du moteur.
3. **Maillage** des pièces, pentes comprises, faces cachées ; tests de maillage (nombre de faces d'une
   dalle, d'une pente, d'une case pleine de deux matériaux).
4. **Objets, palette de matériaux, noms, icônes**.
5. **Établi de taille** et son écran, commandes `ctl`.
6. **Pose, empilement, casse, fiche de visée**.
7. **Guide, terrain lointain, docs, captures** (une scène de formes dans `VOXEL_SCREENSHOTS`).

## Vérifications

`cargo fmt --all`, `cargo clippy --workspace --all-targets`, `cargo test --workspace`,
`cargo xtask check-arch` (aucun fichier au-delà de 2 500 lignes : `main.rs` est à 2 495, ne rien y
ajouter ; mettre le code du jeu dans de nouveaux modules), puis un essai en jeu dans un monde de test
(`VOXEL_WORLD=formestest`) : tailler du chêne et de la pierre, poser une dalle de chêne sur une dalle de
pierre, monter une pente couche par couche jusqu'au niveau 16, poser des microblocs, casser pièce par
pièce, sauvegarder, relancer.
