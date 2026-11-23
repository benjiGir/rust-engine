# Écrire un mod

Un mod ajoute au jeu des **données** (blocs, fluides, créatures, textures, sons) et, s'il le
veut, un **script** : du code Rust compilé en WebAssembly, qui réagit aux événements du jeu et
fait fonctionner des machines. Le mod d'exemple (`mods/example`) montre tout ce qui suit.

## Démarrer vite

```sh
cargo xtask new-mod mon_mod     # le squelette : manifeste, un bloc, un script, un test
cargo xtask test-mod mon_mod    # compile le script, puis lance ses tests sans fenêtre
cargo xtask build-mods          # recompile les scripts (le jeu les recharge à chaud)
cargo xtask docs                # la référence du SDK (chaque fonction, type et méthode), en HTML
```

`new-mod` crée `mods/mon_mod/` (manifeste, `blocks.ron`, `script/` avec un test) et l'ajoute au
workspace. La **référence du SDK** (`cargo xtask docs`, puis `target/doc/mod_sdk/index.html`) documente
chaque fonction, chaque type et chaque méthode du trait `Mod` ; ce guide-ci explique comment les
assembler. Le **rechargement à chaud** couvre `blocks.ron`, les textures, les sons et les scripts ; les
autres fichiers (objets, recettes, créatures, structures, minerais, `mod.ron`) demandent de relancer
le jeu, qui le dit dans ses logs quand on les modifie. Les **tests sans fenêtre** utilisent `engine_mods::testing::Harness` : le vrai script
compilé, dans un petit monde reproductible, avec les blocs et objets du jeu et des mods.

```rust
let mut h = Harness::for_mod("mon_mod")?;
h.place(IVec3::new(0, 100, 0), "mon_mod:machine");
let (pris, demandes) = h.use_block(IVec3::new(0, 100, 0))?;   // un clic droit
assert!(pris);
// h.event(&ModEvent::...), h.machines(MACHINES_TICK, &[pos]), h.ui(pos, UI_BUTTON, id, 0),
// h.block(pos) pour lire le monde, h.next_tick() pour rendre son budget au mod.
```

## Le dossier d'un mod

```
mods/example/
├── mod.ron            # le manifeste (obligatoire)
├── blocks.ron         # ses blocs (facultatif)
├── fluids.ron         # ses fluides (facultatif)
├── entities.ron       # ses créatures, avec ou sans `brain: true` (facultatif)
├── ores.ron, biomes.ron, structures.ron   # le monde généré (facultatifs)
├── dimensions.ron     # ses dimensions et leurs portails (facultatif)
├── items.ron          # ses objets (facultatif)
├── machine_recipes.ron # les recettes de ses machines (facultatif)
├── rarities.ron, affixes.ron, gems.ron, tiers.ron, elites.ron   # l'aventure : raretés, affixes, gemmes, paliers, élites (facultatifs)
├── guide.ron, quests.ron                                       # le Guide du voyageur et le livre de quêtes : ses chapitres (facultatifs)
├── enchantments.ron, charms.ron                                # l'enchantement avancé : enchantements et charmes de potion (facultatifs)
├── textures/          # PNG 16×16, nommés comme dans blocks.ron
├── sounds/            # WAV (ou OGG Vorbis) ; variantes nom_1.wav, nom_2.wav…
├── script/            # les sources du script (une crate Rust)
└── example.wasm       # le script compilé (cargo xtask build-mods)
```

Le nom du dossier est l'**identifiant** du mod et son **espace de noms** : tous ses blocs
s'appellent `example:...`. Deux mods peuvent avoir chacun un bloc `ruby` sans conflit.

## Le manifeste : `mod.ron`

```ron
(
    id: "example",                 // = le nom du dossier
    name: "Mod d'exemple",
    version: "1.0.0",              // versionnage sémantique : MAJEUR.MINEUR.CORRECTIF
    authors: ["benji"],
    description: "Un casseur de blocs et des rubis.",
    dependencies: [
        (id: "core", version: "^0.1"),   // le jeu de base est un mod comme les autres
    ],
    script: "example.wasm",        // facultatif
    options: true,                 // facultatif : une page d'options (voir plus bas)
)
```

Une exigence de version se lit comme dans Cargo : `^1.2` accepte 1.2.0 jusqu'à 2.0 exclu,
`>=0.2, <0.4` un intervalle, `*` n'importe laquelle.

### L'ordre de chargement

Les mods sont chargés **chacun après ses dépendances** (un tri topologique), `core` en premier,
et à égalité par ordre alphabétique : le même ordre sur toutes les machines. Il compte, car il
fixe les numéros des blocs.

Le jeu refuse de charger les mods, avec un message clair dans les logs, si :
- une dépendance n'est pas installée (« le mod « factory » a besoin de « energy » (^2.0), qui
  n'est pas installé ») ;
- sa version ne convient pas (« ... en version ^2.0, mais la version installée est 1.9.0 ») ;
- des mods dépendent les uns des autres en cercle (« dépendances circulaires entre les mods : a, b »).

Le jeu démarre alors sans les mods plutôt que de ne pas démarrer.

## Les formes et les états de bloc

Par défaut un bloc est un cube. Avec `boxes`, il devient une ou plusieurs **boîtes**, en
seizièmes de bloc (0 à 16 sur chaque axe) : dalles, escaliers, barrières, poteaux, engrenages.
La même forme sert au dessin, à la **collision** (le joueur marche sur la moitié d'une dalle
et passe sous une dalle haute) et à la **sélection** (le rayon passe à côté d'une dalle).
La texture est coupée, pas étirée : une dalle montre la moitié de sa texture.

Un cube avec `tint: Grass` peut déclarer `textures.side_tint` en plus de `top`, `side`
et `bottom`. C’est le nom d’un PNG dont l’alpha dose la teinte du biome sur les côtés.
Pour l’herbe, le masque est opaque sur la végétation et transparent sur la terre.

Le champ `textures` accepte `front` pour la façade nord et `back` pour le dos sud.
Les deux peuvent être omis et reprennent alors `side`. Avec `states: [Facing4]`, la
façade, le dos et le dessin du modèle suivent l’orientation de pose. Les variantes
ainsi ajoutées sont numérotées après les autres blocs au chargement du jeu.

```ron
(id: "example:ruby_stairs", name: "Escalier de rubis", color: (0.7, 0.05, 0.12), texture: "ruby",
 boxes: [(from: (0, 0, 0), to: (16, 8, 16)), (from: (0, 8, 0), to: (16, 16, 8))],
 states: [Facing4, Half]),
```

`states` fabrique **une variante par combinaison** (ici 4 × 2 = 8). Chaque variante est un bloc
du registre, avec son identifiant complet (`example:ruby_stairs[facing=east,half=top]`) ; la base
(`example:ruby_stairs`) est la première. La forme écrite est celle de la **première valeur** : le
jeu en déduit les autres par rotation. Au plus 64 variantes par bloc.

**Compatibilité des sauvegardes** : les variantes sont enregistrées immédiatement
après leur base. Ajouter des `states` à un bloc déjà publié décale donc les identifiants
numériques des blocs suivants, même sans changer l’ordre du fichier. Conserver les
états existants lors d’une simple passe visuelle. Les comportements qui échangent
directement deux identifiants (levier, bouton, répéteur, piston…) ne transfèrent pas
automatiquement une orientation à l’autre identifiant.

| État | Valeurs | À la pose |
|---|---|---|
| `Facing4` | `north` (-Z, la forme écrite), `east`, `south`, `west` | l'avant regarde le joueur |
| `Facing6` | les quatre ci-dessus, `up`, `down` | idem, et vers le haut ou le bas si le joueur regarde fort dessus |
| `Half` | `bottom` (la forme écrite), `top` | en haut si on vise le dessous d'un bloc, ou la moitié haute d'une face |
| `Axis` | `y` (la forme écrite), `x`, `z` | l'axe de la face visée |
| `DoorOpen` | `false` (fermé), `true` (ouvert) | fermé |

`[DoorOpen, Facing4]` pivote les boîtes de trois seizièmes d'épaisseur autour du montant
nord-ouest avant de les orienter. La forme fermée doit rester entre `z = 0` et `z = 3`.
Le lien entre deux moitiés et l'ouverture au clic sont gérés par le jeu pour les portes de base.
`glazing` désigne l'indice d'une boîte vitrée, compté depuis zéro. Cette boîte reste
sélectionnable et participe à la collision du modèle. Son rendu est translucide.

Poser et casser : seule la base est dans la barre de blocs et l'inventaire ; la variante est
choisie à la pose, et une variante cassée lâche l'objet (et le butin) de sa base. Un mod qui a
besoin d'un bloc précis le retrouve par son identifiant complet (`block_id("example:ruby_stairs[facing=east,half=top]")`),
et `block_base(bloc)` ramène une variante à sa base (par exemple dans l'événement `BlockPlaced`).

**Conduits et réseaux** : `conduit:` fait d'un bloc un conduit de réseau ; le jeu relie ceux qui se
touchent et répartit le contenu entre les machines branchées (celles qui ont la capacité
correspondante, par leurs faces ouvertes). Un conduit n'a pas de `behavior` ; ses variantes sont des
conduits aussi. Quatre sortes :

| `conduit:` | Ce qui circule | Les machines branchées |
|---|---|---|
| `Energy` | de l'énergie | un stock d'énergie (`add_energy`) |
| `Fluid` | un fluide (un par réseau) | un réservoir (`add_tank`) |
| `Kinetic` | de la rotation | une rotation (`add_kinetic`) : sources et consommateurs |
| `Items` | des objets | un inventaire (`add_inventory`) |

Trois réglages, tous facultatifs, pour fabriquer **ses propres types de réseau** :
- `channel: "vapeur"` — le **canal** : deux conduits de même sorte ne se relient que s'ils ont le même
  canal. Des tuyaux de vapeur et des tuyaux d'eau (tous deux `Fluid`) forment ainsi deux réseaux
  séparés, même côte à côte. Le canal est à l'espace de noms du mod (`mon_mod:vapeur`).
- `throughput: 4` — le **débit** : le plus lent des conduits d'un réseau le plafonne. Des courroies
  de bois et d'acier sont deux blocs de même sorte, l'un avec un débit plus faible.
- `ports: (inputs: [0], outputs: [1], accepts: "presse")` — pour les réseaux d'**objets**, sur le bloc
  d'une machine : les cases que le réseau remplit (`inputs`), celles qu'il vide (`outputs`), et le type de
  recette (`machine_recipes.ron`) dont elle n'accepte que les ingrédients. Sans `ports`, toutes les
  cases d'un inventaire font les deux (un coffre).
- `ports: (inputs: [1, 2, 3], filter: 0)` — un **filtre** : la case 0 est un modèle ; l'objet qu'elle
  contient est le seul à entrer par les cases d'entrée (case vide : tout entre). Le réseau ne la vide
  jamais. C'est un trieur : le joueur y dépose un exemplaire de l'objet voulu.

- `flow: true` — un conduit **à sens unique** (`conduit: Items`, `Energy` ou `Fluid`, avec un état
  `Facing4` ou `Facing6`) : le contenu ne va que vers l'« avant » de la variante (le nord de la forme
  écrite), et posé il **pointe là où regarde le joueur**. Il prend par l'arrière et par les côtés (une
  machine ou un autre conduit peut le rejoindre de côté), et ne rend que par l'avant : une machine sur
  son flanc ne reçoit rien. Deux sens uniques face à face se bloquent. Il se relie aux conduits du même
  canal (un tronçon peut être à sens unique, le reste libre) : un convoyeur à objets, une **diode**
  (un générateur ou une batterie derrière elle alimente ce qui est devant, jamais l'inverse), un
  **clapet anti-retour** (un fluide). Dessinez la flèche avec les boîtes (elles tournent avec
  l'orientation ; une texture ne tourne pas). Refusé pour la rotation (`Kinetic`) : un arbre
  transmet dans les deux sens. Quand un réseau a des sens uniques, chaque producteur ne sert que les
  machines qu'il atteint, dans l'ordre (pas de partage au prorata entre producteurs).

**Les réseaux d'objets** passent 5 fois par seconde et déplacent au plus 8 objets par passage (ou le
`throughput`), chacun son tour. Dans l'ordre : des producteurs (sorties d'une machine, inventaires
« sortie seule ») vers les consommateurs (entrées, inventaires « entrée seule »), puis le surplus des
producteurs vers les **tampons** (les coffres, qui reçoivent et rendent), puis des tampons vers les
consommateurs. Deux coffres côte à côte ne se renvoient donc pas les objets. Le calcul est natif,
sans passage par le script.

À savoir : un bloc à boîtes est « transparent » pour ses voisins (ils gardent leurs faces) ; sa
lumière est arrêtée par défaut s'il est `solid`, et passe sinon (`light_opacity` pour régler).
Les créatures le traitent encore comme un cube plein.

## Les blocs taillables

Le jeu sait **tailler** les cubes en pièces (couches, dalles, pentes, microblocs) à l'établi de taille : aucun bloc
à déclarer, un mod n'a rien à faire. Un bloc se taille s'il est :

- un **cube** (`shape: Cube`, donc sans `boxes`, ni `rail`, ni `connections`), solide ou transparent (le verre), pas
  un liquide ;
- **sans comportement** (`behavior`) ni état (`states`) : une machine, un coffre, l'herbe, les feuilles ne se
  taillent pas ;
- **posable** (`placeable`, c'est-à-dire avec son objet).

Les pièces taillées gardent l'**identifiant texte** de leur matériau (`example:ruby`) : si le mod est retiré, la case
reste lisible et se dessine avec le bloc inconnu jusqu'à son retour. Un mod peut lire et écrire une case de formes avec
les fonctions de bloc existantes (`BlockEntity::data`, voir `docs/moteur/03-monde.md`) ; rien de plus n'est exposé
pour l'instant.

## Les raccords automatiques des conduits

Un conduit peut calculer sa forme à partir de ses voisins, sans `states` ni nouveaux
identifiants de blocs :

```ron
(
    id: "mon_mod:cable", name: "Câble", color: (0.2, 0.1, 0.05), texture: "cable",
    conduit: Energy,
    connections: (core: 4, arm: 4),
)
```

`core` est la largeur du noyau central, `arm` celle des branches, en seizièmes de bloc :
`0 < arm <= core < 16`, avec des valeurs finies. Ce champ remplace `boxes` et ne se
combine pas avec `states` ou `rotation`. Il exige un conduit (champ `conduit`, ou
comportement natif `core:cable`, `core:pipe`, `core:shaft`).

La forme suit les six voisins : même sorte et même canal pour les conduits, capacité
compatible et face ouverte pour les machines. Une pierre ou un conduit d’une autre
sorte ne crée pas de branche. Les machines en retrait reçoivent un raccord jusqu’à
la première pièce qui croise sa section. Les calculs utilisent les capacités réelles,
y compris les faces des réservoirs multiples ; une machine vide reste raccordable.
Le dessin, les collisions, le rayon et le contour de sélection suivent ces formes.
L’icône et l’objet lâché représentent un segment droit.

Les 64 combinaisons sont des géométries calculées, pas des variantes enregistrées :
la pose, la casse et le chargement des voisins les mettent à jour. Les numéros des
blocs et le format des sauvegardes restent inchangés. Les écritures via `Grids::write`
et `write_or_create` invalident le rendu quand les capacités ou leurs faces changent ;
changer uniquement les quantités ne remaille pas le monde. Une écriture directe
dans un chunk doit, comme tout changement visuel direct, faire invalider le rendu.

Dans le jeu, **Maj + clic droit** avec un conduit permet de le poser contre une
machine sans ouvrir son interface. Le clic normal sur un conduit pose sur la face
visée ; les branches se raccordent automatiquement.

### Le contenu visible des conduits

Les transferts `Items` et `Fluid` produisent automatiquement des traces de dessin,
dans le monde et dans les structures mobiles. Les objets utilisent leurs modèles
texturés. Les fluides utilisent la couleur et la lumière propre de `fluids.ron` ; une
densité négative donne une brume gazeuse. Aucun champ ni appel de script supplémentaire
n'est nécessaire. Les courroies basses montrent les objets au-dessus du tapis.

Un modèle fermé de fluide, ou un transporteur d'objets fermé à `connections`/`boxes`,
reçoit un cadre métallique et des fenêtres transparentes pour voir son contenu.
Les boîtes physiques et la sélection gardent leur géométrie d'origine. Pour un
transporteur d'objets en boîtes, le premier volume haut d'au moins un demi-bloc
est considéré comme une enveloppe fermée ; un tapis bas reste ouvert.

Le trajet de dessin suit les conduits compatibles, les faces des machines, le canal
et les sens uniques. Il ne traverse ni une machine intermédiaire ni un autre réseau.
Les réserves changent toujours au tick du transfert : le déplacement visible n'ajoute
aucun délai de livraison et n'est pas sauvegardé. Un flux arrêté s'efface rapidement ;
le réseau ne gagne pas de réserve de fluide dans ses tuyaux.

Le dessin est borné : 256 traces en attente, 256 animations actives, 8192 instances
par image à 64 blocs de la caméra. Une recherche visuelle parcourt au plus 8192
conduits et retient un trajet d'au plus 512 positions. Au-delà, seuls des effets
visuels sont omis ; les transferts continuent normalement.

## Les animations de blocs

Un modèle à boîtes peut déclarer une rotation visuelle continue, calculée par le GPU :

```ron
(
    id: "example:rotor", name: "Rotor", color: (0.6, 0.6, 0.65),
    boxes: [
        (from: (6, 0, 6), to: (10, 16, 10)),
        (from: (1, 6, 1), to: (15, 10, 15)),
    ],
    states: [Axis],
    rotation: (axis: Y, speed: 1.5, boxes: [1]),
)
```

`axis` vaut `X`, `Y` ou `Z` dans le modèle de base ; les états d'orientation tournent
aussi cet axe. `speed` est en radians par seconde, positif ou négatif, arrondi au
huitième, dans l'intervalle `-15.875..=15.875`. `boxes` choisit les indices des boîtes
animées (à partir de 0) ; absent ou vide, toutes tournent. Le pivot est le centre du
bloc `(8, 8, 8)` en seizièmes. Le pied du rotor ci-dessus reste donc fixe.

Les textures suivent la pièce et les limites de rendu couvrent toute son orbite.
Cette animation est décorative : elle tourne en continu, indépendamment de la vitesse
du réseau cinétique ; collision et sélection conservent les boîtes de repos. Elle ne
se combine pas avec `sway`. Les arbres et engrenages de Mécanique en donnent un exemple.

Un conteneur peut déclarer un couvercle avec `lid: (body: 0, boxes: [1, 2], hinge: (10, 15), inside: "chest_inside")`.
`body` désigne la boîte rendue creuse et `boxes` les pièces mobiles. `hinge` donne la
hauteur et la profondeur de la charnière en seizièmes dans le modèle orienté nord.
`inside` est la texture des parois internes. L’ouverture de son inventaire ou de son
menu de mod déclenche l’animation et les sons du coffre. La collision conserve les
boîtes fermées.

Avec `states: [Facing4, ChestPart]`, `lid.paired_textures` peut préciser un jeu de
textures par moitié. Les champs `left` et `right` décrivent chacun `top`, `front`,
`back` et `inside`. Gauche et droite se lisent face au coffre. Les flancs extérieurs
conservent le champ `side` du bloc.

Le four (`behavior: "core:furnace"`) a aussi un effet de flammes dans la bouche de
`furnace_front.png` : il s'affiche uniquement si `FurnaceState::burning()` vaut vrai.
Le shader l'anime à 10 images par seconde et rend les flammes lumineuses, sans ajouter
de lumière aux blocs voisins. L'allumage et l'extinction invalident le maillage du chunk
(ou de la structure mobile), sans remplacer le bloc ni perdre son inventaire. La
combustion sauvegardée détermine le même affichage après rechargement du monde.

La torche de base porte une flamme animée et des braises sur deux panneaux au-dessus
de sa tête carbonisée. Les fissures incandescentes du bois et la flamme s'animent
aussi en main et sur les objets tombés au sol.

## Les blocs actifs (« machines »)

Un bloc devient une machine quand il déclare un comportement :

```ron
(id: "example:breaker", name: "Casseur", color: (0.45, 0.45, 0.5), texture: "breaker",
 behavior: "breaker"),
```

`behavior: "breaker"` (soit `example:breaker`) : ce comportement est écrit dans le script du
mod `example`. Le jeu de base en a deux, natifs : `core:clock` (une horloge qui alterne sa
puissance) et `core:lamp` (une lampe qui s'allume quand un voisin l'alimente).

Comme dans Minecraft, une machine n'est pas parcourue à chaque image. Elle dort jusqu'à ce que :
- **son tick programmé** arrive (« réveille-moi dans 40 ticks ») ;
- **un voisin change** (un bloc posé ou cassé à côté, une puissance qui varie).

Les machines tournent à **20 ticks par seconde**. Une machine a un petit état sauvegardé avec
son chunk : une puissance (0 à 15), des octets libres, et des **capacités** (voir plus bas).

## Le script

Un script est une crate Rust de type `cdylib` qui dépend de `mod_sdk` :

```toml
# mods/<id>/script/Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
mod_sdk = { path = "../../../crates/mod_sdk" }
```

Ajoutez ce dossier aux `members` du `Cargo.toml` racine. Le code :

```rust
use mod_sdk::{Block, Event, EventKind, MachineCall, Mod, Pos};

#[derive(Default)]
struct MonMod {
    stone: Block,
}

impl Mod for MonMod {
    fn init(&mut self) {
        self.stone = mod_sdk::block_id("core:stone").unwrap_or_default();
        mod_sdk::subscribe(EventKind::BlockBroken);
    }

    fn event(&mut self, event: Event) {
        if let Event::BlockBroken { block, .. } = event
            && block == self.stone
        {
            mod_sdk::log("de la pierre !");
        }
    }

    fn machines(&mut self, call: MachineCall, positions: &[Pos]) {
        for &pos in positions {
            if call == MachineCall::ScheduledTick {
                mod_sdk::schedule_tick(pos, 20); // dans une seconde
            }
        }
    }
}

mod_sdk::export_mod!(MonMod);
```

Compiler tous les scripts :

```sh
cargo xtask build-mods
```

La commande compile chaque `mods/<id>/script` pour `wasm32-unknown-unknown` (la cible est
installée par `rust-toolchain.toml`) et copie le `.wasm` là où `mod.ron` l'attend.
**Le jeu recharge le script à chaud** : relancez `cargo xtask build-mods` pendant qu'il tourne,
le nouveau code prend le relais sans redémarrer. Un script invalide est refusé et l'ancien reste
en place.

### Le clic droit

`Mod::used(pos) -> bool` est appelé quand le joueur fait un clic droit sur une **machine du mod**
(un bloc dont le `behavior` est à lui). Renvoyer `true` prend le clic en compte (le jeu ne pose
pas de bloc contre elle) ; `false` le laisse passer. C'est là qu'une machine ouvre son menu, change
de mode ou donne un objet.

### Les menus

Une machine peut avoir un menu (son écran, ouvert par un clic droit). Le script le **décrit**,
le jeu le dessine : aucun dessin ne passe par le WebAssembly, et un menu ouvert ne coûte rien
tant que le joueur n'agit pas.

```rust
fn used(&mut self, pos: Pos) -> bool {
    let menu = Menu::new("Presse")
        .bar("Progression", 0.4)
        .row_begin().slots(0, 2, 2).slots(2, 1, 1).row_end()   // cases de l'inventaire de la machine
        .toggle(1, "Active", true)
        .button(2, "Vider");
    mod_sdk::show_menu(pos, &menu);
    true
}

fn ui(&mut self, pos: Pos, event: UiEvent) {
    match event {
        UiEvent::Button(2) => { /* vider */ }
        UiEvent::Toggle(1, on) => { /* ... */ }
        UiEvent::Refresh => {}   // 5 fois par seconde tant que le menu est ouvert
        _ => return,
    }
    mod_sdk::show_menu(pos, &menu_a_jour(pos));   // remettre le menu à jour
}
```

| Élément | Rôle |
|---|---|
| `label(texte)`, `separator()` | du texte, un trait |
| `bar(texte, 0..1)` | une jauge (progression, niveau) |
| `button(id, texte)` | `UiEvent::Button(id)` quand on clique |
| `toggle(id, texte, actif)` | une case à cocher : `UiEvent::Toggle(id, actif)` |
| `slider(id, texte, min, max, valeur)` | un curseur : `UiEvent::Slider(id, valeur)` |
| `slots(première, nombre, colonnes)` | des cases de l'inventaire de la machine (`add_inventory`) |
| `row_begin()` … `row_end()` | place les éléments côte à côte (pas d'imbrication) |

- **Ouverture** : un menu ne s'ouvre qu'en réponse à un clic droit (dans `Mod::used`). Une fois
  ouvert, le mod peut le remettre à jour (`show_menu` sur la même machine) ou le fermer
  (`close_menu`) ; un mod ne peut pas faire surgir un menu quand il veut.
- **Limites** : 16 Kio, 256 éléments et 64 cases par menu, textes de 255 octets. Un menu invalide est
  refusé avec la raison dans les logs, sans effet sur le jeu.
- **Cases** : le joueur y dépose et en retire des objets comme dans un coffre ; la machine est
  réveillée (`ScheduledTick`) à chaque changement.
- `UiEvent::Closed` arrive quand le joueur ferme le menu (Échap).

### Les pages d'options et les réglages

Un mod peut avoir une **page d'options** dans le menu pause (« Options : <nom du mod> »). Il le
dit dans son manifeste (`options: true`) et la décrit avec le même `Menu` que les machines :

```rust
fn options(&mut self) -> Option<Menu> {
    Some(Menu::new("Mon mod")
        .slider(1, "Délai (ticks)", 5.0, 200.0, mod_sdk::setting_i32("delay", 40) as f32)
        .toggle(2, "Effets", mod_sdk::setting_bool("fx", true)))
}

fn options_event(&mut self, event: UiEvent) {
    match event {
        UiEvent::Slider(1, v) => { mod_sdk::set_setting("delay", &(v.round() as i32).to_string()); }
        UiEvent::Toggle(2, on) => { mod_sdk::set_setting("fx", &on.to_string()); }
        _ => {}
    }
}
```

`options()` est appelée à l'ouverture de la page puis après chaque action du joueur ; elle redonne
le menu à jour (les cases d'inventaire n'y sont pas disponibles). `UiEvent::Closed` arrive au retour.

Les **réglages** sont des paires clé → texte propres à chaque mod, écrites dans `mod_settings.ron`
(à la racine du projet, hors des mondes : valables partout, comme les options du jeu). Les lire :
`setting(clé)`, ou `setting_i32`, `setting_f32`, `setting_bool` avec une valeur par défaut ; les écrire :
`set_setting(clé, valeur)` (clé de 64 octets, valeur de 1 024, 256 clés par mod). Un script les relit
quand il veut : un réglage changé dans la page d'options agit aussitôt. Le fichier est écrit au plus
une fois par seconde, et au retour de la page.

La page n'est disponible qu'en jeu (le script d'un mod vit avec le monde), pas depuis le menu principal.

### Les données du monde

À côté des réglages du joueur (valables partout), un mod range ses données **propres à un monde** :
`world_value(clé)` et `set_world_value(clé, valeur)` (du texte : nombres, listes, JSON...). Elles
suivent la sauvegarde du monde (`saves/<monde>/mod_data.ron`, écrit avec le monde et à la
fermeture) : un compteur de parties, une progression, un registre de machines. Limites : clé de
64 octets, valeur de 64 Kio, 1 024 clés par mod. Dans un monde sans sauvegarde, elles ne durent
que la partie.

### Les appels entre mods

Un mod peut offrir des fonctions aux autres, et en appeler :

```rust
// Dans le mod qui offre (mecanique) :
fn call(&mut self, name: &str, _args: &[u8]) -> Option<Vec<u8>> {
    match name {
        "crank_speed" => Some(crank_speed().to_le_bytes().to_vec()),
        _ => None,   // « je ne sais pas faire ça »
    }
}

// Dans un autre mod (qui déclare `mecanique` dans ses dépendances) :
if mod_sdk::mod_loaded("mecanique")
    && let Some(bytes) = mod_sdk::call_mod("mecanique", "crank_speed", &[])
{ /* ... */ }
```

Arguments et réponse sont des octets (64 Kio au plus) : à chacun de choisir son format. L'appel est
synchrone, dans le même tick. `call_mod` renvoie `None` si le mod est absent, désactivé, hors
budget, ne répond pas, ou si l'appel ferait un **cycle** (A appelle B qui rappelle A : refusé, jamais
bloqué). Le temps passé dans le mod appelé compte dans **son** budget ; ce qu'il demande au jeu
(donner un objet, poser un bloc) est fait comme si l'appelant l'avait demandé.

### Les commandes

Le joueur ouvre la **console** avec Entrée (ou `/`), tape `/commande arguments`, et la réponse
s'affiche au-dessus de la barre d'objets (Échap ferme la console, les flèches rappellent les
commandes déjà tapées). `/help` liste tout, `/mods` montre ce que consomme chaque mod. Le jeu a ses
commandes (`/give`, `/time`, `/weather`, `/summon`, `/tp`, `/xp`) ; un mod ajoute les siennes :

```rust
fn init(&mut self) {
    mod_sdk::register_command("rubis", "[nombre] : se donner des rubis");   // dans `init`
}

fn command(&mut self, name: &str, args: &str) -> Option<String> {
    match name {
        "rubis" => {
            mod_sdk::give_stack(self.ruby, args.trim().parse().unwrap_or(1));
            Some("Voilà.".to_string())          // la réponse : une ligne par `\n`
        }
        _ => None,                              // rien à dire
    }
}
```

Le nom : minuscules, chiffres et `_`, 32 caractères au plus, 32 commandes par mod. Si deux mods
déclarent la même, `/mod:nom` désigne la bonne. Une commande est une action du joueur : elle ne
dépend pas du budget par tick, et ce qu'elle demande au jeu (donner un objet, poser un bloc) est
fait aussitôt. `cargo xtask ctl console /commande` la lance sans clavier (pour les tests),
`h.command(nom, args)` du banc d'essai la teste sans fenêtre.

### Les événements

`mod_sdk::subscribe(EventKind::...)` dans `init`, puis `Mod::event` les reçoit :

<!-- hooks:events -->
| Événement | Quand | Données (interface binaire) |
|---|---|---|
| `BlockBroken` (1) | un bloc a été cassé | `grille, x, y, z, bloc` (i32) |
| `BlockPlaced` (2) | un bloc a été posé | `grille, x, y, z, bloc, dx, dy, dz` (i32 ; `d` : direction horizontale du regard du joueur) |
| `EntityDied` (3) | une créature est morte | `x, y, z` (f32), puis le type de créature en UTF-8 |
| `Tick` (4) | un tick de machines (20 par seconde) | le numéro du tick (u64) |
| `PlayerDamaged` (5) | le joueur a perdu de la vie | `dégâts, vie restante` (f32) |
| `ItemUsed` (7) | le joueur a fait un clic droit avec un objet `usable` en main (hors d'une action du jeu : poser un bloc, ouvrir un coffre...) | `objet` (i32 : son numéro, voir `item_id`) |
| `QuestCompleted` (8) | une quête vient de se terminer (une seule fois par quête et par monde, ses récompenses restent à récupérer) | l'identifiant de la quête (`mod:chapitre/quête`) en UTF-8 |
| `ChunkGenerated` (6) | un chunk non vide vient d'être généré pour la première fois (pas relu d'une sauvegarde) : le mod peut le décorer | `grille, cx, cy, cz` (i32 ; coordonnées de chunk, 32 blocs de côté) |
<!-- /hooks:events -->

### Les appels de machines

`Mod::machines(call, positions)` reçoit **toutes les machines du mod à réveiller d'un coup** :

<!-- hooks:machines -->
| `MachineCall` | Quand |
|---|---|
| `Placed` (0) | elles viennent d'être posées |
| `Loaded` (1) | leur chunk vient d'être chargé (les ticks programmés ne sont pas sauvegardés) |
| `NeighborChanged` (2) | un voisin a changé |
| `ScheduledTick` (3) | leur tick programmé est arrivé |
<!-- /hooks:machines -->

Pourquoi par lot : chaque passage du jeu au script coûte un temps fixe (voir les mesures
ci-dessous). 10 000 machines en un appel coûtent bien moins que 10 000 appels.

### Les fonctions du jeu

| Fonction | Rôle |
|---|---|
| `log(texte)` | écrit dans le journal du jeu |
| `block_id("core:stone") -> Option<Block>` | le numéro d'un bloc (il dépend des mods installés) |
| `get_block(pos)`, `set_block(pos, bloc)` | lire, poser (les voisins sont prévenus ; le contenu d'un coffre ou d'une machine remplacé tombe au sol) |
| `power(pos)`, `received_power(pos)`, `set_power(pos, 0..15)` | la puissance, comme la redstone |
| `data(pos)`, `set_data(pos, octets)` | l'état libre du bloc, sauvegardé avec le monde |
| `schedule_tick(pos, délai)` | se faire réveiller dans `délai` ticks |
| `give_item(bloc, nombre)` | donner des objets au joueur |
| `spawn_entity("core:pig", x, y, z)` | faire apparaître une créature |
| `random()` | un nombre pseudo-aléatoire |

### Objets et recettes de machine

Un script peut donner au joueur n'importe quel **objet**, pas seulement un bloc :
`item_id("core:ruby")` donne son numéro, `give_stack(objet, nombre)` le donne.

Un mod déclare les recettes de ses machines dans `machine_recipes.ron`, par **type** de
recette (un type est à lui, `press` devient `example:press` ; un autre mod peut y ajouter) :

```ron
(machines: {
    "press": [
        (inputs: [("core:iron_ingot", 2)], outputs: [("example:plate", 1)], time: 2.0, energy: 200),
    ],
})
```

La machine demande au jeu ce qu'elle peut fabriquer : `find_recipe("example:press",
&[("core:iron_ingot", 5)])` renvoie la première recette faisable (`Recipe` : durée en ticks,
énergie, entrées, sorties), ou `None`. Un ingrédient peut être un groupe (`#planks`).
Le jeu ne retire rien tout seul : la machine consomme ses entrées (`item_extract`) et range ses
sorties (`item_insert`) quand elle le décide.

### Les capacités : énergie, fluides, objets, rotation

Les mods techniques s'échangent tous les mêmes choses. Le moteur fournit des conteneurs
**génériques**, que n'importe quel mod sait lire : un câble d'un mod peut alimenter la machine
d'un autre (le principe des « capabilities » de Forge).

| Déclarer (à la pose) | Utiliser |
|---|---|
| `add_energy(pos, capacité, débit, faces)` | `energy_stored`, `energy_insert`, `energy_extract` |
| `add_tank(pos, capacité_mB, faces)` | `fluid_amount`, `fluid_fill`, `fluid_drain` |
| `add_inventory(pos, cases, faces)` | `item_count`, `item_insert`, `item_extract` |
| `add_kinetic(pos, faces)` | `kinetic_speed`, `set_kinetic(vitesse, capacité, consommation)` |

- **Faces** : chaque conteneur dit par quelles faces il échange (`Sides::ALL`,
  `Sides::only(Face::PosY)`...). Une opération indique la face par laquelle elle arrive, ou
  `None` pour « de l'intérieur » (la machine elle-même), toujours permis.
- **Simulation** : `simulate = true` demande « combien accepterais-tu ? » sans rien changer.
- **Unités** : énergie libre (comme les FE), fluides en millibuckets (1000 mB = un bloc), objets et
  fluides désignés par leur identifiant texte (`"core:water"`, `"core:ruby"`).

Les **réseaux** (câbles, tuyaux, arbres de transmission) qui font circuler ces quantités
d'elles-mêmes arrivent en phase 9c ; d'ici là, un script déplace ce qu'il veut entre voisins.

### Les grilles

Toutes les positions ont une **grille** (`pos.grid`) : `Grid::WORLD` pour le monde. Les
structures mobiles (phase 9d : contraptions, vaisseaux) auront chacune la leur, avec des
coordonnées locales qui ne changent pas quand la structure bouge. Un mod qui utilise
`pos.offset(...)` et les fonctions ci-dessus marchera donc sans changement sur un vaisseau.

## Le bac à sable

Un script est du code écrit par quelqu'un d'autre. Il tourne dans `wasmtime`, isolé :
- **ni fichiers, ni réseau** : il ne voit que les fonctions ci-dessus (pas de WASI). Un module
  qui demande autre chose est refusé au chargement ;
- **mémoire bornée** : 32 Mio au plus ;
- **temps borné** : chaque appel reçoit du « carburant » (50 millions d'instructions). Un script
  qui boucle à l'infini tombe en panne sèche, l'appel s'arrête ;
- **budget par tick** : chaque mod dispose de 20 millions d'instructions par tick de machines (tous
  ses appels confondus). Au-delà, ses événements du tick sont perdus et ses machines reportées d'un
  tick (réveillées par un tick programmé) : un mod gourmand se ralentit lui-même, pas le jeu.
  Un mod qui dépasse son budget pendant 100 ticks de suite (5 secondes) est désactivé. Le budget est
  rendu à chaque tick ; `cargo xtask ctl mods` montre ce que chaque mod consomme (dernier tick,
  pire tick, appels reportés) ;
- **un script qui plante ou boucle est désactivé**, avec un message dans les logs ; le jeu
  continue sans lui.

## L'interface binaire (pour écrire un mod dans un autre langage)

`mod_sdk` cache ces détails ; un mod en C, Zig ou AssemblyScript doit les respecter. Le module
WebAssembly n'échange que des entiers et des flottants ; les textes et les listes passent par sa
mémoire.

**Exports du mod** :

| Export | Rôle |
|---|---|
| `memory` | sa mémoire |
| `sdk_alloc(taille: i32) -> i32` | réserve `taille` octets, où le jeu écrira les données d'un appel |
| `mod_init()` | facultatif, appelé au chargement |
| `on_event(type: i32, ptr: i32, taille: i32)` | facultatif |
| `on_machines(type: i32, ptr: i32, nombre: i32)` | facultatif ; type 0 posées, 1 chargées, 2 voisin, 3 tick |
| `on_ui(type: i32, g: i32, x: i32, y: i32, z: i32, id: i32, valeur: i32)` | facultatif ; action dans un menu : type 1 bouton, 2 case (valeur 0/1), 3 curseur (bits d'un f32), 4 rafraîchissement, 5 fermé |
| `on_options(type: i32, id: i32, valeur: i32)` | facultatif ; page d'options : type 6 ouverture, puis 1 à 3 actions du joueur, 5 fermée ; le mod répond par `options_show` |
| `on_call(ptr: i32, taille: i32) -> i32` et `call_reply() -> i32` | facultatifs ; un autre mod appelle une fonction : le jeu écrit `[longueur du nom: u16][nom][arguments]` à `sdk_alloc`, `on_call` renvoie la taille de la réponse (−1 : aucune), que le jeu lit à l'adresse donnée par `call_reply` |
| `on_command(ptr: i32, taille: i32) -> i32` | facultatif ; une commande `/nom` : même format que `on_call` (`[longueur du nom: u16][nom][arguments en texte]`), la réponse est lue à l'adresse de `call_reply` |
| `on_mobs(ptr: i32, nombre: i32)` | facultatif ; un lot de créatures à comportement : `nombre` enregistrements de 12 mots de 4 octets (`id, type, x, y, z, vie, vie max, vers_joueur_x, _y, _z, drapeaux, mémoire`), à répondre par `mob_*` |
| `on_use(g: i32, x: i32, y: i32, z: i32) -> i32` | facultatif ; clic droit du joueur sur une machine du mod ; renvoie 1 si le clic est pris en compte |

**Données** (petit-boutistes) : les positions de `on_machines` sont `nombre` fois
`grille, x, y, z` (i32) ; celles de chaque événement sont dans le tableau des événements
ci-dessus, avec son numéro.

**Imports** (module `voxel`) : un bloc s'y désigne par `g, x, y, z` (grille puis position) ;
une face par 0 à 5 (+X, −X, +Y, −Y, +Z, −Z) ou −1 ; un texte par `(ptr, len)` ; `simulate` vaut
0 ou 1. En cas d'échec (grille ou bloc inexistant), une fonction renvoie 0 ou −1, jamais une
erreur ; seul un pointeur hors de la mémoire du mod arrête le script.

Ces tableaux sont générés depuis le catalogue `crates/mod_sdk/hooks.ron` (`cargo xtask
hooks`), la seule source de vérité : le SDK et les numéros du jeu en sont générés aussi, et un
test vérifie que le jeu définit exactement ces fonctions.

<!-- hooks:imports -->
*Bases*

| Fonction | Rôle |
|---|---|
| `log(ptr: i32, len: i32)` | Écrit le texte dans le journal du jeu, préfixé du nom du mod. |
| `subscribe(kind: i32)` | S'abonne à un type d'événement (voir les événements) : `on_event` le recevra. |
| `random() -> i32` | Un nombre pseudo-aléatoire positif (reproductible pour une même graine). |
| `give_item(id: i32, count: i32)` | Donne `count` objets du bloc `id` au joueur. |
| `spawn_entity(ptr: i32, len: i32, x: f32, y: f32, z: f32)` | Fait apparaître une créature (identifiant texte, par exemple `core:pig`) à cette position. |

*Blocs*

| Fonction | Rôle |
|---|---|
| `block_id(ptr: i32, len: i32) -> i32` | Le numéro d'un bloc à partir de son identifiant (`core:stone`) ; −1 s'il n'existe pas. |
| `block_key(id: i32, ptr: i32, cap: i32) -> i32` | L'identifiant texte d'un bloc (`core:stone`), écrit à `ptr` (au plus `cap` octets) ; renvoie sa taille, −1 si le bloc n'existe pas. C'est aussi le nom de l'objet qu'il donne en inventaire. |
| `block_drops(id: i32, ptr: i32, cap: i32) -> i32` | Ce que lâcherait ce bloc cassé : `objet*nombre;objet*nombre`, écrit à `ptr` (au plus `cap` octets) ; renvoie la taille (0 : rien), −1 si le bloc n'existe pas. Le hasard est tiré à chaque appel. |
| `block_base(id: i32) -> i32` | Le bloc de base d'un bloc à états : `example:stairs[facing=east]` donne `example:stairs`. Un bloc ordinaire se renvoie lui-même. |
| `get_block(g: i32, x: i32, y: i32, z: i32) -> i32` | Le bloc à cette position (0 : de l'air, ou une grille inconnue). |
| `set_block(g: i32, x: i32, y: i32, z: i32, id: i32) -> i32` | Pose un bloc ; les voisins sont prévenus. Si le bloc remplacé avait un inventaire (coffre, machine), son contenu tombe au sol. 1 si c'est fait. |
| `get_power(g: i32, x: i32, y: i32, z: i32) -> i32` | La puissance émise par ce bloc (0 à 15). |
| `received_power(g: i32, x: i32, y: i32, z: i32) -> i32` | La plus forte puissance reçue des 6 voisins. |
| `set_power(g: i32, x: i32, y: i32, z: i32, power: i32) -> i32` | Change la puissance émise (0 à 15) ; les voisins sont prévenus. 1 si c'est fait. |
| `get_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, cap: i32) -> i32` | Copie l'état libre du bloc (au plus `cap` octets) ; renvoie sa taille, −1 sans état. |
| `set_data(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32` | Remplace l'état libre du bloc, sauvegardé avec le monde. 1 si c'est fait. |
| `schedule_tick(g: i32, x: i32, y: i32, z: i32, delay: i32)` | Réveille la machine dans `delay` ticks (20 par seconde). |

*Capacités*

| Fonction | Rôle |
|---|---|
| `add_energy(g: i32, x: i32, y: i32, z: i32, capacity: i64, rate: i64, sides: i32) -> i32` | Donne au bloc un stockage d'énergie, ouvert sur les faces `sides` (6 bits). 1 si c'est fait. |
| `add_tank(g: i32, x: i32, y: i32, z: i32, capacity: i32, sides: i32) -> i32` | Ajoute un réservoir de `capacity` mB. 1 si c'est fait. |
| `add_inventory(g: i32, x: i32, y: i32, z: i32, slots: i32, sides: i32) -> i32` | Donne au bloc un inventaire de `slots` cases. 1 si c'est fait. |
| `add_kinetic(g: i32, x: i32, y: i32, z: i32, sides: i32) -> i32` | Donne au bloc une rotation (vitesse, capacité et consommation de contrainte). 1 si c'est fait. |
| `energy_stored(g: i32, x: i32, y: i32, z: i32) -> i64` | L'énergie stockée ; −1 sans stockage. |
| `energy_insert(g: i32, x: i32, y: i32, z: i32, face: i32, amount: i64, simulate: i32) -> i64` | Insère de l'énergie par cette face ; renvoie la quantité acceptée. |
| `energy_extract(g: i32, x: i32, y: i32, z: i32, face: i32, amount: i64, simulate: i32) -> i64` | Retire de l'énergie par cette face ; renvoie la quantité retirée. |
| `fluid_amount(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32` | La quantité (mB) de ce fluide dans les réservoirs du bloc. |
| `fluid_fill(g: i32, x: i32, y: i32, z: i32, face: i32, ptr: i32, len: i32, amount: i32, simulate: i32) -> i32` | Remplit avec ce fluide par cette face ; renvoie la quantité acceptée (mB). |
| `fluid_drain(g: i32, x: i32, y: i32, z: i32, face: i32, ptr: i32, len: i32, amount: i32, simulate: i32) -> i32` | Vide ce fluide par cette face ; renvoie la quantité retirée (mB). |
| `item_count(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32) -> i32` | Le nombre de cet objet dans l'inventaire du bloc. |
| `item_insert(g: i32, x: i32, y: i32, z: i32, face: i32, ptr: i32, len: i32, count: i32, simulate: i32) -> i32` | Range des objets par cette face ; renvoie le nombre accepté. |
| `item_extract(g: i32, x: i32, y: i32, z: i32, face: i32, ptr: i32, len: i32, count: i32, simulate: i32) -> i32` | Sort des objets par cette face ; renvoie le nombre retiré. |
| `slot_get(g: i32, x: i32, y: i32, z: i32, slot: i32, ptr: i32, cap: i32) -> i32` | Lit une case de l'inventaire du bloc : écrit `objet*nombre` (au plus `cap` octets) et renvoie la taille ; −1 si la case est vide ou n'existe pas. |
| `slot_set(g: i32, x: i32, y: i32, z: i32, slot: i32, ptr: i32, len: i32) -> i32` | Remplace une case de l'inventaire par `objet*nombre` (texte vide : la vider). 1 si c'est fait. |
| `slot_damage(g: i32, x: i32, y: i32, z: i32, slot: i32) -> i32` | L'usure de l'objet dans cette case (0 : neuf, l'objet casse quand elle atteint sa durabilité) ; −1 si la case est vide ou n'existe pas. |
| `slot_set_damage(g: i32, x: i32, y: i32, z: i32, slot: i32, damage: i32) -> i32` | Règle l'usure de l'objet dans cette case. 1 si c'est fait ; 0 si la case est vide ou n'existe pas. `slot_set` remet l'usure à 0. |
| `kinetic_speed(g: i32, x: i32, y: i32, z: i32) -> f32` | La vitesse de rotation (tr/min, signée). |
| `set_kinetic(g: i32, x: i32, y: i32, z: i32, speed: f32, capacity: f32, impact: f32) -> i32` | Règle la vitesse, la capacité et la consommation de contrainte. 1 si c'est fait. |

*Interface*

| Fonction | Rôle |
|---|---|
| `ui_show(g: i32, x: i32, y: i32, z: i32, ptr: i32, len: i32)` | Ouvre (ou met à jour) le menu de la machine à cette position : `len` octets décrivant le menu (voir `Menu`). Le joueur le voit à l'écran. |
| `ui_close()` | Ferme le menu du mod s'il est ouvert. |
| `options_show(ptr: i32, len: i32)` | Affiche (ou met à jour) la page d'options du mod : `len` octets décrivant le menu. |

*Réglages*

| Fonction | Rôle |
|---|---|
| `setting_get(key_ptr: i32, key_len: i32, out_ptr: i32, cap: i32) -> i32` | Copie la valeur (texte) du réglage du mod (au plus `cap` octets) ; renvoie sa taille, −1 s'il n'existe pas. |
| `setting_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32` | Enregistre un réglage du mod (clé de 64 octets, valeur de 1 024, 256 clés au plus). 1 si c'est fait. |
| `world_get(key_ptr: i32, key_len: i32, out_ptr: i32, cap: i32) -> i32` | Copie la valeur (texte) d'une donnée du mod **dans ce monde** (au plus `cap` octets) ; renvoie sa taille, −1 si elle n'existe pas. |
| `world_set(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32` | Enregistre une donnée du mod dans ce monde (clé de 64 octets, valeur de 64 Kio, 1 024 clés au plus), sauvegardée avec le monde. 1 si c'est fait. |

*Créatures*

| Fonction | Rôle |
|---|---|
| `mob_kind(ptr: i32, len: i32) -> i32` | Le numéro d'un type de créature (`example:guard`) tel que le montrent les créatures reçues dans `on_mobs` ; −1 s'il n'existe pas. |
| `mob_move(mob: i32, dx: f32, dz: f32, speed: f32)` | Pendant `on_mobs` : la créature marche dans la direction (dx, dz) à `speed` blocs par seconde (au plus le double de sa vitesse de base) ; elle se tourne vers là. Une vitesse nulle l'arrête. Valable jusqu'au prochain appel. |
| `mob_move_to(mob: i32, x: f32, y: f32, z: f32, speed: f32)` | Pendant `on_mobs` : la créature va jusqu'à ce point en **cherchant son chemin** (contourne les murs, saute les marches, évite la lave) à `speed` blocs par seconde (au plus le double de sa vitesse de base). Elle continue seule entre deux appels, vers le point le plus proche si le but est inaccessible ; `mob_move` ou un nouveau `mob_move_to` change son ordre. |
| `mob_jump(mob: i32)` | Pendant `on_mobs` : la créature saute (si elle touche le sol). |
| `mob_attack(mob: i32, damage: f32)` | Pendant `on_mobs` : la créature blesse le joueur s'il est à portée de bras (3 blocs) et si elle n'a pas frappé depuis une seconde. |
| `mob_state(mob: i32, value: i32)` | Pendant `on_mobs` : la mémoire de la créature (un entier de votre choix, rendu dans `state` au prochain appel). |

*Joueur*

| Fonction | Rôle |
|---|---|
| `player_info(out_ptr: i32, cap: i32) -> i32` | Écrit l'état du joueur : 9 décimaux (f32) `x, y, z` (ses yeux), `dx, dy, dz` (le regard, de longueur 1), `vie, vie max, niveau` ; renvoie leur nombre (0 s'il n'y a pas de joueur). |
| `player_target(max_dist: f32, out_ptr: i32) -> i32` | Lance un rayon dans la direction du regard : si un bloc est touché à moins de `max_dist` blocs, écrit le point d'impact (3 f32) et renvoie 1 ; sinon 0. |
| `player_heal(amount: f32)` | Rend de la vie au joueur. |
| `player_teleport(x: f32, y: f32, z: f32)` | Téléporte le joueur (les pieds en x, y, z), à 64 blocs au plus de là où il est. |
| `player_effect(ptr: i32, len: i32, seconds: f32, power: f32)` | Donne un effet au joueur : `speed`, `strength`, `regeneration`, `fire_resistance` ou `healing`. |
| `hurt_area(x: f32, y: f32, z: f32, radius: f32, damage: f32)` | Blesse les créatures à moins de `radius` blocs de ce point (jusqu'à 16 blocs) ; elles reculent. |
| `say(ptr: i32, len: i32)` | Écrit une ligne dans la console du joueur (un message : « pas assez de mana »). |
| `consume_held(count: i32)` | Retire `count` exemplaires de l'objet que le joueur tient en main (une potion qu'on boit, un cristal qu'on use). |
| `hurt_player(amount: f32)` | Blesse le joueur (l'armure et l'invulnérabilité après un coup comptent, comme pour une créature) : un piège, une explosion, une malédiction. |
| `hud_bar(id: i32, fraction: f32, r: f32, g: f32, b: f32, ptr: i32, len: i32)` | Affiche (ou met à jour) une jauge du mod à l'écran : `fraction` de 0 à 1, couleur (r, g, b) de 0 à 1, et son texte. Quatre jauges au plus par mod. |
| `hud_clear(id: i32)` | Retire une jauge du mod de l'écran. |

*Dimensions*

| Fonction | Rôle |
|---|---|
| `travel_to(ptr: i32, len: i32) -> i32` | Fait passer le joueur dans cette dimension (`mon_mod:ma_dimension`) : le monde est sauvegardé puis rechargé, et le joueur arrive au sol aux mêmes x et z. 1 si le voyage est lancé, 0 si la dimension est inconnue ou si le monde n'est pas sauvegardé. |
| `dimension(out_ptr: i32, cap: i32) -> i32` | Copie l'identifiant de la dimension où se trouve le joueur (`core:overworld`, `mon_mod:ma_dimension`) ; renvoie sa taille. |

*Commandes*

| Fonction | Rôle |
|---|---|
| `register_command(name_ptr: i32, name_len: i32, help_ptr: i32, help_len: i32)` | Déclare la commande `/nom` du mod (à appeler dans `init`) avec sa ligne d'aide : le joueur la tape dans la console, et le mod la reçoit dans `on_command`. |

*Appels entre mods*

| Fonction | Rôle |
|---|---|
| `call_mod(mod_ptr: i32, mod_len: i32, name_ptr: i32, name_len: i32, arg_ptr: i32, arg_len: i32, out_ptr: i32, cap: i32) -> i32` | Appelle la fonction `name` du mod `mod` avec ces octets (64 Kio au plus) ; copie sa réponse (au plus `cap` octets) et renvoie sa taille. −1 si le mod est absent, désactivé, hors budget, déjà en cours d'appel (pas de cycle) ou s'il ne répond pas. |
| `mod_loaded(ptr: i32, len: i32) -> i32` | 1 si le mod (son identifiant) est chargé et actif. |

*Objets et recettes*

| Fonction | Rôle |
|---|---|
| `item_id(ptr: i32, len: i32) -> i32` | Le numéro d'un objet (bloc ou non) à partir de son identifiant (`core:ruby`) ; −1 s'il n'existe pas. |
| `item_max_stack(ptr: i32, len: i32) -> i32` | Combien d'exemplaires de cet objet tiennent dans une case (1 pour un outil) ; 0 s'il n'existe pas. |
| `item_durability(ptr: i32, len: i32) -> i32` | Le nombre d'utilisations d'un objet avant qu'il casse (outils, armures) ; 0 s'il n'use pas ou n'existe pas. |
| `give_stack(id: i32, count: i32)` | Donne `count` objets de numéro `id` (voir `item_id`) au joueur. |
| `recipe_find(kind_ptr: i32, kind_len: i32, in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> i32` | Cherche une recette du type donné (`example:press`) faisable avec les objets `cle*nombre,cle*nombre` de l'entrée. Écrit `ticks;énergie;entrées;sorties` (même format) dans la sortie ; renvoie sa taille, −1 sans recette. |

*Quêtes*

| Fonction | Rôle |
|---|---|
| `quest_signal(ptr: i32, len: i32) -> i32` | Envoie le signal `nom` du mod au livre de quêtes : les tâches `Signal("<mod>:nom")` de ses quêtes se remplissent (l'espace de noms est toujours celui du mod). Le nom : minuscules, chiffres, `_`, `-`, `.` et `/`, 64 octets au plus. 1 si le signal est pris en compte, 0 si le nom est refusé. |
| `quest_done(ptr: i32, len: i32) -> i32` | 1 si la quête (son identifiant complet, `core:wood/workbench`) est terminée dans ce monde, 0 sinon (aussi pour une quête inconnue ou si le livre n'est pas encore chargé). |

<!-- /hooks:imports -->

## Un mod de référence : Mécanique

`mods/mecanique` est écrit **uniquement avec l'API publique** : c'est le test grandeur nature de ce
document. Il contient :
- des **arbres de transmission et des engrenages** (blocs en boîtes, orientables, `conduit: Kinetic`),
  sans une ligne de script ;
- une **manivelle** (source de rotation : clic droit, vitesse et durée réglables dans sa page
  d'options) ;
- une **presse** (machine à deux cases, branchée au réseau, avec son menu : cases, progression,
  vitesse), qui transforme lingots et pavés en plaques et graviers selon `machine_recipes.ron` ;
- des objets (`items.ron`), des recettes d'artisanat (`recipes.ron`) et ses tests sans fenêtre
  (`cargo xtask test-mod mecanique`).

Ce qu'il a fallu ajouter à l'API pour l'écrire : les conduits de mods (`conduit:`), la lecture et
l'écriture d'une case d'inventaire (`slot`, `set_slot`), et les réseaux d'objets pour ses **courroies**
(`conduit: Items`, `channel`, `ports` sur la presse), puis le **sens unique** (`flow`) pour son convoyeur, sa diode et son clapet.

## Un mod de référence : Entrepôt

`mods/entrepot` : le stockage et l'automatisation, toujours avec l'API publique.
- un **grand coffre** (54 cases, comme un coffre pour les réseaux d'objets) avec un bouton « Ranger »
  qui regroupe les piles et trie par objet (`item_max_stack` : les objets qui ne s'empilent pas, outils
  et armures, ne bougent pas) ;
- un **trieur** : `ports` avec un `filter` (voir plus haut) ; un grand coffre voisin lui donne ce qui
  correspond, le reste passe au coffre ;
- une **moissonneuse** : toutes les 2 secondes, elle cueille les cultures mûres à 3 blocs et les replante.
  Elle reconnaît une culture mûre sans rien savoir de chaque plante : `block_key` donne `core:wheat_3`,
  et `block_drops` tire le butin des tables du jeu (blé et graines, carottes...) ; si tout ne tient pas dans
  ses 9 cases, elle ne casse rien ;
- une **foreuse** : alimentée (un levier voisin), elle creuse un puits droit, un bloc par seconde, et
  garde le butin (`block_drops` : la pierre donne des pavés). Elle s'arrête sur la roche mère, l'eau,
  la lave, ou quand ses cases sont pleines.

Les objets circulent par les courroies de Mécanique (ou tout conduit `Items`) : harvester ou foreuse
*produisent* (leurs `outputs`), trieurs et coffres *reçoivent*. Un réseau envoie d'abord aux machines
(trieur), puis aux coffres : la moissonneuse + trieur de graines + grand coffre trie donc la récolte
toute seule. Tests : `cargo xtask test-mod entrepot`.

## Un mod de référence : Forge

`mods/forge` : l'artisanat en chaîne, toujours avec l'API publique. Minerais → lingots → **alliages**
→ **pièces forgées** → outils et armures.
- un **four à alliages** : trois cases d'entrée, une de combustible, une de sortie. Il lit ses recettes
  dans `machine_recipes.ron` (type `forge:alloy` : fer + charbon donne de l'acier, cuivre + malachite
  du bronze) avec `find_recipe`, qui accepte plusieurs ingrédients ; il brûle du charbon (ou des
  bâtons) et la recette n'avance que tant que le feu tient. Ses données (feu restant, progression)
  sont dans `set_data`. Il se réveille à chaque tick seulement quand il travaille ;
- une **enclume** : un **marteau de bronze** (catalyseur, jamais consommé), deux cases de lingots et
  une sortie. Un bouton par pièce ; chaque bouton est **un type de recette** (`forge:anvil_plate`,
  `forge:anvil_pickaxe`...), donc ajouter une pièce = une ligne de données et un bouton ;
- des outils et armures d'acier (`items.ron`) que l'on assemble à l'établi (`recipes.ron`) à partir
  des pièces : tête de pioche + 2 bâtons, plaques d'acier pour l'armure.
Le four et l'enclume ont des `ports`, donc un réseau d'objets (Mécanique, Entrepôt) peut les alimenter
et vider leur sortie. Le **marteau s'use** (200 coups, chacun une usure, il se brise ensuite) et l'enclume **répare** : le
bouton « Réparer » rend un quart de la durabilité d'un outil par lingot d'acier. C'est l'API d'usure :
`slot_damage` / `set_slot_damage` lisent et règlent l'usure d'une pile, `item_durability` donne le
maximum (une pile est cassée quand l'usure l'atteint). Attention : `set_slot` remet l'usure à 0, donc
la régler après avoir posé l'objet. Tests : `cargo xtask test-mod forge`.

## La génération du monde

Un mod enrichit le terrain de trois façons, de la plus simple à la plus libre. Tout est calculé par
le moteur, en tâche de fond, à partir de la graine du monde : le même monde a toujours les mêmes
structures et filons, et rien ne ralentit l'image.

**1. Des minerais** — `ores.ron` du mod (même format que `assets/core/ores.ron` : bloc, altitude,
taille d'un filon, nombre par chunk, biomes). Un filon ne remplace que la roche (`replace`), et un mod
peut y placer les blocs des autres.

**2. Des biomes** — `biomes.ron` du mod (même format que `assets/core/biomes.ron`).
Dans chaque famille (`kind`), le biome retenu est celui dont le point (`climate`, `variety`) est le
plus proche du climat de la colonne : un biome de mod avec une `variety` forte devient une variante rare
d'un climat existant. `groves` règle le regroupement des arbres en bosquets, et une plante peut
s'écrire `("poppy", 0.01, 0.9)` pour dire à quel point elle pousse en taches.
`ground_patches: [("podzol", 0.3)]` sème des taches d'un autre sol en surface, `bands: [...]` remplace
le sous-sol par des strates horizontales, et `relief: (offset: 18.0, scale: 2.8, terraces: 1.0)` découpe
le relief en paliers (des mesas).

**3. Des structures** — `structures.ron` du mod. Les sortes du jeu (villages, ruines, donjons, mines)
ont leur dessin dans le moteur ; un mod ajoute ses propres **plans**, décrits en texte :

```ron
(
    structures: [
        (id: "mon_mod:atelier", kind: Template, template: "atelier", spacing: 12, chance: 0.5, biomes: []),
    ],
    templates: {
        "atelier": (
            palette: {'#': "core:cobblestone", 'c': "mon_mod:manivelle"},
            layers: [                       // de bas en haut ; une ligne par z, un caractère par x
                ["#####", "#####", "#####"],
                ["#.c.#", "#...#", "#####"],   // `.` : de l'air, espace : le terrain reste
            ],
            chests: [((3, 1, 1), "butin")], // (x, y, z) dans le plan, table de butin
            rotate: false,                  // ne pas tourner au hasard (oui par défaut)
        ),
    },
    loot: { "butin": (rolls: (2, 4), entries: [(item: "core:coal", count: (2, 5), weight: 6)]) },
)
```

`spacing` (en chunks) et `chance` règlent la rareté ; `biomes` où le plan apparaît (vide : toutes les
terres). Le plan est posé sur le point le plus haut de son emprise, avec des fondations jusqu'au sol
(`foundation`, pierre taillée par défaut), et l'air de son volume est dégagé (`clear: false` pour
l'éviter). Une palette peut citer un bloc à états par son identifiant complet
(`mecanique:shaft[axis=x]`) ; comme une rotation ne les tourne pas, mettre `rotate: false` pour un
plan qui en contient. Les tables de butin d'un mod sont à son nom (`mon_mod:butin`) ; il peut aussi
utiliser celles du jeu (`ruin`, `village`...). Un fichier invalide est refusé avec la raison dans
les logs, et le monde se génère sans ce que les mods apportent.

**4. Du code, pour décorer** — l'événement `ChunkGenerated` : quand un chunk **non vide** est généré
pour la première fois (jamais pour un chunk relu d'une sauvegarde), le script peut le décorer.
`Event::ChunkGenerated { chunk }` donne ses coordonnées de chunk (`chunk.chunk_origin()` : son coin
minimal, 32 blocs de côté) ; le script lit et écrit dans ce chunk avec `get_block` et `set_block`
(hors des chunks chargés, ils échouent). Le jeu confie au plus 8 chunks par tick, et seulement quand
tous les scripts abonnés ont du budget : un chunk n'est pas perdu parce qu'un mod est occupé.
Limites : un chunk déchargé avant que son événement soit livré n'est pas décoré, et un chunk entièrement
vide (de l'air) ne déclenche rien.

## Les créatures à comportement

Une créature se décrit en données (`entities.ron` : vie, vitesse, taille, apparence, butin, règles
d'apparition). Son **comportement** peut venir du jeu (errance, poursuite des hostiles, fuite, tir,
reproduction) ou de **votre script** : ajoutez `brain: true` et c'est le script du mod qui la dirige.
La physique, les dégâts reçus, la mort, le butin et le rendu restent ceux du jeu.

```ron
(id: "mon_mod:gardien", name: "Gardien", health: 20.0, speed: 2.2, size: (0.7, 1.7),
 hostile: true, brain: true, spawn_on: ["core:grass"], parts: [ /* ... */ ])
```

**L'animation** vient de la forme : chaque boîte de `parts` peut dire son rôle avec `bone` (sans rien, elle
reste immobile). `bone: Leg` et `bone: Arm` se balancent en marchant (par le haut de la boîte ; une patte
et son sabot, à la même place, pivotent ensemble par le haut de la plus haute ; les pattes d'un animal
vont en diagonale), d'autant plus que la créature va vite. `bone: Head` marque toutes les boîtes de la
tête : elles se tournent ensemble, autour du cou, vers le joueur quand il est à moins de 8 blocs (16
pour une hostile), par moments, et font un petit coup d'œil au hasard le reste du temps. Une créature
de mod en profite sans script : `parts: [(offset: (0.3, 0.1, 0.2), size: (0.2, 0.3, 0.2), color: (..),
bone: Leg), ...]`.

**Les matières** : une pièce peut ajouter `texture: "pelage"` (dans
`mods/mon_mod/textures/pelage.png`) ou un nom qualifié comme
`texture: "core:mob_pig_skin"`. Le PNG RGBA remplit chaque face de la boîte ; il
est étiré avec cette pièce, puis suit son articulation. Sans `texture`, la pièce
conserve sa couleur unie. Avec une texture, `color` reste sa couleur de repli :
utiliser la moyenne linéaire des pixels sRGB, pixels transparents exclus.
Les surfaces sont ajoutées au tableau graphique après celles des blocs, sans
ajouter de bloc ni changer ses identifiants. Le pack sélectionné remplace aussi
ces PNG, en 16 ou en 32 ; les mêmes formes et pivots servent aux deux résolutions.
F5 recharge les images ; changer les boîtes de `entities.ron` demande de relancer.

Une créature avec `aquatic: true` apparaît dans le liquide de `spawn_on` (actuellement
l'eau `core:water`), avec sa boîte entièrement immergée. Elle nage en trois dimensions,
évite les volumes secs et respire dans l'eau ; hors de l'eau elle chute et perd sa
réserve d'air. Les bancs ont une limite distincte de douze créatures dans la zone
de simulation. Le champ est facultatif et vaut `false` pour les anciens modèles.
La queue et toutes ses nageoires caudales peuvent porter `bone: Tail` : elles
partagent le pivot à l'avant de la plus grosse boîte de queue et oscillent latéralement.
`bone: Head`, `Leg` et `Arm` conservent leurs rôles habituels.

Le mouton de base conserve le même type `core:sheep` quelle que soit sa couleur.
Son composant de toison choisit une entrée de `assets/core/wool_colors.ron` ; ce
fichier contient les RGB sRGB des teintes, leur objet de laine et leur poids
d'apparition (`weight` pour un mouton, `alpaca` pour un alpaga ; 0 : la couleur n'apparaît
pas naturellement chez l'espèce). Il est intégré à la compilation : après modification, régénérer
les PNG et reconstruire le jeu. Les valeurs `color` des objets restent les
moyennes linéaires des PNG. Les teintes s'appliquent seulement aux pièces portant
une texture dont le nom finit par `_wool` ou `_fur` (`core:mob_sheep_wool`, `core:mob_alpaca_fur`). La couleur est héritée d'un parent à la naissance
et copiée dans l'événement interne de mort pour le butin après disparition de
l'entité ; l'événement envoyé aux scripts garde son format. Le groupe d'objets
`#wools` contient les dix-huit laines ; `core:wool` est toujours la blanche.

Toute créature peut avoir une **toison** : `fleece: (coats: Alpaca, count: (1, 3), regrow: 60.0,
skin: (0.78, 0.58, 0.5))` (`coats` : `Sheep` par défaut ou `Alpaca`, les couleurs naturelles de
l'espèce ; `count` : les laines d'une tonte ; `regrow` : les secondes de pâture avant que la
laine repousse ; `skin` : la peau sous la laine). Le joueur la tond d'un clic droit avec des
cisailles : la laine, de la couleur de l'animal, tombe, et l'animal est dessiné sans les boîtes
dont la texture finit par `_wool` (elles deviennent une peau amincie) jusqu'à ce qu'il ait
brouté. Un petit ne se tond pas. `spit: (range, interval, speed, damage, color)` fait cracher
(un projectile) une créature pacifique sur le joueur qui la frappe.

```ron
(offset: (0.0, 0.5, 0.0), size: (0.8, 0.6, 0.5),
 color: (0.2, 0.3, 0.1), texture: "pelage")
```

```rust
fn init(&mut self) {
    self.gardien = mod_sdk::mob_kind("mon_mod:gardien");          // son numéro de type
}

fn mobs(&mut self, mobs: &[MobView]) {                            // 4 fois par seconde, TOUTES d'un coup
    for m in mobs.iter().filter(|m| Some(m.kind) == self.gardien) {
        if m.distance_to_player() < 18.0 {
            let p = m.position;
            let to = [p[0] + m.to_player[0], p[1] + m.to_player[1], p[2] + m.to_player[2]];
            mod_sdk::mob_move_to(m.id, to, 3.0);   // vers le joueur en cherchant son chemin, à 3 blocs/s
        }
    }
}
```

Le jeu confie au script les créatures de **son mod** (celles de l'espace de noms de leur
identifiant) qui sont à moins de 48 blocs du joueur, les plus proches d'abord, 64 au plus par appel.
Un appel du jeu vers le script coûte cher et un ordre du script vers le jeu presque rien : c'est
pourquoi tout le monde passe en un seul lot. Chaque `MobView` donne le numéro, le type, la position,
la vie, le vecteur vers le joueur, des drapeaux (au sol, dans un liquide, blessée) et la **mémoire**
de la créature (`state`). Les ordres :

| Ordre | Effet |
|---|---|
| `mob_move(id, dx, dz, vitesse)` | marcher dans cette direction (vitesse en blocs/s, au plus le double de sa vitesse de base ; 0 : s'arrêter) ; elle se tourne ; devant un obstacle elle saute |
| `mob_move_to(id, [x, y, z], vitesse)` | aller à ce point **en cherchant son chemin** (contourne les murs, saute les marches, évite la lave et les grandes chutes) ; elle continue seule entre deux appels, recalcule si le but bouge ou si elle se coince, et va au plus près si le but est inaccessible. Les recherches ont un budget partagé avec les créatures du jeu |
| `mob_jump(id)` | sauter |
| `mob_attack(id, dégâts)` | blesser le joueur s'il est à moins de 3 blocs, au plus une fois par seconde |
| `mob_state(id, valeur)` | ranger un entier (un état d'esprit : repos, poursuite, fuite...) qui revient au prochain appel |

Un ordre vaut jusqu'au prochain appel (250 ms) : une créature à qui on ne dit rien continue ce
qu'elle faisait. `mob_move` marche **droit** (utile pour une charge, ou pour fuir) et annule un
`mob_move_to` en cours ; pour poursuivre ou rejoindre un point, préférez `mob_move_to`. Le script ne commande que les créatures du lot qu'il vient de recevoir. Le coût
compte dans le budget par tick du mod ; hors budget, le lot est sauté et les créatures gardent leurs
derniers ordres. Le gardien du mod d'exemple (`example:guard`) rôde, poursuit le joueur, frappe de
près et fuit quand il est blessé ; son test sans fenêtre (`h.mobs(&[MobSnapshot {..}])`) vérifie
chaque état.

**5. Des donjons** — `kind: Graph` : des **salles reliées en graphe**, enterrées, avec un escalier jusqu'à la
surface. Les salles sont des plans (`templates`) ; `dungeons` dit comment les assembler :

```ron
dungeons: {"crypte": (
    rooms: [(template: "entree", role: Entry), (template: "salle", role: Room, weight: 3),
            (template: "tresor", role: Treasure), (template: "arene", role: Boss)],
    count: (6, 9), cell: 24, depth: 22,           // 6 à 9 salles, 24 blocs entre leurs centres, plancher à 22 sous la surface
    wall: "mon_mod:brique", light: "core:glowstone",
    stairs: "mon_mod:escalier",                   // facultatif : un bloc à états Facing4 (voir « Les formes et les états »)
    tiers: ["crypte_1", "crypte_2", "crypte_3"],  // les tables de butin, du plus pauvre au plus riche
)},
structures: [(id: "mon_mod:crypte", kind: Graph, dungeon: "crypte", spacing: 10, chance: 0.5, biomes: [])],
```

- **La disposition** pousse comme un arbre depuis l'entrée (chaque salle s'accroche à une salle voisine de la
  grille), à partir de la graine du site : le même monde a toujours les mêmes donjons. La salle la plus
  **profonde** (en nombre de salles depuis l'entrée) est celle du **boss** ; jusqu'à deux bouts de branche sont
  des salles du **trésor** ; les autres tirent un plan parmi les salles `Room` (selon leur `weight`).
- **La construction** : des couloirs de 3 × 3 (murs, sol et toit en `wall`, un bloc `light` encastré dans le toit
  tous les 6 blocs) percent les murs des salles pour y faire des portes ; un escalier monte de l'entrée
  jusqu'à la surface. Avec `stairs`, il est fait de **demi-marches** (le bloc donné, tourné dans le sens de la
  montée : la partie haute du côté où l'on monte) : le joueur le monte en marchant, car un corps au sol monte
  seul une marche de 0,6 bloc au plus (dalles et escaliers de tous les mods en profitent). Sans `stairs`, ce
  sont des marches pleines qu'on franchit en sautant. Les plans ne tournent pas.
- **Le butin à paliers** : la profondeur d'une salle donne son palier. Dans un plan, un coffre nomme sa table :
  `"@tier"` (le palier de la salle), `"@treasure"` (un palier au-dessus) ou `"@boss"` (le dernier), ou une
  table précise. Plus on s'enfonce, plus c'est riche.
- **La vie du donjon** vient des blocs que les plans posent : des **machines** de script (un piège qui blesse, une
  cage qui fait surgir des créatures à l'approche, un sceau qui réveille le boss). Le monde donne un état à
  ces blocs pour que le jeu les réveille au chargement du chunk.
- **Limites** : 3 à 9 salles, `cell` d'au moins la plus grande salle + 6, `spacing × 32` plus grand que le rayon du
  donjon (le fichier est refusé sinon, avec le nombre qu'il faut).

Le mod **Donjons** (`mods/donjons`) en est la démonstration : une crypte (entrée, salles à piliers avec cage à
gardiens, salles piégées, salle du trésor, arène), un **boss** (`donjons:warden`, dirigé par script : il poursuit,
charge, frappe, appelle des renforts à mi-vie, affiche sa jauge et laisse un coffre à sa mort) et trois paliers de
butin.

## L'aventure : raretés, affixes, gemmes, paliers, élites

Le butin à affixes est entièrement en données (`assets/core/*.ron` est le modèle). Un mod ajoute les siens en
**plaçant le même fichier** dans son dossier : les listes s'ajoutent à la suite de celles du jeu de base, et chaque
identifiant porte l'espace de noms du mod.

- `rarities.ron` : les raretés (nom, couleur de la bulle et du liseré, poids, nombre d'affixes et d'emplacements de
  gemme, matériau, coûts de récupération et de réforge). Leur **ordre** est celui que les objets sauvegardent : on
  n'en déplace jamais une, on en ajoute à la fin.
- `affixes.ron` : un nom (`Prefix("acéré", "acérée")` ou `Suffix("de la tempête")`), les catégories d'objets qu'il
  peut orner (`weapon`, `tool`, `armor`, `head`, `chest`, `legs`, `feet`), un attribut (`Attribute::key` : `damage`,
  `crit_chance`...) et un intervalle de valeurs **par rareté**. Le nom de l'objet est composé de ces affixes.
- `gems.ron` : des sortes de gemmes ; chaque qualité (1 à 5) est un objet (`item: "mon_mod:perle_{q}"`, à déclarer
  dans `items.ron`), et le bonus dépend de la catégorie de l'objet qui la porte.
- `tiers.ron` : des paliers du monde (vie et dégâts des créatures, élites, envahisseurs, hausse de rareté) et leurs
  critères de déblocage (`Kills`, `Bosses`, `Rarity`, `Depth`).
- `elites.ron` : les augmentations d'élite (`Health`, `Damage`, `Speed`, `Armor`, `Regen`) et la liste des objets de
  base des butins à affixes.
- Dans `entities.ron`, `boss: true` marque un boss : il ne devient jamais élite et le vaincre compte pour les paliers.
- Dans les tables de butin d'une structure (`structures.ron`), `affixed: true` (et `min_rarity`) range un objet à
  affixes dans le coffre ; ses affixes sont tirés à la première ouverture.

## L'enchantement avancé : étagères, enchantements, bibliothèque, charmes

Plan et choix : `docs/plan-apotheose.md` (phase 3). Tout se règle en données, et un mod ajoute les siennes.

- **Les étagères et les stats de la table.** Un bloc peut avoir un champ `enchanting` dans son `blocks.ron` :
  `enchanting: (eterna: 1.25, max_eterna: 40.0, quanta: 0.5, arcana: 0.0, clues: 0.0, stability: 0.0, treasure: false)`
  (tout est facultatif). Une table d'enchantement lit les **32 positions** autour d'elle (deux anneaux à deux blocs :
  à sa hauteur et un bloc au-dessus), à condition qu'il y ait de l'**air** à la même hauteur entre elle et le bloc. Les
  contributions s'**additionnent** (la table apporte aussi les siennes : elle donne la base de Quanta et un indice) ;
  `max_eterna` est un **plafond** : l'Eterna ne dépasse jamais le plus haut plafond des blocs présents (bois : 15,
  saphir : 40, améthyste : 50, jade : 65, obsidienne volcanique : 100). Quanta (hasard des niveaux), Arcana (nombre et
  rareté des enchantements) et Stabilité (le hasard ne fait que du bien) vont de 0 à 100 ; les indices (partie entière
  du total) disent combien d'enchantements d'un lot sont montrés avant l'achat ; `treasure: true` ouvre les
  enchantements rares.
- `enchantments.ron` : la liste des enchantements. Chacun a un identifiant (`mon_mod:vertu`), un **numéro** stable
  (de 64 à 127 pour un mod : il est sauvegardé dans les objets, on n'en change jamais un), un niveau maximal sans
  Eterna (`max_level`) et avec 100 d'Eterna (`hard_max`), un poids, `treasure`, les cibles (`Weapon`, `Digger`,
  `Armor`, `Durable`), éventuellement des pièces d'armure (`armor_slots`) et des **effets** : un bonus d'attribut par
  niveau, objet en main (`Held`) ou porté (`Worn`). Les attributs ajoutés par la phase 3 : `fortune`, `thorns`,
  `fall_resist`, `silk_touch`, `magnet`.
- **Livres et tomes.** `core:book` (vierge), `core:tome_weapons`, `core:tome_tools`, `core:tome_armor`, `core:tome_extraction`
  et `core:enchanted_book` sont des objets du jeu de base ; un livre enchanté est un objet à fiche (voir
  `player/item_data.rs`) comme les autres.
- **La bibliothèque** : un bloc avec `behavior: "core:enchant_library"`, `enchanting: (library_level: 10)` (le plus haut
  niveau rangé ; un niveau `n` vaut `2^(n-1)` points, et la sorte plafonne à `2^library_level` points) et
  `ports: (inputs: [0], outputs: [1])` : les réseaux d'objets y déposent des livres (case 0) et reprennent ce qu'elle
  refuse (case 1). Ses points sont dans les octets libres de l'entité du bloc.
- `charms.ron` : `(item: "mon_mod:charme_x", potion: "mon_mod:potion_x")`. L'objet doit avoir une `durability` (sa
  charge en secondes) ; la potion doit être dans `potions.ron`.

## Le Guide du voyageur

Le livre `core:guide` (dans le kit de départ, ou une planche et un bâton) s'ouvre d'un clic droit. Un mod
y ajoute ses chapitres avec un `guide.ron`, lus après ceux du jeu (voir `assets/core/guide.ron`) :

```ron
(chapters: [
    (id: "mon_mod:machines", title: "Mes machines", icon: "broyeur",
     pages: [
        (title: "Le broyeur", text: "Clic droit pour l'ouvrir. {touche:inventory} : l'inventaire.",
         items: ["broyeur", "core:iron_ingot"]),
     ]),
])
```

- `id` commence par l'espace de noms du mod ; `icon` et `items` sont des objets (sans espace de noms :
  celui du mod). Un objet inconnu empêche le chargement.
- Un clic sur un objet d'une page ouvre ses recettes dans l'explorateur.
- `{touche:action}` affiche la touche choisie par le joueur (`inventory`, `drop`, `backpack`, `tiers`,
  `sprint`, `jump`, `fly`…).

## Le livre de quêtes

Un mod y ajoute un chapitre avec un `quests.ron`, lu après celui du jeu (`assets/core/quests.ron`).
Le livre dit **quoi viser, pas comment** : chaque quête demande d'avoir ou de faire quelque chose, avec
un court texte qui donne une piste. Rien ne se joue sans le fichier : un mod sans `quests.ron` n'a pas
de chapitre.

```ron
(
    // `settings` n'existe que dans le fichier de core.
    groups: [(id: "mon_mod:groupe", title: "Mon groupe", icon: "broyeur")],
    // Des tables de tirage, pour les récompenses `Loot` : `rolls` tirages, un choix par poids.
    loot: { "gouter": (rolls: (1, 2), entries: [(item: "core:bread", count: (1, 2), weight: 3), (item: "core:apple")]) },
    chapters: [(
        id: "mon_mod:broyage",
        group: "core:mods",                    // le groupe d'un autre espace de noms est permis
        title: "Broyer",
        icon: "broyeur",
        progression: Flexible,                 // facultatif : Flexible ou Linear pour tout le chapitre
        quests: [
            (
                id: "mon_mod:broyage/broyeur",
                title: "Un broyeur",
                subtitle: "Pour commencer",    // facultatif
                text: "Une phrase ou deux. {touche:inventory} devient la touche choisie.",
                x: 0.0, y: 0.0, shape: Hexagon, size: 2.0,   // le plan : une case de grille chacune
                after: ["core:first_steps/pickaxe"],        // dépendances (autre espace permis)
                requires: All,                              // All (défaut) ou One
                tasks: [Item(item: "broyeur")],
                rewards: [Item(item: "core:bread", count: 2), Xp(10), Loot("gouter")],
                guide: "mon_mod:machines",                  // un chapitre du guide (facultatif)
            ),
        ],
    )],
)
```

- **Identifiants** : `espace:nom` ; un mod n'ajoute que des groupes, chapitres et quêtes de son espace
  de noms. Un objet, bloc, créature, biome, dimension ou structure sans espace de noms prend celui du
  mod. `#logs` désigne un groupe d'objets de `items.ron`.
- **Une quête** : `id`, `title`, `subtitle`, `text`, `x`, `y`, `shape` (`Circle`, `Square`, `RoundSquare`,
  `Hexagon`, `Diamond`, `Pentagon`), `size`, `icon` (par défaut l'objet de la première tâche), `after`,
  `requires`, `optional` (ne compte pas dans le pourcentage du chapitre), `hidden` (cachée tant que ses
  dépendances ne sont pas finies), `secret` (cachée jusqu'à ce qu'elle soit finie), `progression`,
  `tasks`, `rewards`, `autoclaim` (récompenses données dès la fin, sauf s'il y a un choix), `guide`.
- **Progression** : `Flexible` (défaut) : les compteurs avancent toujours, la quête se termine quand ses
  dépendances le sont ; `Linear` : rien n'avance tant que les dépendances ne sont pas finies. Elle se règle
  par quête, par chapitre, ou dans `settings` du fichier de core.
- **Tâches** : `Item(item, count, consume, crafted)` (avoir `count` objets sur soi ; `consume: true` : on
  les remet à la main ; `crafted: true` : seuls comptent ceux qu'on fabrique), `Check` (une case à
  cocher), `Break(block, count)`, `Place(block, count)`, `Kill(creature, count)` (sans `creature` :
  n'importe quelle hostile, tuée à moins de 16 blocs du joueur), `Dimension(id)`, `Biome(id)`,
  `Structure(id)`, `Depth(y)` (descendre sous cette hauteur), `Level(n)`, `Tier(n)`, `Signal("nom")` (le script du mod l'envoie, voir plus bas). Un compteur ne
  redescend jamais : jeter l'objet ensuite ne défait rien. `Break` et `Place` ignorent les états du bloc (`core:stairs[facing=west]` compte comme `core:stairs`).
- **Signaux** : `Signal("nom")` (ou `Signal("monmod:nom")`) se remplit quand le script du mod appelle
  `quest_signal("nom")`. Le jeu ajoute **toujours** l'espace de noms du mod appelant : un mod ne remplit
  que ses propres signaux, et le fichier ne peut déclarer que `Signal("<son mod>:...")` (sinon le chargement
  refuse la quête, avec un message clair). Le nom : minuscules, chiffres, `_`, `-`, `.`, `/`, 64 octets au
  plus. Un signal reçu quand la quête est verrouillée suit le mode de progression (`Flexible` : mémorisé ;
  `Linear` : ignoré), comme `Break` ou `Kill`. Pour remplacer une case `Check` « je l'ai fait » par une
  vérification du script : il suffit d'appeler `quest_signal` au bon moment (magie : un sort lancé).
  Dans l'autre sens, l'événement `QuestCompleted` (identifiant de la quête, une seule fois par quête) et
  l'appel `quest_done("mod:chapitre/quête")` permettent à un script de réagir au livre.
- **Récompenses** : `Item(item, count)`, `Xp(points)`, `Loot("table")`, `Choice([(item: ..., count: ...), ...])`
  (le joueur choisit un objet). Elles se récupèrent à la main et ne sont données **qu'une fois**.
- **Vérifications au chargement** (une erreur claire, comme pour `guide.ron`) : objet, groupe d'objets, bloc,
  créature, biome, dimension, structure, palier, table de tirage ou chapitre du guide inconnus ;
  identifiants en double ou hors de l'espace de noms ; dépendance inconnue ou en boucle ; chapitre
  vide ; quête sans tâche. Une dépendance vers un mod **absent** (désactivé) est ignorée avec un
  avertissement dans le journal. Un `quests.ron` de mod invalide est laissé de côté sans empêcher de jouer.
- **Le plan** : `x` vers la droite, `y` vers le bas, une case de grille = 40 points au zoom 1 ; une quête
  de taille 1 occupe une case. Garde au moins **2 cases** entre deux quêtes voisines (3 pour une quête de
  taille 2) : le titre s'écrit en dessous, sur deux lignes. L'écran (touche L, ou le bouton « Quêtes » de
  l'inventaire) cadre le plan tout seul ; une quête `hidden` ou `secret` n'est ni dessinée ni reliée tant
  qu'elle est cachée ; une dépendance d'un autre chapitre n'a pas de trait, mais la fiche la propose
  en bouton cliquable. Une quête qui a un `guide` offre « Lire dans le guide ».
- **La progression** est dans `quests.ron` à la racine du monde. Les identifiants inconnus (un mod
  retiré) y sont conservés : la progression revient avec le mod.

## Les dimensions

Une dimension est un monde parallèle, décrit dans le `dimensions.ron` d'un mod : son générateur, son
ciel, sa gravité. Chacune est **sauvegardée à part** (`saves/<monde>/dimensions/<nom>/`), et le joueur,
lui, est un seul : inventaire, vie, faim et expérience le suivent d'une dimension à l'autre.

```ron
[(id: "mon_mod:fonderie", name: "La Fonderie",
  generator: Terrain(biomes: ["core:desert"]), seed_offset: 777,
  sky: (color: (0.35, 0.12, 0.08), daylight: 0.55),
  gravity: 0.6,
  arrival: "mon_mod:portail_retour")]
```

| Champ | Rôle |
|---|---|
| `generator: Terrain(biomes: [...])` | le terrain du jeu (collines, océans, grottes, structures), limité à ces biomes de terre (vide : tous) |
| `generator: Flat(layers: [("core:stone", 40), ...], biome: ...)` | un monde plat : des couches de bas en haut depuis y = 0, 200 blocs au plus |
| `generator: Void(platform: "core:stone")` | le vide, avec (facultatif) une plateforme de 17 blocs de côté à y = 64 autour de l'origine |
| `seed_offset` | ajouté à la graine du monde : une autre carte dans cette dimension |
| `sky` | `color` : le ciel et le brouillard ; `daylight` : une lumière de 0 à 1, fixe (pas de cycle jour/nuit). Absents : le ciel habituel |
| `gravity` | un multiple de la gravité normale (0,05 à 4) ; pour le joueur et les créatures |
| `arrival` | le bloc posé à deux blocs du joueur à son arrivée : en général le portail de retour |

**Les portails** : un bloc dont `blocks.ron` dit `portal: "mon_mod:fonderie"` (ou `"core:overworld"` pour
revenir) emmène le joueur qui s'y tient 1,5 seconde. Il n'a pas besoin d'être solide (`solid: false`) ; on
peut le dessiner en boîtes translucides. Après une arrivée il faut sortir d'un portail avant d'en reprendre
un. Un script voyage aussi : `travel_to("mon_mod:fonderie")` (par exemple depuis un objet ou une commande)
et `dimension()` donne l'identifiant de celle où l'on est ; la console a `/travel <dimension>`.

**Passer une dimension, c'est recharger le monde** : le jeu sauvegarde, note la destination dans
`world.ron`, puis relance le monde par le chemin du menu ; le joueur arrive au sol aux mêmes x et z. Les
créatures, objets au sol et scripts repartent à neuf : ce qui doit rester va dans `set_world_value`
(les données du mod, propres au monde, se retrouvent). Les dimensions demandent un monde sauvegardé (pas
le monde jetable). Si le mod d'une dimension est retiré, le joueur qui s'y trouvait est ramené à la
dimension de départ. Une dimension peut aussi avoir ses propres minerais et structures (`ores.ron`,
`structures.ron` du mod s'appliquent à toute dimension de type `Terrain`).

## Le joueur, les objets utilisables et l'écran

Un mod agit sur le joueur et sur l'écran sans toucher au moteur :

| Fonction | Rôle |
|---|---|
| `player()` | l'état du joueur : ses yeux, son regard (longueur 1), sa vie, sa vie max et son niveau d'expérience ; rafraîchi à chaque pas de simulation |
| `player_target(portée)` | le point où le regard touche un bloc, à moins de `portée` blocs (sans repasser par le jeu : c'est un rayon dans le monde) |
| `heal_player(vie)` | rend de la vie |
| `hurt_player(dégâts)` | blesse le joueur (l'armure et l'invulnérabilité après un coup comptent) : un piège |
| `teleport_player(x, y, z)` | téléporte le joueur (ses pieds), à 64 blocs au plus, vers un endroit chargé |
| `give_effect("speed", secondes, puissance)` | un effet : `speed`, `strength`, `regeneration`, `fire_resistance`, `healing` |
| `hurt_area(centre, rayon, dégâts)` | blesse les créatures autour d'un point (16 blocs de rayon au plus) ; elles reculent |
| `consume_held(n)` | retire `n` exemplaires de l'objet tenu en main |
| `say(texte)` | une ligne dans la console du joueur |
| `hud_bar(id, fraction, couleur, texte)` / `hud_clear(id)` | une jauge à l'écran, au-dessus de la barre d'objets (quatre au plus par mod) |

**Les objets utilisables** : `usable: true` dans `items.ron` (une baguette, un grimoire, un outil de mod). Un
clic droit avec cet objet en main, quand le jeu n'a rien d'autre à faire du clic (poser un bloc, ouvrir
un coffre, manger...), prévient les scripts abonnés : `Event::ItemUsed { item }`. Le script décide de ce
qui se passe, et de consommer l'objet ou non.

**Un objet qui pose un bloc à part** : `places: "mon_bloc"` dans `items.ron`. L'objet garde son icône et
sa pile, et pose ce bloc (déclaré avec `placeable: false`, pour qu'il n'ait pas d'objet à lui). Le jeu s'en
sert pour les sacs à dos posés : Maj + clic droit pose le sac, l'entité du bloc retient le numéro du
sac, et le bloc cassé rend le même sac (voir `games/voxel_sim/src/storage/backpack_block.rs`). Un `places` qui nomme
un bloc inconnu empêche le chargement.

Le mod **Magie** (`mods/magie`) est bâti uniquement avec cela : un **mana** qui se régénère (1 par
seconde, réglable dans sa page d'options) avec une jauge à l'écran et un maximum qui monte avec le niveau,
une **table des runes** (menu à trois cases) où l'on compose un sort (une rune d'effet : flamme, soin ou
saut ; des modificateurs : puissance, portée), une **baguette** qui lance le sort inscrit sur un clic
droit, un **cristal de mana**, les commandes `/mana` et `/sort`. Ses tests sans fenêtre vérifient la
composition, chaque sort, le mana insuffisant et la jauge.

## Les coûts mesurés

`cargo bench -p engine_mods --bench wasm`, sur le Mac de développement (Apple Silicon) :

| Mesure | Temps |
|---|---|
| un appel du jeu vers le mod (`on_event` vide) | 134 ns |
| un appel du mod vers le jeu (`get_block`) | 6 ns |
| 10 000 horloges natives (un tick) | 1,07 ms |
| 10 000 horloges en WebAssembly, réveillées par lot | 1,39 ms (+30 %) |
| les mêmes, un appel par machine | 1,91 ms |
| les mêmes, en un seul appel | 0,55 ms |

À retenir : passer du jeu au script coûte cher (le plein de carburant, la copie des
données) ; l'inverse est presque gratuit. D'où les appels **par lot** : un mod qui fait
tourner 10 000 machines ne coûte que 30 % de plus que du code natif.

## Reproduire un mod Minecraft

La démarche pour « reverse engineer » un mod (Create, Mekanism...) : observer ce qu'il fait en
jeu, puis le décomposer avec les briques ci-dessus :
1. **ses blocs** et leurs propriétés → `blocks.ron` ;
2. **ses machines** : ce qui les réveille (tick régulier ? voisin ? impulsion ?), leur état
   (→ `data` ou des capacités) ;
3. **ce qu'elles échangent** : énergie, fluides, objets, rotation → les capacités, par face ;
4. **ses événements** : un butin à la mort d'une créature, un effet quand on casse un bloc.

Le casseur du mod d'exemple suit ce schéma : il réagit à une impulsion (comme un distributeur),
range son orientation et son état dans `data`, casse le bloc devant lui et donne l'objet.

## Packs graphiques optionnels

Le menu principal permet de choisir entre les packs 16 × 16, 32 × 32 et 64 × 64
avant l'ouverture d'un monde. La sélection est une préférence
visuelle du joueur, indépendante des sauvegardes. Voir
[`texture_packs/README.md`](../texture_packs/README.md).

Un remplacement graphique réside dans
`texture_packs/earth-copper-32/assets/<namespace>/textures/<nom>.png` (ou
`earth-copper-64` pour le pack 64) ou
`.../items/<nom>.png`. Les noms restent ceux des assets du mod ; aucun nouveau
champ n'est requis dans `blocks.ron`. Les PNG absents reviennent aux images du mod.
Le tableau de textures GPU utilise la résolution du pack choisi : une texture
16 × 16 non remplacée est agrandie au plus proche voisin dans les packs 32 et 64.
Le pack ne remplace pas les scripts, sons, recettes ou définitions de blocs.

## Packs de shaders

Le menu principal charge les packs WGSL de `shader_packs/`, à la racine du jeu
(à côté de `texture_packs/`). Voir [la référence des packs de shaders](shader-packs.md)
pour le manifeste et les fonctions qui personnalisent le terrain, l'eau et le ciel.
