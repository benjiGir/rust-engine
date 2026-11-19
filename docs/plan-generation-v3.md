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
