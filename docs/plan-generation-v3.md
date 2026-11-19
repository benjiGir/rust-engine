# Plan : génération v3, sur le modèle de Minecraft moderne (repris d'Horizon)

## Pourquoi

La recette du lot 9 a été jouée et rejetée. Les falaises v2 sont laides. Des rivières coulent sur des crêtes de roche
comme des aqueducs. Le monde est trop bas (192 blocs, mer à 62) pour de vraies variations de biomes et de grandes
grottes. Les performances, elles, sont bonnes (lots 6 à 8) : c'est la **génération** qu'il faut refaire.

La v2 inventait son propre modèle : géographie macro, graphe hydrologique, escarpements posés après coup. La v3 reprend
au contraire un modèle éprouvé, celui de Minecraft depuis 1.18, tel qu'Horizon le réimplémente en Rust. Le relief n'est
plus une hauteur par colonne retouchée : c'est une **densité 3D**. Les falaises, surplombs, vallées et grottes sortent
tous de cette densité, sans retouche.

## Ce qu'on reprend d'Horizon, et comment

Horizon (`horizon-rust/horizon`) n'a pas d'algorithme de terrain à lui. C'est un **interpréteur** des fichiers de
génération de Minecraft : graphe de fonctions de densité, réglages de bruit, règles de surface, aquifères. Il lit les
données dans le jeu Minecraft du joueur. On reprend son **architecture**, pas son code ni ses données :

- **Le code d'Horizon** est sous LGPL-3.0. Décision : on **réécrit** l'architecture dans notre moteur.
- **Les données de Mojang** (les JSON de `worldgen`) ne sont pas redistribuables. Nos réglages sont les nôtres, en RON,
  écrits et mesurés ici.
- **Biomes O' Plenty** est sous « tous droits réservés ». On ne prend ni code ni texture, seulement des **idées** de
  biomes (bayou, séquoias, champs de lavande, toundra, volcan, sources chaudes, gouffre cristallin...). Nos textures sont
  produites par `assetgen`.

Les concepts repris sont tous publics : bruits de Perlin à octaves (« normal noise »), fonctions de densité composées, splines de
continentalité, d'érosion et de crêtes, interpolation par cellules, grottes « fromage », « spaghetti » et « nouilles »,
aquifères locaux, biomes choisis par paramètres climatiques multiples, règles de surface. Horizon sert de référence de
lecture pour les cas limites : ordre d'évaluation, caches, bords de cellules, aquifères.

## Décisions prises

| Sujet | Choix |
| --- | --- |
| Reprise | Réécrire l'architecture Minecraft/Horizon dans le moteur, réglages en RON |
| Hauteur | 384 blocs : y de -128 à 255, 12 couches de chunks (-4 à 7) ; la mer reste à 62 |
| Biomes | Nouveaux biomes inspirés de BoP, par vagues, après le nouveau relief |
| Versions | Nouvelle version **v3** ; v2 reste ouvrable puis sera retirée ; v1 intacte |

**Pourquoi -128..255 et pas 0..383.** La mer reste à 62, comme le sont 134 usages de `SEA_LEVEL` (fluides, lointain,
météo, structures...). On gagne 190 blocs de sous-sol sous la mer (Minecraft : 127) et des sommets jusqu'à 255
(Minecraft : 320 avec une mer à 63, donc un rapport proche). Les mondes v1 ne changent pas : leur générateur ne produit
rien sous 0 ni au-dessus de 191, et le streamer ne demande que les couches de leur version.

## Lots

### Lot 0 : un monde plus haut (moteur, sans changer la génération)

- La plage verticale devient une donnée : `MIN_CHUNK_Y..MAX_CHUNK_Y` pour le stockage (-4..8). Chaque générateur
  déclare la sienne : v1 et v2 `0..6`, v3 `-4..8`. Les 41 usages de `WORLD_HEIGHT_CHUNKS` passent par elle :
  streamer, lumière, lumière du ciel depuis la plus haute couche, cartes, fluides, sous-grilles, sauvegarde, culling.
- Coordonnées négatives partout : sauvegarde (régions par x/z, y négatif dans la clé), maillage, raycast, physique
  (le vide sous le monde), apparition, lointain (origine verticale), machines.
- Streaming vertical par distance : on ne charge plus toute la colonne d'un coup. Les couches proches du joueur et de
  la surface passent d'abord ; les couches profondes ne viennent que si on y descend ou si une grotte y mène (culling
  des grottes).
- **Passage :** empreintes v1 et v2 identiques, mondes v1 sauvegardés relus à l'identique, tests au-dessus et en dessous
  de 0, banc v1 sans régression (chargement, mémoire, p99).

### Lot 1 : le moteur de densité (sans terrain final)

- Bruits : Perlin amélioré et « normal noise » à octaves (deux Perlin décalés, amplitudes par octave), graines dérivées
  par nom (comme `PositionalRandom` : déterminisme indépendant de l'ordre).
- Graphe de fonctions de densité décrit en RON : constantes, bruit, bruit décalé, add, mul, min, max, clamp, abs,
  square, cube, half/quarter negative, squeeze, gradient de y, spline cubique (points, dérivées, splines imbriquées),
  `interpolated`, `flat_cache`, `cache_2d`, `cache_once`, plage (`range_choice`), interpolation par cellule (4 × 8 × 4).
- Compilation du graphe en arbre de nœuds avec caches. Évaluation par chunk aux coins des cellules, puis interpolation
  trilinéaire.
- **Passage :** tests unitaires de chaque nœud (valeurs connues, bornes d'intervalle), déterminisme sous ordre et
  threads, coût mesuré par chunk (cible : du même ordre que la v1).

### Lot 2 : le relief v3 (routeur de bruit)

- Les cinq paramètres : continentalité, érosion, crêtes (weirdness) avec pics et vallées, température, humidité.
- Le décalage (`offset`), le facteur et les dentelures (`jaggedness`) en splines de ces paramètres. Une densité finale =
  pente verticale + décalage, multipliée par le facteur, plus le bruit 3D. Plateaux, falaises, surplombs et pics
  viennent de la densité, sans escarpement posé après coup.
- **Rivières** comme dans Minecraft : là où les crêtes passent par zéro, les vallées descendent **au niveau de la mer**.
  Une rivière ne peut donc pas être perchée. Elle disparaît avec l'érosion forte des montagnes. Les lacs viennent des
  aquifères et des cuvettes.
- Outils : cartes de chaque paramètre et coupes verticales (`worldmap`, `section`) adaptées à la densité.
- **Passage :** captures et coupes jugées par toi avant de continuer (le point qui a échoué avec v2), aucune rivière
  au-dessus de son voisinage, falaises lisibles en jeu.

### Lot 3 : grottes et aquifères

- Grottes « fromage » (grandes salles en bruit 3D), « spaghetti » (tunnels longs, deux bruits), « nouilles » (fins
  réseaux), piliers, avec une densité qui protège la surface (pas de trous partout).
- Aquifères : niveau d'eau local par zone, lave en profondeur, barrières entre zones, pas d'inondation du sous-sol par
  la mer.
- Ravins éventuels en « carver ». Couches profondes en pierre profonde (`deepstone`) avec mélange au passage.
- **Passage :** grandes cavernes visibles en coupe et en jeu, aucune grotte qui vide la mer, coût mesuré.

### Lot 4 : biomes multi-paramètres et règles de surface

- Choix du biome par plus proche voisin dans l'espace des paramètres (température, humidité, continentalité, érosion,
  crêtes, profondeur). Les biomes de grotte sont choisis par la profondeur.
- Règles de surface en RON : conditions (biome, au-dessus de l'eau, profondeur sous la surface, pente, bruit, hauteur)
  vers matériaux (herbe, terre, sable, gravier, bandes de terre cuite, neige, pierre).
- Biomes existants convertis en points de paramètres. Arbres, minerais par hauteur, végétation : les features actuelles
  rebranchées.
- **Passage :** transitions sans scintillement, biomes de montagne et de grotte présents, recette de paysage (outil
  `recette`) contre v1.

### Lot 5 : nouveaux biomes (inspirés de BoP), par vagues

- Vague 1 (surface tempérée et humide) : séquoias, bayou, champs de lavande, prairie, verger, forêt d'érables.
- Vague 2 (froid et altitude) : toundra, forêt de conifères enneigée, hautes terres, landes, crags.
- Vague 3 (chaud et volcanique) : volcan, plaines volcaniques, sources chaudes, garrigue, désert fleuri.
- Vague 4 (sous terre) : gouffre cristallin, grotte luminescente, grotte fongique.
- Chaque vague : blocs et textures (`assetgen`), arbres, données RON, captures.

### Lot 6 : lointain, recette et activation

- Le terrain lointain lit une hauteur de surface approchée de la densité (recherche verticale grossière, comme la
  surface « préliminaire » de Minecraft), plus la mer et les biomes.
- Recette : outil `recette`, banc (rayons 8, 16, 32), captures, partie jouée par toi. Activation de v3 pour les mondes
  neufs seulement après ton accord. Retrait de v2.

## Risques

- **Coût de la densité 3D** : plus cher qu'une hauteur par colonne. Réponses : interpolation par cellules, caches 2D,
  et le streaming vertical du lot 0.
- **Réglage artistique** : ce sont nos splines, pas celles de Mojang. Chaque lot de relief s'arrête sur des captures
  que tu valides.
- **Hauteur doublée** : plus de chunks par colonne (mémoire, lumière). Le lot 0 mesure avant d'aller plus loin.
- **Lumière du ciel** sur 12 couches : à vérifier, propagation et coût.

## Statut du lot 0 : un monde plus haut (fait)

### Ce qui change

- **Bornes de stockage** : `MIN_CHUNK_Y = -4` et `MAX_CHUNK_Y = 8` (y de -128 à 255). Chaque générateur déclare les couches
  qu'il remplit (`TerrainGenerator::chunk_range`) : v1 et v2 gardent `0..6` (rien ne change pour eux). `with_chunk_range`
  impose une plage (tests, en attendant la v3) ; la génération v1 n'étant pas définie sous 0 (elle déborde en debug),
  elle y met alors de la pierre creusée d'une salle fermée par chunk.
- **Le monde connaît ses couches** (`World::vertical`, `block_y_range`) : le streamer y met celles de son générateur. Sous
  la plus basse, la lumière est noire ; au-dessus de la plus haute, c'est le plein ciel.
- **Tout ce qui supposait 0..6 lit ces couches** : file de chargement et complétude des colonnes du streamer, lumière
  (zone de calcul de hauteur variable, origine verticale, application par couche), maillage (le vide sous le monde,
  maillage culling et binaire), fluides, plantes, sous-grilles, machines, culling des grottes, cartes et coupes de blocs,
  sauvegarde (`saved_heights` sur toutes les bornes), résumé du lointain (`EditedSurfaces`).
- **Maillage vertical par distance** (`StreamingConfig::mesh_depth`, 3 par défaut) : la lumière d'une colonne demande
  tous ses blocs, donc les chunks restent chargés en entier. En revanche, un chunk **sombre** (aucune lumière du ciel :
  roche, grotte fermée) n'est maillé que s'il est à moins de 3 couches sous le joueur, et il l'est quand le joueur descend.
  Tout ce qui voit le ciel (surface, montagnes, entrées de grottes) est toujours maillé.

### Tests (`tests/vertical_range.rs`)

- La plage suit le générateur (v1 : `0..6`), bornée au stockage ; le monde du streamer prend `-4..8`.
- Couches sous 0 générées, éclairées (la roche est noire, sous le monde aussi) ; un puits creusé depuis le haut du monde
  jusqu'à y = -60 reçoit la lumière du ciel par la mise à jour incrémentale.
- Sauvegarde d'un chunk en y négatif (stockage direct et streamer : une modification à y = -77 survit à la réouverture).
- En surface, la roche de la couche -4 n'est pas maillée ; en descendant à y = -100, elle l'est. Sans limite, tout l'est.

### Mesures

- **Coût d'un monde de 12 couches** (même générateur v1 avec couches imposées, `print_deep_world_cost`, release) : au
  rayon 10, chargement 0,61 s contre 0,34 s (×1,8), 3 476 chunks gardés en mémoire contre 1 360, et 1 140 maillages contre
  823 (2 091 sans `mesh_depth`). Le doublement vient de la génération et de la lumière des couches profondes ; la mémoire
  suit le nombre de chunks non vides.
- **Banc v1 à rayon 16 après le lot** : CPU médian 0,6 à 1,3 ms (avant : 0,9 à 1,6), p99 1,0 à 2,6 ms (avant : 2,5 à 3,6),
  chargement 1,7 à 2,3 s (premier trajet 10 s, démarrage compris), mêmes chunks dessinés. L'écran n'était pas plafonné
  ce jour-là : les FPS et le GPU ne sont pas comparables.
- Empreintes v1 et v2 inchangées ; tous les tests passent.

### Limites

- Les chunks profonds restent chargés (blocs et lumière) même non maillés : la mémoire d'un monde v3 sera environ le
  double de v1 au même rayon. Charger les couches profondes à la demande demanderait une lumière par chunk et non par
  colonne : à reconsidérer si la mémoire pose problème avec le vrai terrain v3.
- Un chunk sombre non maillé vu de très haut (fond d'océan très profond, grotte vue d'une falaise) peut manquer : il
  faut qu'il n'ait aucune lumière du ciel et soit à plus de 3 couches sous le joueur.
- Les captures automatiques n'attendent pas le terrain lointain : une vue lointaine peut le montrer incomplet (déjà le
  cas avant ce lot, vérifié sur le commit précédent).

## Statut du lot 1 : le moteur de densité (fait)

Module `engine_voxel::density_graph`, sans encore aucun terrain : il servira au relief v3 (lot 2).

### Ce qui existe

- **Bruits** (`density_graph/noise.rs`) : Perlin amélioré (gradients des 12 arêtes, lissage quintique, permutation et
  origine tirées de la graine), sommes d'octaves (fréquence doublée et poids divisé par deux à chaque octave, somme des
  poids 1), « normal noise » (deux sommes d'octaves, la seconde à l'échelle 1,0181, normalisées selon l'étendue des
  octaves). Chaque bruit est dérivé de la graine et de son **nom** : l'ordre de création ne change rien. Les coordonnées
  sont ramenées près de 0 (période de 2²⁵) pour garder la précision loin de l'origine.
- **Description RON** (`density_graph/def.rs`) : un fichier de bruits (`first_octave`, `amplitudes`) et de fonctions
  nommées. Nœuds : `Constant`, `Ref`, `Noise`, `ShiftedNoise`, `Add`, `Mul`, `Min`, `Max`, `Clamp`, `Abs`, `Square`,
  `Cube`, `HalfNegative`, `QuarterNegative`, `Squeeze`, `YClampedGradient`, `Spline` (points, pentes, splines imbriquées),
  `Interpolated`, `FlatCache`, `Cache2d`, `CacheOnce`, `RangeChoice`.
- **Compilation** (`density_graph/graph.rs`) : un tableau de nœuds ; une fonction nommée n'est compilée qu'une fois et
  partagée ; erreurs claires (bruit ou fonction inconnus, cycle, spline vide ou mal ordonnée, RON illisible).
- **Bornes d'intervalle** calculées à la compilation pour chaque nœud (sommes, produits, splines avec leur dépassement
  possible et leurs prolongements, choix de plage qui écarte une branche impossible) : toute valeur y tombe.
- **Évaluation** : `sample` (une position, exacte) et `fill_chunk` (32³ blocs). Les `Interpolated` sont calculés aux 9 ×
  5 × 9 coins des cellules de 4 × 8 × 4 blocs puis interpolés ; `FlatCache` une fois par carré de 4 × 4 blocs ; `Cache2d`
  par colonne ; `CacheOnce` par position. Splines : hermite par morceaux, prolongée en ligne droite.

### Tests (`noise.rs` : 5, `tests/density_graph.rs` : 9)

- Bruits : bornés, nuls aux nœuds de la grille, dépendants de la graine et du nom mais pas de l'ordre, centrés, échelle
  réglée par la première octave, précis à 30 millions de blocs.
- Chaque nœud contre sa valeur attendue ; gradient borné ; splines exactes à leurs points, droites au-delà, plates entre
  deux points égaux, imbriquées ; partage des fonctions nommées ; chaque erreur.
- Bornes respectées par 3 000 échantillons de chaque fonction d'un graphe proche d'un relief.
- `fill_chunk` égale `sample` partout sans interpolation ; avec interpolation : exacte aux coins, trilinéaire entre eux
  (y compris en coordonnées négatives et sous 0) ; pas de couture entre deux chunks voisins.
- Mêmes résultats dans tous les ordres, sur plusieurs threads, d'une compilation à l'autre ; une autre graine diffère.

### Coût (`print_fill_cost`, release, M3 Pro)

Graphe d'essai proche d'un relief (deux bruits, spline de continentalité, bruit déformé, caches) : **0,27 ms par chunk**
avec interpolation, contre 8,0 ms en calculant chaque bloc exactement (× 29). La génération v1 complète d'un chunk coûte
de l'ordre de la milliseconde : la densité tient dans le budget. Le vrai coût sera mesuré avec le relief du lot 2 (plus
de bruits, grottes au lot 3).

### Limites

- Pas encore de « blend » entre anciens et nouveaux chunks (inutile tant que v3 ne remplace pas un monde existant).
- `FlatCache` garde une table par remplissage : suffisant ici, à revoir si le profil le montre.

## Statut du lot 2 : le relief v3 (à valider sur captures)

### Ce qui existe

- **Version v3** (`GenerationVersion::V3`, numéro 3, monde de test par `VOXEL_GENERATION=3`) : couches de -4 à 7
  (y de -128 à 255), mer à 62. `FOR_NEW_WORLDS` reste v1.
- **Relief en densité** (`assets/core/worldgen/terrain_v3.ron`, nos valeurs) : continentalité, érosion et crêtes pliées en
  pics et vallées (pv), température et humidité, légèrement déformées par un bruit de décalage. Le décalage (hauteur
  moyenne) est une spline de la continentalité, puis de l'érosion, puis des pics et vallées ; le facteur et les
  dentelures aussi. Densité = 4 × quart-négatif((pente verticale + décalage + dentelures) × facteur) + bruit 3D ; le haut
  du monde se vide, le bas reste plein ; le tout interpolé par cellules.
- **Rivières en vallée** : le fond des vallées (pv = -1) est à -0,08 (y = 53), sous la mer ; l'eau ne remplit que ce qui
  est sous y = 62. Une rivière ne peut donc pas être perchée. Les vallées coupent aussi les massifs (canyons).
- **Contexte de colonne** (`generator/v3.rs`) : la densité aux 9 × 49 × 9 coins d'une colonne de chunks est calculée une
  fois (cache de 512 colonnes) ; les blocs et les hauteurs de surface en sont interpolés, donc les hauteurs collent aux
  blocs. Biome simple (océan, rivière, plage, montagne, pics selon hauteur et paramètres, sinon le climat) ; surface du
  biome, sous-couche, pierre, pierre profonde sous 0, socle à -128, neige au froid, glace à la surface de la mer gelée ;
  minerais, plantes et arbres existants.
- **Outils** : `cargo xtask map --generator v3` (altitude, biomes, cartes de blocs) et `--section` pour les coupes.

### Tests (`tests/generation_v3.rs`, 7)

Version et plage ; hauteurs égales au bloc plein le plus haut et jamais d'eau au-dessus de la mer (trois graines) ; au
moins 95 % des fonds de vallée de l'intérieur sous la mer, aucun au-dessus de 65 ; sous-sol plein de pierre profonde ;
océans profonds, plaines et sommets au-delà de 162 ; mêmes blocs dans tous les ordres, sur plusieurs threads, avec un
cache neuf ; un monde v3 chargé, éclairé et maillé par le vrai streamer, apparition sur la terre ferme.

### Coût (`print_v3_cost`, release)

3,9 ms par colonne de chunks en v3 (12 couches) contre 2,0 ms en v1 (6 couches) : le même ordre par chunk.

### Ce qui n'est pas fait (lots suivants)

- Grottes et aquifères (lot 3) : le sous-sol est plein ; pas de lacs au-dessus de la mer.
- Règles de surface et biomes multi-paramètres (lot 4) : la neige est posée par colonne froide, en taches sur les pentes.
- Structures (villages, ruines...) : pas encore posées en v3.
- Terrain lointain v3 (lot 6) : une tuile lointaine ne tient qu'environ 110 blocs de dénivelé au niveau le plus fin ; les
  massifs v3 dépassent, d'où des artefacts à l'horizon.

## Statut du lot 3 : grottes et aquifères (à valider sur coupes)

### Grottes (dans la densité, `terrain_v3.ron`)

- **Salles** (« fromage ») : un bruit 3D aplati verticalement, rangé en étages par un bruit de couche ; **tunnels**
  (« spaghetti ») : là où deux bruits sont tous deux proches de zéro, d'épaisseur variable ; **réseaux fins**
  (« nouilles ») : le même principe, plus fin, seulement par endroits.
- **Croûte** : pas de grotte à moins d'une dizaine de blocs du sol (la densité du relief sert de garde), sauf aux
  **entrées** : des tunnels peuvent alors déboucher, mais jamais sous la mer, sur les côtes basses ni au fond des vallées
  (l'eau s'y engouffrerait). Sous un fond de mer, la croûte tient toujours.
- Environ 8 à 13 % du sous-sol est creusé (mesuré sur plusieurs colonnes), plus que la v1 qui n'avait pas de monde sous 0.

### Nappes et lave (dans `generator/v3.rs`)

Les aquifères à cellules 3D, à la Minecraft, ont été essayés et écartés : leurs barrières laissaient des éclats de roche
suspendus et de l'eau en cuvettes au milieu des grandes salles. À la place :

- **Régions de nappe** de 96 × 96 blocs : un niveau d'eau plat par région (un bruit lu au centre de la région, toujours une
  vingtaine de blocs sous la surface, ou région sèche). Tout le vide souterrain sous ce niveau est noyé : la surface
  d'un lac souterrain est plate, et l'eau ne repose jamais sur du vide.
- **Barrages** : à la frontière de deux régions de niveaux différents, une paroi de roche de 3 blocs retient l'eau.
- **Lave** : le vide des 20 derniers blocs au-dessus du socle (y < -108).
- La mer ne remplit que ce qui est à l'air libre sous y = 62 ; les grottes ne sont jamais noyées par elle.

### Corrections trouvées en route

- **Matériaux de surface** : ils ne vont plus qu'au sol du dessus, plein sans interruption depuis le haut de la colonne.
  Avant, l'herbe posée sur les plafonds et planchers de grottes proches de la surface redevenait de la terre dans le noir
  sous les ticks aléatoires, et le monde se remaillait sans fin (chargement de 69 s au lieu de 8).
- **Froid** : la température d'une colonne suit la même règle que `plants::is_cold` (la neige des ticks aléatoires) ;
  sinon la génération et le jeu se contredisaient sur les montagnes.
- **Terrain lointain** : `column_at_grid_point` cherche la surface sur la seule verticale (49 points, sans grottes) au lieu
  de construire un contexte de colonne complet (et d'évincer ceux dont les chunks ont besoin). Test : 90 % des points à
  3 blocs près de la vraie hauteur, aucun à plus de 24.
- **Moteur de densité** : `Min` et `Max` s'arrêtent dès que les bornes de l'autre côté ne peuvent plus changer le
  résultat (au-dessus du sol, les grottes ne sont plus calculées) ; la pente est gardée par position (`CacheOnce`).

### Tests (`tests/generation_v3.rs`, 3 nouveaux)

Sous-sol creusé de 3 à 25 %, avec des grottes sèches et des nappes ; aucune eau ni lave au-dessus du vide ; aucun vide
dans les 6 blocs sous un fond de mer ; lave seulement au fond ; peu de niveaux d'eau par chunk (nappes plates) ; terrain
lointain proche de la vraie surface.

### Mesures

- Génération : 7,0 ms par colonne de 12 couches (3,9 ms sans grottes, 2,0 ms pour les 6 couches de v1).
- Banc v3 à rayon 16 : chargement de 3,4 à 7,7 s par trajet (v1 : 2 à 7 s), CPU médian 0,5 à 1,3 ms, p99 1,7 à 2,8 ms.

### Limites

- Les grottes n'ont pas encore de décor (biomes de grotte, stalactites, végétation) : lot 4 et vague 4 du lot 5.
- Les barrages sont des parois droites ; l'eau ne coule pas à la génération (elle est figée jusqu'à une modification).
- Pas de ravins.

## Statut du lot 4 : biomes par paramètres et règles de surface (à valider)

### Ce qui change

- **Biomes par paramètres** (`assets/core/worldgen/biomes_v3.ron`, `generator/v3_rules.rs`) : chaque biome occupe une
  case dans l'espace (température, humidité, continentalité, érosion, pics et vallées) ; une colonne prend le biome le
  plus proche (distance nulle dedans, carré de l'écart dehors, petit décalage pour départager). Mers selon la
  continentalité et la température, plages étroites, rivières au fond des vallées de l'intérieur, pics et massifs selon
  l'érosion et les crêtes, terres selon le climat (glacial, froid, tempéré, chaud, brûlant × sec à détrempé). Les 32 biomes
  du jeu ont une case (un test le vérifie) ; un biome inconnu (mod retiré) est ignoré.
- **Règles de surface** (`assets/core/worldgen/surface_v3.ron`) : une liste ordonnée (conditions → matériau) appliquée au
  sol du dessus ; conditions de profondeur, sous-couche, biome, sous la mer, gel, pente raide, hauteur, bruit, négation ;
  matériaux bloc, surface du biome (avec ses taches de sol), sous-couche, fond sous l'eau, strates. Les versants raides
  sont en roche avec des plaques de gravier, la neige tient au gel (même règle que les ticks aléatoires), les pics
  enneigés et rocheux ont leur couverture, les badlands leurs strates.
- **Pente** : par colonne, l'écart de hauteur entre les voisins (plus de 3 blocs sur 2 : raide).
- **Outil de recette** : `--versions 1,2,3` (v1 et v3 par défaut).

### Tests

`v3_rules.rs` (3) : les tables du jeu ne nomment que des biomes et blocs connus, chaque biome a une case ; la case la
plus proche gagne et le décalage départage ; la première règle qui tient donne le matériau. `generation_v3.rs` (2) : les
fonds de vallée de l'intérieur sont des rivières, les mers profondes des océans, les crêtes des massifs des pics ; jamais
d'herbe là où il gèle ni sous le sol du dessus (planchers et plafonds de grottes).

### Paysage, v1 contre v3 (outil `recette`, neuf graines, 20 000 × 20 000 blocs, un point tous les 200)

| graine | gén. | terre | océan prof. | montagne | falaise | marches 1 bloc | constructible | forêt | arbres/chunk | eau douce | biomes | entropie | h moy. | s |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 7 | v1 | 61.9 % | 9.4 % | 9.3 % | 0.5 % | 28.5 % | 52.7 % | 21.0 % | 3.8 | 3.4 % | 27 | 4.62 | 70 | 1 |
| 7 | v3 | 55.6 % | 10.9 % | 6.8 % | 0.7 % | 26.9 % | 38.4 % | 26.9 % | 4.8 | 3.6 % | 29 | 4.11 | 67 | 123 |
| 42 | v1 | 62.7 % | 8.8 % | 10.8 % | 0.6 % | 29.9 % | 51.8 % | 20.4 % | 3.7 | 3.7 % | 27 | 4.63 | 72 | 1 |
| 42 | v3 | 56.8 % | 8.3 % | 5.4 % | 0.7 % | 26.9 % | 39.5 % | 25.1 % | 4.5 | 3.9 % | 29 | 4.12 | 67 | 124 |
| 1337 | v1 | 61.6 % | 9.8 % | 12.1 % | 0.5 % | 29.8 % | 50.0 % | 18.3 % | 3.4 | 3.3 % | 27 | 4.59 | 72 | 1 |
| 1337 | v3 | 56.1 % | 9.2 % | 5.8 % | 0.8 % | 26.6 % | 39.2 % | 25.7 % | 4.5 | 3.7 % | 29 | 4.11 | 66 | 122 |
| 20260 | v1 | 62.7 % | 8.9 % | 10.4 % | 0.7 % | 29.2 % | 51.8 % | 20.1 % | 3.7 | 3.7 % | 27 | 4.63 | 71 | 1 |
| 20260 | v3 | 56.5 % | 11.2 % | 6.4 % | 0.7 % | 26.7 % | 38.9 % | 26.3 % | 4.7 | 3.9 % | 29 | 4.15 | 66 | 124 |
| 987654321 | v1 | 62.1 % | 8.2 % | 10.5 % | 0.5 % | 28.7 % | 53.5 % | 20.6 % | 3.7 | 4.2 % | 27 | 4.58 | 71 | 1 |
| 987654321 | v3 | 58.0 % | 10.1 % | 6.8 % | 0.8 % | 27.5 % | 38.5 % | 28.5 % | 4.9 | 3.9 % | 29 | 4.10 | 68 | 126 |
| 0 | v1 | 62.3 % | 9.2 % | 11.4 % | 0.4 % | 29.6 % | 51.2 % | 19.9 % | 3.7 | 3.7 % | 27 | 4.61 | 71 | 1 |
| 0 | v3 | 58.3 % | 8.6 % | 7.2 % | 0.7 % | 27.4 % | 37.5 % | 24.6 % | 4.5 | 4.5 % | 29 | 4.21 | 68 | 127 |
| 1 | v1 | 61.7 % | 8.5 % | 10.9 % | 0.6 % | 30.2 % | 50.4 % | 19.4 % | 3.5 | 3.7 % | 27 | 4.61 | 71 | 1 |
| 1 | v3 | 60.3 % | 8.3 % | 7.5 % | 0.9 % | 28.8 % | 36.2 % | 25.2 % | 4.6 | 4.2 % | 29 | 4.16 | 69 | 128 |
| 123456 | v1 | 62.6 % | 8.7 % | 10.0 % | 0.6 % | 28.6 % | 53.0 % | 19.8 % | 3.6 | 3.7 % | 27 | 4.61 | 71 | 1 |
| 123456 | v3 | 56.0 % | 9.9 % | 5.6 % | 0.9 % | 27.5 % | 39.0 % | 27.2 % | 4.7 | 3.9 % | 29 | 4.13 | 66 | 125 |
| 4294967295 | v1 | 61.7 % | 8.3 % | 10.6 % | 0.8 % | 28.8 % | 52.4 % | 22.6 % | 4.0 | 3.5 % | 27 | 4.58 | 71 | 1 |
| 4294967295 | v3 | 58.1 % | 8.3 % | 7.1 % | 1.0 % | 26.9 % | 38.4 % | 26.5 % | 4.7 | 4.4 % | 29 | 4.13 | 68 | 127 |

v3 est **stable d'une graine à l'autre**. Par rapport à v1 : un peu moins de terre (56 à 60 % contre 62 %), moins de
« montagne » au sens de l'outil (plus de 60 blocs au-dessus de la mer : 5 à 8 % contre 9 à 12 %, mais des sommets bien plus
hauts), plus de falaises (0,7 à 1,0 % contre 0,4 à 0,8 %), moins de terrain plat constructible (36 à 40 % contre 50 à 53 %),
plus de forêts (25 à 29 %), autant de rivières, 29 biomes vus.

### Coût

7,9 ms par colonne de 12 couches (le bruit des plaques de gravier compris).

### Limites

- Les biomes restent par colonne (x, z) : pas encore de biome de grotte (un chunk ne garde qu'un biome par colonne) ; le
  choix par la profondeur viendra avec la vague 4 du lot 5.
- Les cases de `biomes_v3.ron` ne sont pas encore extensibles par les mods (un mod qui ajoute un biome devra y ajouter sa
  case : à prévoir avec le lot 5).
- Le terrain lointain coupe encore l'horizon des grands massifs (lot 6).

## Statut du lot 5, vague 1 : six biomes inspirés de Biomes O' Plenty (à valider)

Idées reprises de Biomes O' Plenty (sans son code ni ses textures) et écrites ici :

| Biome | Climat (cases de `biomes_v3.ron`) | Ce qui le distingue |
| --- | --- | --- |
| Forêt de séquoias (`core:redwood_forest`) | tempéré, détrempé, sur les hauteurs | séquoias géants (tronc 2 × 2, 20 à 26 blocs, couronne en cône), sapins, podzol, fougères |
| Bayou (`core:bayou`) | chaud, détrempé, plaines usées et basses | cyprès (tronc haut et fin, « genoux », lianes), eau trouble, argile, nénuphars |
| Champs de lavande (`core:lavender_field`) | tempéré, sec, sur les hauteurs | lavande en nappes, quelques arbres en fleurs |
| Prairie (`core:prairie`) | chaud, assez sec, plaines usées | herbe dorée haute, buissons |
| Verger (`core:orchard`) | tempéré, moyen, creux doux | arbres fruitiers en fleurs (pommes à la récolte) |
| Forêt d'érables (`core:maple_woods`) | froid, moyen à humide, sur les hauteurs | érables rouge orangé, bouleaux |

- **Blocs** : bûche, feuilles, planches et pousse de séquoia ; feuilles et pousse d'érable ; feuilles en fleurs. Textures
  produites par `assetgen` (pack classique, et pack 32 par agrandissement), recettes (planches), butin (pousses, bâtons,
  pommes), étiquettes `logs`, `planks`, `leaves`.
- **Arbres** (`trees.rs`) : `Redwood`, `Maple`, `Fruit`, `Cypress` ; les pousses de séquoia et d'érable donnent leur arbre ;
  le terrain lointain a leurs silhouettes.
- **Les mondes v1 et v2 ne changent pas** : ces biomes portent `v3_only` et n'entrent pas dans leurs choix (empreintes
  gelées de v1 et v2 inchangées, et un test le vérifie sur des milliers de colonnes).
- **Table v3** : les biomes larges (plaines, forêt, bouleaux, taïga, garrigue) ont reçu un petit décalage, pour que les
  cases plus étroites logées dedans l'emportent ; la prairie fleurie, qui n'était jamais choisie à cause d'un décalage
  inverse, l'est maintenant. Le marais s'arrête aux climats tempérés (le bayou prend le chaud), la jungle aux reliefs
  non usés.
- **Rendu** : le tableau de textures des blocs dépassait la limite par défaut de wgpu (256 couches, il en faut 258) ; le
  renderer demande désormais la limite de la carte (2 048 sur Metal, plafonnée à 2 048).
- **Captures** : six vues ajoutées au tour (`biome-sequoias`, `biome-bayou`, `biome-lavande`, `biome-prairie-seche`,
  `biome-verger`, `biome-erables`).

Tests : les quatre formes restent dans les bornes de la génération par chunk (rayon 8, hauteur 30) ; les pousses donnent
le bon arbre ; les six biomes apparaissent dans un monde v3 de 12 km de côté et jamais dans un monde v1 ou v2.

Limites : la prairie forme de petites zones (son climat chaud et assez sec est partagé avec la garrigue et la savane) ;
pas encore de bloc de boue pour le bayou ; les vagues 2 à 4 (froid, chaud et volcanique, souterrain) restent à faire.

## Statut du lot 5, vague 2 : froid et altitude (à valider)

| Biome | Climat (cases de `biomes_v3.ron`) | Ce qui le distingue |
| --- | --- | --- |
| Toundra (`core:tundra`) | froid, sec à moyen, plaines usées | herbe rousse, terre grossière et podzol, buissons bas, rochers, un peu de bruyère ; la neige ne tient qu'en altitude |
| Sapinière enneigée (`core:snowy_fir_forest`) | glacial, humide, sur les hauteurs | sapins serrés aux aiguilles bleutées, sol enneigé |
| Hautes terres (`core:highland`) | froid à tempéré, sec, collines de l'intérieur | herbe haute, rochers épars, bruyère, rares épicéas |
| Landes (`core:moor`) | froid, détrempé, plaines usées et basses | tapis de bruyère, podzol, rochers, eau sombre, fonds d'argile |
| Escarpements (`core:crag`) | froid, sec, massifs escarpés | roche nue avec plaques de gravier, de moellons moussus et d'herbe, rochers, épicéas et sapins ; la neige ne tient qu'au-dessus de 180 environ (ils sont entre 145 et 190, au pied des pics) |

- **Blocs** : aiguilles et pousse de sapin, bruyère, moellons moussus (textures `assetgen` dans les deux packs, butin des
  aiguilles, étiquettes `leaves` et `stones`, recette sans forme moellons + liane → moellons moussus).
- **Arbres** (`trees.rs`) : `Fir` (tronc d'épicéa, 11 à 17 blocs, étages courts, flèche) que donne la pousse de sapin, et
  `Boulder`, un rocher de moellons à demi enfoncé, moussu par endroits, posé par le même tirage que les arbres (il accepte
  la roche et la terre, pas le sable ni le gravier). Le terrain lointain a la silhouette du sapin, pas celle des rochers.
- **Table v3** : les montagnes, la taïga enneigée et la vieille taïga reçoivent un petit décalage pour laisser gagner les
  nouvelles cases. Part de la terre (5 graines, `print_v3_biome_shares`) : toundra 2,4 %, hautes terres 2,1 %, sapinière
  1,2 %, landes 0,9 %, escarpements 0,5 % (les massifs eux-mêmes sont rares : les escarpements y prennent la place des
  montagnes sèches).
- **Captures** : `biome-toundra`, `biome-sapiniere`, `biome-hautes-terres`, `biome-landes`, `biome-escarpements`.

Tests : le sapin reste dans les bornes de la génération par chunk et sort de sa pousse ; le rocher est petit, à demi
enfoncé, sans mousse sous terre ; les onze biomes des vagues 1 et 2 apparaissent dans un monde v3 et jamais en v1 ou v2.

Limites : pas de relief propre aux escarpements (ce sont des biomes de surface sur le relief commun : pas d'aiguilles
rocheuses comme dans Biomes O' Plenty) ; la neige ne se pose pas sur les feuilles.

## Statut du lot 5, vague 3 : chaud et volcanique (à valider)

| Biome | Où | Ce qui le distingue |
| --- | --- | --- |
| Volcan (`core:volcano`) | les cônes du relief (pas de case climatique) | cône de roche volcanique de 80 blocs de rayon et 75 de haut, cendres, cratère et lac de lave plat |
| Plaines volcaniques (`core:volcanic_plains`) | brûlant, ni sec ni humide, intérieur | herbe sombre, coulées de roche volcanique et de cendres, buissons, acacias, rochers ; les volcans s'y posent |
| Sources chaudes (`core:hot_springs`) | chaud moyen, plaines usées | bassins d'eau turquoise d'un ou deux blocs sur le sol plat, fonds d'argile, gravier |
| Maquis (`core:maquis`) | chaud assez sec, sur les hauteurs | oliviers (feuillage argenté, couronnes plates), lavande, terre grossière |
| Désert fleuri (`core:flowering_desert`) | brûlant, au bord le moins sec du désert | sable couvert de soucis et de coquelicots, cactus |

La garrigue existait déjà (`core:shrubland`) : la vague prend le maquis à sa place.

- **Volcans dans le relief** : un nouveau nœud de densité, `Cones` (au plus un cône par carré de `cell` blocs, tiré
  avec `chance`, entièrement dans son carré, gardé si un masque est positif à son centre). Sa hauteur (en blocs)
  s'ajoute à `depth` (1/128 par bloc), donc à tout ce qui lit le relief : blocs, hauteurs, terrain lointain. Le
  masque (`volcano_mask` de `terrain_v3.ron`) suit le climat des plaines volcaniques ; carrés de 320 blocs, chance 0,85.
  Au-dessus de 12 blocs de cône, le biome est le volcan.
- **Lave des cratères** : un niveau par volcan (deux blocs sous le point le plus bas du bord, lu sur huit points aux
  coins des cellules), le même pour toutes les colonnes de chunks qu'il touche ; la lave ne remplit que les colonnes du
  cratère plus basses.
- **Bassins des sources chaudes** : là où un bruit est fort et les 5 × 5 colonnes autour à la même hauteur, un ou deux
  blocs d'eau remplacent le sol ; la hauteur de la colonne devient le fond.
- **Blocs** : cendres volcaniques, feuilles et pousse d'olivier, souci du désert (textures `assetgen` dans les deux
  packs, butin, étiquette `leaves`). Arbre `Olive`, que donne sa pousse ; silhouette lointaine.
- **Plantes** : dans les biomes réservés à la v3, une plante pousse aussi sur le sol du biome (les fleurs sur le sable
  du désert fleuri) ; v1 et v2 ne changent pas.
- **Table v3** : la lisière de jungle chaude et le désert reçoivent un décalage, les hautes terres rendent un peu
  d'érosion aux alpages, les escarpements s'arrêtent au froid. Part de la terre (5 graines) : plaines volcaniques 3,3 %,
  maquis 2,5 %, désert fleuri 0,9 %, sources chaudes 0,8 %, volcans 0,2 % ; la lisière de jungle passe de 15 à 6 %.
- **Coût** : 8,7 ms par colonne de chunks contre 8,2 avant la vague (`print_v3_cost`).
- **Captures** : `biome-volcan` (vue sur le cratère du volcan le plus proche), `biome-plaines-volcaniques`,
  `biome-sources-chaudes`, `biome-maquis`, `biome-desert-fleuri`.

Tests : le nœud `Cones` est compilé et borné ; un cratère tient un lac de lave plat, sans vide dessous ni à côté ; les
bassins des sources chaudes tiennent leur eau de même ; le volcan porte son biome ; les seize biomes des vagues 1 à 3
apparaissent en v3 et jamais en v1 ou v2.

Limites : les bassins ne fument pas ; les volcans n'ont ni coulées ni cheminée sous le cratère ; le désert proprement
dit et les alpages restent rares (0,1 % chacun).
