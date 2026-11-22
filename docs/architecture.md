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

### Réseaux (phase 9c, fait : `tech/networks.rs`, `tech.rs`) : câbles, tuyaux, arbres de transmission

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

**Ce qui est fait.** Un bloc dont le comportement est `core:cable` (énergie), `core:pipe`
(fluides et gaz) ou `core:shaft` (rotation) est un conduit ; un mod l'utilise dans son propre
`blocks.ron`. Les machines se branchent en ayant une capacité (`EnergyStorage`, `FluidTank`,
`Kinetic`) dont les `sides` incluent la face tournée vers le conduit. Leur rôle (`Io::In`,
`Out` ou `Both`) est rangé dans deux bits libres des `Sides` : aucun format de sauvegarde n'a
changé.

- Le recalcul est incrémental : poser, casser ou changer un voisin marque le réseau « sale »,
  reconstruit au tick suivant par un parcours en largeur (fusion et coupure comprises).
- Le bilan d'énergie et de fluide est un seul passage par réseau : producteurs vers
  consommateurs (au prorata), surplus vers les tampons, tampons vers les consommateurs en
  manque, puis équilibrage des tampons. Pas de latence : 3 câbles ou 300, même tick.
- La rotation : la vitesse de la source la plus rapide, partagée si la contrainte demandée
  ne dépasse pas la contrainte fournie ; sinon (ou si deux sources tournent en sens
  contraire) le réseau est bloqué et tout s'arrête.
- Machines de base (`tech.rs`) : panneau solaire, batterie, lampe électrique, pompe, réservoir,
  remplisseur, moteur électrique, ventilateur.

Mesures (`cargo bench -p engine_voxel --bench machines -- réseaux`, Mac, release) :

| Réseau | Construction | Tick (régime établi) | Casser un câble au milieu, puis le remettre |
|---|---|---|---|
| 1 000 câbles | 0,2 ms | 0,5 µs | 0,22 ms |
| 10 000 câbles | 1,8 ms | 1,7 µs | 2,8 ms |
| 50 000 câbles | 9,7 ms | 6,9 µs | 15 ms |

Pas encore fait : les engrenages (rapports de vitesse entre deux arbres) et la répartition des
réseaux sur plusieurs cœurs (ils sont indépendants : c'est possible, mais un tick coûte déjà
quelques microsecondes).

### Terrain lointain (fait : `worldgen/far.rs`)

Les chunks détaillés (grottes, minerais, arbres, lumière) sont chers : 3 000 à 32 chunks de
rayon, plus de 50 000 à 128. Très loin, on ne voit que la forme du terrain. Le **terrain
lointain** ne regarde donc que la surface : pour une grille de points, la hauteur et le biome que
le générateur donne déjà sans rien creuser, maillés en tuiles de 32 × 32 cellules, colorées selon
le biome, l'altitude et la pente, avec de l'eau à la hauteur de la mer.

- **Niveaux de détail** : cinq niveaux, une cellule fait 2, 4, 8, 16 ou 32 blocs. Chaque tuile
  coûte la même chose (1 089 sommets) : l'arbre quaternaire (`select_tiles`) prend des tuiles
  fines près du monde détaillé et de plus en plus grandes avec la distance. À 128 chunks de
  rayon, il faut environ 500 tuiles.
- **Aucun trou, aucun recouvrement** : le décor commence exactement là où les chunks détaillés
  s'arrêtent, par des tuiles fines au grain des chunks ; des jupes verticales cachent les fentes
  entre deux niveaux. Une région de 32 chunks ne change de niveau que d'un coup, quand toutes ses
  nouvelles tuiles sont prêtes.
- **Même format que les chunks** : les sommets font 24 octets et le shader multiplie leurs
  positions par l'échelle de la tuile (`Renderer::add_mesh_scaled`).
- **En tâche de fond** : deux threads construisent les tuiles (`FarStreamer`), sans toucher au
  monde ; le thread principal n'envoie au GPU que ce qui est prêt.

Mesures (`cargo xtask bench --only foret`, Mac, release) :

| Réglage | Chunks détaillés | FPS (1 % le plus lent) | CPU | GPU |
|---|---|---|---|---|
| rayon 16, pas de terrain lointain | 529 | 120 (82) | 1,26 ms | 3,8 ms |
| rayon 16 + terrain lointain à 128 chunks (4 km) | 703 | 120 (91) | 1,25 ms | 3,0 ms |
| rayon 64 détaillé, sans terrain lointain | 6 275 | 39 (19) | 15 ms | 20 ms |

Voir 4 km de paysage ne coûte donc pas de temps mesurable, alors que 64 chunks détaillés
divisent les FPS par trois et ne finissent pas de charger en 90 s. Limites : pas d'arbres ni de
villages au loin (le décor y est lissé), et le décor lointain ne montre pas les modifications du
joueur.

### Fluides dans le monde (fait : `fluids/sim.rs`)

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

### Structures mobiles (phase 9d, fait : `motion/subgrid.rs`, `motion/ship.rs`, `games/voxel_sim/src/vehicles/ships.rs`) : contraptions et vaisseaux

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

**Ce qui est fait, et ce qui diffère du plan.**

- **Extraction** (`extract`) : un parcours à partir d'un bloc (l'assembleur) rassemble tout ce qui
  lui est relié par une face, sauf le terrain (pierre, terre, sable, eau…) ; les blocs et leurs
  états sont copiés dans une `SubGrid` (un vrai `World` local) et retirés du monde. Au plus 4 096 blocs.
- **Transformation** : une position (celle du pivot, au centre de masse) et un **lacet** : la
  rotation est limitée à l'axe vertical (une structure ne se retourne pas). Les machines, les
  ticks programmés et les réseaux de la phase 9c tournent en coordonnées locales (`Grids::with_subgrids`).
- **Rendu** : chaque chunk local est maillé par le mailleur du monde ; le shader reçoit le lacet
  avec l'origine du maillage, et les textures restent accrochées au repère de la structure.
- **Physique** : au lieu de `rapier3d`, un petit corps rigide à quatre degrés de liberté (la
  position et le lacet) : gravité, flottaison (chaque bloc immergé déplace 1,6), ballons,
  hélices (poussée proportionnelle à la rotation reçue, et couple si elles sont décentrées),
  barre, frottements, collisions axe par axe avec le terrain. Rapier ne servirait qu'à des
  structures qui tanguent, roulent ou se heurtent entre elles.
- **Le joueur porté** : une entité qui se pose sur un pont reçoit `OnGrid` ; sa physique
  (`PlayerBody`) tourne alors dans le repère de la structure, donc elle marche, saute et tombe
  normalement sur un pont qui avance et tourne, et la caméra tourne avec lui.
- **Sauvegarde** : `grids.bin` dans le dossier du monde (blocs par nom, états, transformation,
  vitesses).
- **Réassemblage** (`land`) : la structure se repose alignée sur la grille (rotation arrondie au
  quart de tour, orientation des pistons, répéteurs et hélices tournée avec elle) si la place
  est libre ; sinon elle refuse et rien ne change.

Limites : une structure assemblée ne se modifie pas (on la repose d'abord) ; sa lumière est
celle du plein ciel (pas d'ombres) ; pas de collisions entre structures ; pas de fluides dans
une structure.

### Wagonnets et rails (fait : `motion/rails.rs`, `motion/rail_motion.rs`, `motion/rail_save.rs`, `games/voxel_sim/src/vehicles/carts.rs`)

Les rails sont des **blocs actifs** (`rail: Normal | Booster | Detector | Activator | Stop` dans
`blocks.ron`, un mod peut donc ajouter les siens) ; les wagons sont des **entités ECS** : ce qui est
fixé dans la grille est un bloc, ce qui roule librement est une entité.

- **Forme d'un rail** (`motion/rails.rs`) : dix formes (deux droites, quatre montées, quatre virages),
  calculées depuis les rails voisins à la pose et au changement d'un voisin, comme les raccords des
  conduits, puis rangées dans `BlockEntity::data` (`data[0]` : la forme, `data[1]` : alimenté).
  Aucune variante de bloc n'est sauvegardée. Un voisin n'est relié que s'il accepte le lien (il
  pointe déjà vers le rail, ou une de ses extrémités ne mène nulle part) : deux voies parallèles ne
  se tordent pas l'une vers l'autre. Avec trois liens, c'est un aiguillage : droit sans signal,
  virage avec. Le chemin dans le bloc (segment ou quart de cercle), sa tangente et la sortie vers le
  bloc suivant (une marche plus haut ou plus bas) sont des fonctions pures et testées.
- **Rendu** : `Shape::Rail`, un panneau au ras du sol dont le mailleur choisit la texture (droite ou
  virage) et le nombre de quarts de tour (`FLAG_TURN_SHIFT`, lu par le shader) ; la montée est
  inclinée. Sélection : une dalle très fine ; le joueur le traverse (`solid: false`).
- **Signal** : l'accélérateur, l'activateur, le butoir et l'aiguillage lisent `received_power` ; le
  détecteur émet 15 tant qu'un wagon roule dessus (`Machines::set_power`, appelé par le jeu).
- **Physique** (`motion/rail_motion.rs`) : une abscisse et une vitesse signée le long du chemin du rail
  courant, en sous-pas de 0,5 bloc au plus (aucun rail sauté, même à 30 blocs par seconde) ;
  gravité dans les pentes, frottement, accélérateur alimenté (pousse) ou non (retient), butoir
  (arrêt au milieu du bloc), vitesse maximale réglable (8 par défaut, 30 au plus). Au bout de la
  voie le wagon déraille : le jeu reprend avec une boîte (`PlayerBody`) et un fort frottement, et un
  wagon libre au-dessus d'un rail le reprend.
- **Train** : `couple` garde deux wagons à une distance donnée (ils avancent sur leur chemin et
  partagent leur vitesse : la locomotive tire le train) ; `bump` repousse deux wagons qui se
  touchent.
- **Jeu** (`vehicles/carts.rs`, `vehicles/cart_play.rs`) : quatre sortes (wagonnet, wagon coffre, wagon trémie,
  locomotive), passager (`Riding`, la caméra suit), créatures qui montent dans un wagonnet vide,
  attelage (objet : un clic sur un wagon puis sur un autre), inventaire en écran, dessin en boîtes
  (`render/entities.rs`, depuis la réplique des entités du client). Le chargeur et le déchargeur (blocs avec `Inventory`,
  branchables aux tuyaux) remplissent ou vident un wagon coffre ou trémie arrêté sur un rail
  voisin et émettent un signal quand ils ont fini.
- **Sauvegarde** : `carts.bin` dans le dossier de la dimension, à côté de `grids.bin` (sorte,
  position, rail et vitesse, inventaire, combustible, attelages). Un wagon dont le terrain n'est pas
  chargé attend, figé.

Limites : pas de signal qui se propage de rail accélérateur en rail accélérateur (chacun lit ses
voisins), pas de détecteur qui change d'apparence, un wagon poussé par un autre peut traverser un
butoir (les corrections d'attelage et de chocs ne tiennent pas compte du butoir).

### Stockage : coffres à paliers, contrôleur, sacs à dos (fait : `tech/storage.rs`, `games/voxel_sim/src/storage/`, `games/voxel/src/storage/`, `games/voxel/src/ui/chest_preview.rs`)

Plan et choix : `docs/plan-stockage.md`.

- **Disposition des cases** (`engine_voxel::Layout`). Un coffre à paliers garde tout dans
  l'`Inventory` de son entité de bloc, sans champ de plus (le format des chunks ne change pas), rangé
  en trois blocs : les **cases principales** (les seules que voient les tuyaux d'objets), les
  **emplacements d'amélioration** (un objet « amélioration » par case, juste après), puis les cases
  **filtre** (fantômes : un exemplaire pour désigner un objet, rien n'est consommé, et elles ne
  tombent jamais quand le bloc casse). Paliers : coffre 27 cases / 1 amélioration, cuivre 45 / 2, fer
  54 / 3, or 81 / 4, saphir 108 / 5. Monter de palier (`upgrade_chest`, par un kit en clic droit)
  change le bloc et garde l'entité : seul le début des deux derniers blocs se déplace (`relayout`). Un
  coffre posé avant les paliers (27 cases) se complète au chargement (`ensure_layout`).
- **Améliorations** (`Upgrade`) : pile ×2 et ×4 (×8 au plus, dans `Policy`), compactage (lu dans les
  recettes 3 × 3 réversibles : `CompactTable`), néant (filtre, ou surplus sans filtre), entonnoir
  (une pile par seconde du dessus vers le dessous, ports respectés). Les passives se rejouent chaque
  seconde par `StorageChest` (tant qu'il y en a, jamais au repos) et tout de suite après un geste de
  l'interface.
- **Réseaux d'objets** (`tech/networks.rs`) : un coffre à paliers n'offre que ses cases principales, avec
  les piles de ses améliorations et son néant ; le contrôleur est un bloc à `Inventory` sans case qui
  dépose dans les coffres reliés. Une pile garde son `damage` en passant par un tuyau.
- **Contrôleur** : `controller_members` suit les coffres qui se touchent (64 au plus, rayon 16),
  `controller_groups`/`controller_insert`/`controller_take` les voient comme un seul inventaire.
- **Sacs à dos** (`storage/backpacks.rs`, côté jeu) : l'objet ne contient rien, son `damage` est un numéro
  (1 à 65 535, les bits hauts sont ceux de l'enchantement). Le contenu est dans le
  `BackpackStore`, sauvegardé dans `backpacks.bin` à la racine du monde (postcard, `version`, `next`,
  `(numéro, palier, cases, améliorations coupées)`), à côté de `world.ron` : pas par dimension. Un sac
  neuf reçoit son numéro au premier usage ; le palier suit l'objet (une montée par recette garde le
  numéro, le magasin range le contenu à la disposition du nouveau palier à la vue suivante). Un sac
  refuse les sacs.
- **Interface** (`storage/ui.rs`) : un seul écran pour coffres, wagons coffres et sacs (`Target`), avec
  les gestes de `storage/transfer.rs` (Déposer, Compléter, Tout prendre, Trier, Maj + clic : logique pure sur
  des listes de cases, avec les règles de chaque côté), la colonne des emplacements d'amélioration et
  les filtres. `ui/chest_preview.rs` regroupe le contenu pour l'aperçu de la fiche de visée.

### Génération : version, empreintes et cache des colonnes (fait : `worldgen/generation.rs`, `generator/column_cache.rs`, `worldgen/worldmap.rs`)

- **Version de génération** (`GenerationVersion`), distincte de la version du format de sauvegarde :
  `world.ron` écrit `generation` (version, configuration effective par dimension, empreintes des
  fichiers `biomes.ron`, `ores.ron`, `structures.ron` et `dimensions.ron` du jeu et des mods). Un
  monde sans ce champ est lu en **v1, la génération historique** ; une version inconnue est refusée.
  Les écarts de configuration (mod retiré, modifié ou ajouté) sont signalés dans les logs à
  l'ouverture ; la référence enregistrée n'est pas réécrite.
- **La génération v1 est figée** par `crates/engine_voxel/tests/generation_reference.rs` : des
  empreintes de blocs (graines 7, 42 et un corpus fixe, surface, sous-sol, bords, coordonnées
  négatives) qui ne changent jamais, dans l'ordre séquentiel, inverse, en parallèle, cache vide,
  plein, vidé ou évincé.
- **Cache des colonnes** : la hauteur, le biome et les températures d'une colonne (x, z) ne dépendent
  pas de l'altitude ; les six chunks d'une verticale partagent un instantané immuable (`Arc`),
  gardé dans un cache borné (1 024 colonnes, environ 17 Mio) à clés explicites (graine, dimension,
  version, coordonnées). Aucun verrou n'est tenu pendant le calcul, et deux workers qui demandent
  la même colonne ne la calculent qu'une fois. Les bruits de grottes ne sont calculés que pour les
  chunks qui ont des blocs sous le plafond des grottes, dans des tampons réemployés par thread.
- **Observation** : `ChunkStreamer::set_pipeline_timing` (`VOXEL_PIPELINE=1` en jeu) mesure les délais
  demande → génération → lumière → maillage → envoi ; `cargo xtask map` exporte des cartes PNG.

### Génération v3 : densité 3D, celle des nouveaux mondes (fait : `density_graph/`, `generator/v3.rs`, `generator/v3_rules.rs`, `assets/core/worldgen/`)

- `GenerationVersion::V3` (`FOR_NEW_WORLDS`) : le terrain est une densité 3D décrite en RON
  (`terrain_v3.ron` : paramètres climatiques, splines, bruits, grottes, volcans) et compilée en graphe
  (`density_graph`), calculée aux coins de cellules de 4 × 8 × 4 blocs et interpolée ; un monde de y = -128
  à 255. Biomes par cases de paramètres (`biomes_v3.ron`), règles de surface (`surface_v3.ron`), biomes
  souterrains en 3D, nappes par régions avec berges, structures posées sur le relief v3.
- Un monde enregistré garde sa version (`WorldMeta::generation_version()`) : v1 ou sans métadonnée -> v1,
  v3 -> v3. `VOXEL_GENERATION=1` crée encore un monde v1. La v2 (prototype de relief macro : géographie,
  drainage) a été retirée : ses mondes sont refusés avec une explication, et la liste des mondes le dit.
- Tests : `tests/generation_v3.rs`, `tests/far_v3.rs`, `tests/lifecycle_v3.rs`, `tests/density_graph.rs`.
  Observation : `cargo xtask map` (v3 par défaut, `--generator v1`), `--section z=N --from A --to B`
  (coupes), `cargo run --release -p engine_voxel --example recette`. Détails : `docs/plan-generation-v3.md`.

### Streaming : priorités et envois régulés (fait : `streaming.rs`, `streaming/transfer.rs`, `worldgen/far.rs`, `games/voxel/src/client/stream_upload.rs`)

- Le streamer complète des colonnes entières de la plus proche à la plus lointaine, avec un biais borné pour
  la direction de la caméra (`update_view`) ; ses maillages prêts attendent dans une table (le plus récent de
  chaque chunk, du plus proche au plus lointain) et le jeu les envoie sous un budget d'octets et de temps
  commun avec le terrain lointain (`TransferBudget`, `next_event_budgeted`) : le premier envoi de l'image passe
  toujours. Le terrain lointain reçoit une allocation de CPU d'après la file des chunks proches
  (`far_allowance`) et livre ses changements par lots de région. Mesures et limites : `docs/plan-generation-rendu.md`,
  `cargo run --release -p engine_voxel --example streaming_scenarios`.

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

## Le rangement des fichiers

Les crates du jeu et le moteur sont rangés **par domaine** (voir `docs/plan-rangement.md`) :

- la simulation (`games/voxel_sim/src`) et le jeu (`games/voxel/src`) ont les mêmes dossiers de domaine
  (`adventure/`, `enchanting/`, `quests/`, `storage/`, `vehicles/`, `shaping/`, `modding/`, `world/`) :
  les règles d'un côté, les écrans et la part fenêtre de l'autre, au même chemin ;
- s'y ajoutent des dossiers techniques : `server/`, `net/`, `player/`, `mobs/` dans la simulation ;
  `client/`, `render/`, `ui/`, `control/` dans le jeu ; `worldgen/`, `blocks/`, `tech/`, `motion/`,
  `fluids/`, `streaming/`, `mesh/` dans `engine_voxel` ;
- le fichier `x.rs` à côté du dossier `x/` porte le cœur du domaine et déclare ses sous-modules (pas de
  `mod.rs`, pas de réexportation) ; le nom d'un fichier ne répète ni son dossier ni son côté
  (`enchanting/table.rs`, pas `enchant_table_server.rs`) ;
- `cargo xtask check-arch` refuse un dossier de plus de 20 fichiers `.rs` (le fichier `x.rs` compte avec
  son dossier `x/`).

## Les performances, mesurées

Voir le README (section « Mesurer ») et `docs/modding.md` pour le coût du WebAssembly.
