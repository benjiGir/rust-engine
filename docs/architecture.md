# Architecture : où va le moteur

Ce document décrit l'organisation actuelle et le **plan cible** pour les mods techniques
visés : Create (rotation, contraptions), Mekanism (énergie, fluides, machines), Create
Aeronautics et Valkyrien Skies (vaisseaux, structures qui volent). Les phases 9c (réseaux)
et 9d (structures mobiles) le réaliseront ; la phase 9b en a posé les fondations.

## Trois façons de représenter « une chose du jeu »

| | Exemples | Où | Combien | Coût au repos |
|---|---|---|---|---|
| **Entité ECS** | joueur, créatures, objets sur un tapis, projectiles | `engine_ecs::World` | milliers | parcourue à chaque pas |
| **Bloc actif** (block entity) | horloge, lampe, câble, machine, réservoir | dans son chunk | centaines de milliers | **nul** : réveillé seulement par un tick programmé ou un voisin |
| **Grille** | le monde, une contraption, un vaisseau | `GridId` → `BlockAccess` | dizaines | sa simulation physique |

Règle : ce qui bouge librement est une entité ; ce qui est fixé dans une grille est un bloc
actif. Un objet posé sur un tapis est une entité (il glisse) ; le tapis est un bloc actif.

## Ce qui existe (phase 9b)

### Les grilles et l'adressage

Un bloc se désigne par `GridPos { grid: GridId, pos: IVec3 }`, jamais par ses seules
coordonnées monde. `GridId::WORLD` est le monde. Tout le code des machines passe par :

- le trait `BlockAccess` (lire/écrire blocs et block entities d'une grille), implémenté par
  `ChunkStreamer` aujourd'hui, et par les sous-grilles demain ;
- `Grids`, qui retrouve la grille d'un `GridId` (aujourd'hui : le monde seulement).

Le `Scheduler` (ticks programmés, voisins à prévenir), les comportements (`BlockBehavior`),
l'interface des mods (chaque fonction prend `grid, x, y, z`) utilisent tous `GridPos`. Une
machine posée sur un vaisseau gardera donc ses coordonnées locales et continuera de tourner
pendant que le vaisseau vole.

### Les block entities et leurs capacités

`BlockEntity { power, data, caps: Option<Box<Capabilities>> }`, rangé dans le chunk
(table `index → état`), sauvegardé avec lui (format v2, postcard + lz4).

`Capabilities` : des conteneurs génériques, partagés par tous les mods :
- `EnergyStorage` : quantité, capacité, débits d'entrée et de sortie ;
- `FluidTank` (plusieurs par bloc) : fluide (identifiant texte), quantité et capacité en mB ;
- `Inventory` : des cases d'`ItemStack` (identifiant texte + nombre) ;
- `Kinetic` : vitesse de rotation signée (tr/min), capacité et consommation de contrainte
  (le « stress » de Create).

Chacun est exposé **par face** (`Sides`, 6 bits) et offre `insert`/`extract` avec un mode
simulation. Les identifiants sont des textes, pas des numéros : ils restent valables quand la
liste des mods change. Les capacités sont dans une boîte : un bloc qui n'en a pas ne paye que
8 octets.

### Les fluides

`assets/core/fluids.ron` (et `mods/<id>/fluids.ron`) décrit les fluides : couleur, viscosité
(ticks entre deux avancées), densité (négative : un gaz), température, lumière. Un bloc
référence le fluide qu'il représente dans le monde (`core:water` → `fluid: "water"`).

## Plan cible

### Réseaux (phase 9c) : câbles, tuyaux, arbres de transmission

Simuler chaque câble séparément (chacun passe son énergie au voisin) est lent et instable :
l'énergie met N ticks à traverser N câbles, et oscille. Mekanism et Create regroupent plutôt les
conduits en **réseaux** :

1. **Le graphe** : un réseau est une composante connexe de conduits du même type (énergie,
   un fluide, rotation), avec ses **extrémités** (les machines branchées et leur face).
2. **Recalcul incrémental** : poser un conduit fusionne au plus 6 réseaux voisins ; en casser
   un peut en couper un en plusieurs, à retrouver par un parcours en largeur limité à ce réseau.
   Rien n'est recalculé ailleurs. Les réseaux sont reconstruits au chargement des chunks
   (ils ne sont pas sauvegardés : les block entities le sont).
3. **Un tick par réseau** : à chaque tick de machines, chaque réseau fait le bilan de ses
   producteurs et consommateurs et répartit (énergie : au prorata de la demande ; fluides :
   par pression et viscosité ; rotation : la vitesse se propage avec les rapports
   d'engrenages, et le réseau se bloque si la contrainte demandée dépasse la capacité).
   Le contenu propre des conduits est mis en commun dans le réseau (un seul tampon).
4. **Parallélisme** : les réseaux sont indépendants entre eux. Leur simulation se répartit
   sur les cœurs (rayon), chacun ne touchant que ses propres données ; les écritures dans les
   machines sont appliquées ensuite, sur le thread principal.

### Fluides dans le monde (fait : `fluid_sim.rs`)

Le modèle retenu conserve le **volume** : chaque cellule a une quantité (4096 = un bloc),
qui tombe, s'étale vers l'équilibre et remonte sous pression (vases communicants). L'eau
générée (océans, lacs, rivières, nappes) est un **réservoir** stable : elle ne se vide pas
et ne coûte rien tant qu'on n'y touche pas. Seules les cellules actives sont calculées, dix
fois par seconde, dans un budget de 4 ms par étape (le reste attend l'étape suivante).

Mesures (`cargo bench -p engine_voxel --bench fluids`, Mac, release) :

| Scénario | Modèle à volume | Modèle à niveaux (genre) |
|---|---|---|
| Lac de 24 × 24 × 3 qui se vide par un trou de 2 × 2 | vidé à 95 % en 318 s de jeu, ~2,6 ms par étape (900 ns par cellule), endormi à 339 s | impossible : une source ne se vide pas |
| Grotte de 30 × 6 × 30 inondée par la mer | remplie à 95 % en 336 s de jeu, ~2,7 ms par étape | arrêtée au bout de 5 s : l'eau courante s'arrête à 7 blocs de la source, la grotte n'est jamais remplie |

Pourquoi garder le modèle à volume : il fait ce que le modèle à niveaux ne sait pas faire
(vider un lac, remplir une grotte, faire monter l'eau dans un tube en U), pour environ
2,5 % d'un cœur pendant un écoulement important, et rien au repos. Le budget par étape
borne le pire cas : une très grande inondation coule plus lentement, sans faire saccader le
jeu. Le modèle à niveaux reste dans le benchmark comme point de comparaison.

### Fluides : la suite (phase 9c)

L'eau qui coule : un **automate cellulaire** sur des **ticks programmés**. Un bloc de fluide
garde son niveau (dans les données du bloc) ; à son tick, il descend s'il le peut, sinon il
s'étale vers les voisins plus bas d'un niveau, puis reprogramme un tick après `viscosity` ticks
si quelque chose a changé. Seuls les fluides en mouvement travaillent ; un lac au repos ne
coûte rien, comme une machine qui dort. Une pompe ou un seau convertit entre bloc de fluide
(1000 mB) et réservoir.

### Structures mobiles (phase 9d) : contraptions et vaisseaux

1. **Extraction** : quand une structure se met en mouvement (un piston de Create, un vaisseau
   qui décolle), ses blocs et leurs block entities sont **retirés du monde** et copiés dans une
   **sous-grille** : ses propres chunks, en coordonnées locales, avec un nouveau `GridId`.
2. **Transformation** : la sous-grille a une position et une rotation (un quaternion) dans le
   monde. Ses coordonnées locales ne changent jamais : ses machines, son scheduler, ses
   réseaux continuent de fonctionner sans rien savoir du mouvement.
3. **Rendu** : la sous-grille a ses maillages (même mailleur que le monde), dessinés avec sa
   matrice de transformation.
4. **Physique** : un **corps rigide** `rapier3d` par structure. La masse et l'inertie sont
   calculées depuis les blocs (densité par bloc dans le registre). La collision utilise une
   forme composée (des boîtes fusionnées, comme le maillage glouton).
5. **Forces** : moteurs, hélices, ballons et propulseurs sont des blocs actifs avec une
   capacité « force » (direction locale, intensité) ; à chaque pas, on les somme en force et
   couple appliqués au corps rigide.
6. **Collisions** : grille ↔ monde (le corps contre les blocs voisins du monde, chargés autour
   de lui) et grille ↔ grille (deux corps rapier).
7. **Le joueur porté** : quand le joueur se tient sur une structure, sa position est exprimée
   dans le repère de la grille ; il bouge avec elle, et sa physique se fait en coordonnées
   locales.
8. **Sauvegarde** : chaque sous-grille est sauvegardée à part (ses chunks, sa transformation,
   ses vitesses) ; la réassembler dans le monde (atterrir) recopie ses blocs dans le monde
   et supprime la grille.

### Comment tout s'articule

```
                    ┌──────────────── Grids (GridId → BlockAccess) ─────────────────┐
                    │  WORLD : ChunkStreamer          vaisseau 7 : SubGrid + RigidBody │
                    │  chunks + block entities        chunks + block entities          │
                    └───────────▲──────────────────────────────▲──────────────────────┘
                                │ GridPos                       │ GridPos
   Machines (Scheduler, BlockBehavior natifs et scripts WASM) ─┘
   Réseaux (énergie, fluides, rotation : un graphe par réseau, par grille)
   ECS : joueur, créatures, objets en mouvement ; un composant `OnGrid(GridId)` pour ceux
         qui se tiennent sur une structure ; la physique rapier synchronise les grilles.
```

- Les **machines** ne connaissent que `GridPos` et `BlockAccess`.
- Les **réseaux** sont calculés par grille (un câble ne traverse pas le vide entre un vaisseau
  et le monde ; un connecteur explicite le pourra).
- L'**ECS** porte ce qui bouge librement, et la physique ; il référence les grilles par
  `GridId`.
- Les **mods** voient le tout à travers les mêmes concepts : positions de grille, capacités
  par face, événements.

## Les performances, mesurées

Voir le README (section « Mesurer ») et `docs/modding.md` pour le coût du WebAssembly.
