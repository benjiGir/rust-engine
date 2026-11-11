# Écrire un mod

Un mod ajoute au jeu des **données** (blocs, fluides, créatures, textures, sons) et, s'il le
veut, un **script** : du code Rust compilé en WebAssembly, qui réagit aux événements du jeu et
fait fonctionner des machines. Le mod d'exemple (`mods/example`) montre tout ce qui suit.

## Le dossier d'un mod

```
mods/example/
├── mod.ron            # le manifeste (obligatoire)
├── blocks.ron         # ses blocs (facultatif)
├── fluids.ron         # ses fluides (facultatif)
├── entities.ron       # ses créatures (facultatif)
├── items.ron          # ses objets (facultatif)
├── machine_recipes.ron # les recettes de ses machines (facultatif)
├── textures/          # PNG 16×16, nommés comme dans blocks.ron
├── sounds/            # WAV
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

```ron
(id: "example:ruby_stairs", name: "Escalier de rubis", color: (0.7, 0.05, 0.12), texture: "ruby",
 boxes: [(from: (0, 0, 0), to: (16, 8, 16)), (from: (0, 8, 0), to: (16, 16, 8))],
 states: [Facing4, Half]),
```

`states` fabrique **une variante par combinaison** (ici 4 × 2 = 8). Chaque variante est un bloc
du registre, avec son identifiant complet (`example:ruby_stairs[facing=east,half=top]`) ; la base
(`example:ruby_stairs`) est la première. La forme écrite est celle de la **première valeur** : le
jeu en déduit les autres par rotation. Au plus 64 variantes par bloc.

| État | Valeurs | À la pose |
|---|---|---|
| `Facing4` | `north` (-Z, la forme écrite), `east`, `south`, `west` | l'avant regarde le joueur |
| `Facing6` | les quatre ci-dessus, `up`, `down` | idem, et vers le haut ou le bas si le joueur regarde fort dessus |
| `Half` | `bottom` (la forme écrite), `top` | en haut si on vise le dessous d'un bloc, ou la moitié haute d'une face |
| `Axis` | `y` (la forme écrite), `x`, `z` | l'axe de la face visée |

Poser et casser : seule la base est dans la barre de blocs et l'inventaire ; la variante est
choisie à la pose, et une variante cassée lâche l'objet (et le butin) de sa base. Un mod qui a
besoin d'un bloc précis le retrouve par son identifiant complet (`block_id("example:ruby_stairs[facing=east,half=top]")`),
et `block_base(bloc)` ramène une variante à sa base (par exemple dans l'événement `BlockPlaced`).

À savoir : un bloc à boîtes est « transparent » pour ses voisins (ils gardent leurs faces) ; sa
lumière est arrêtée par défaut s'il est `solid`, et passe sinon (`light_opacity` pour régler).
Les créatures le traitent encore comme un cube plein.

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
| `get_block(pos)`, `set_block(pos, bloc)` | lire, poser (les voisins sont prévenus) |
| `power(pos)`, `received_power(pos)`, `set_power(pos, 0..15)` | la puissance, comme la redstone |
| `data(pos)`, `set_data(pos, octets)` | l'état libre du bloc, sauvegardé avec le monde |
| `schedule_tick(pos, délai)` | se faire réveiller dans `délai` ticks |
| `give_item(bloc, nombre)` | donner des objets au joueur |
| `spawn_entity("core:pig", x, y, z)` | faire apparaître une créature |
| `random()` | un nombre pseudo-aléatoire |

### Objets et recettes de machine

Un script peut donner au joueur n'importe quel **objet**, pas seulement un bloc :
`item_id("example:ruby")` donne son numéro, `give_stack(objet, nombre)` le donne.

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
  fluides désignés par leur identifiant texte (`"core:water"`, `"example:ruby"`).

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
| `block_base(id: i32) -> i32` | Le bloc de base d'un bloc à états : `example:stairs[facing=east]` donne `example:stairs`. Un bloc ordinaire se renvoie lui-même. |
| `get_block(g: i32, x: i32, y: i32, z: i32) -> i32` | Le bloc à cette position (0 : de l'air, ou une grille inconnue). |
| `set_block(g: i32, x: i32, y: i32, z: i32, id: i32) -> i32` | Pose un bloc ; les voisins sont prévenus. 1 si c'est fait. |
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
| `kinetic_speed(g: i32, x: i32, y: i32, z: i32) -> f32` | La vitesse de rotation (tr/min, signée). |
| `set_kinetic(g: i32, x: i32, y: i32, z: i32, speed: f32, capacity: f32, impact: f32) -> i32` | Règle la vitesse, la capacité et la consommation de contrainte. 1 si c'est fait. |

*Objets et recettes*

| Fonction | Rôle |
|---|---|
| `item_id(ptr: i32, len: i32) -> i32` | Le numéro d'un objet (bloc ou non) à partir de son identifiant (`example:ruby`) ; −1 s'il n'existe pas. |
| `give_stack(id: i32, count: i32)` | Donne `count` objets de numéro `id` (voir `item_id`) au joueur. |
| `recipe_find(kind_ptr: i32, kind_len: i32, in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> i32` | Cherche une recette du type donné (`example:press`) faisable avec les objets `cle*nombre,cle*nombre` de l'entrée. Écrit `ticks;énergie;entrées;sorties` (même format) dans la sortie ; renvoie sa taille, −1 sans recette. |

<!-- /hooks:imports -->

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
