# Plan : géographie cohérente et affichage rapide des chunks

Établi le 17 novembre 2026. Statut au 18 novembre 2026 : **lots 0 et 1 exécutés** (livraison A,
voir « Statut de la livraison A » en bas) ; **lots 2 et 3 exécutés comme prototype macro** (livraison B,
voir « Statut de la livraison B ») ; **lot 4 exécuté comme monde de test v2** (voir « Statut du lot 4 ») ; lots 5 à 9 non
commencés.

## Résultat recherché

Construire un monde dont les paysages se comprennent à plusieurs échelles : des
continents et leurs côtes, des chaînes et leurs contreforts, des bassins, des
affluents qui rejoignent un cours principal, des plaines et des biomes adaptés au
relief. Le bruit ajoute de la variété à cette organisation. Le paysage conserve
sa silhouette lorsqu'on s'approche et les chunks proches arrivent rapidement.

Ce plan complète l'analyse de Horizon avec la direction visuelle de Lithosphere.
Il définit nos propres algorithmes, courbes et réglages.

## Références et portée de l'étude

- [Lithosphere, présentation de l'auteur](https://modrinth.com/datapack/lithosphere) :
  continents, côtes progressives, chaînes moins ponctuelles, vallées continues et
  régions climatiques plus étendues.
- [Lithosphere 1.9, notes de version](https://modrinth.com/datapack/lithosphere/version/1.9) :
  reliefs mieux articulés, ombre pluviométrique et placement des biomes revu.
  Ces pages documentent un résultat et certains mécanismes ; elles ne démontrent
  pas une simulation de plaques tectoniques ou un solveur complet d'hydrologie.
  Notre réseau de bassins décrit ci-dessous est une proposition pour ce moteur.
- [Horizon, génération](https://github.com/horizon-rust/horizon/blob/main/docs/world-generation.md)
  et [rendu](https://github.com/horizon-rust/horizon/blob/main/docs/graphics.md).
  Le code examiné lors de l'analyse précédente est la révision
  `c70c5b965838718a7def1994ffb6eee7794d363b` : contextes par worker, densité
  interpolée, arènes GPU et chemin de regroupement adapté à Metal.
- Génevaux et al., [Terrain Generation Using Procedural Models Based on Hydrology](https://perso.liris.cnrs.fr/eric.galin/Articles/2013-river-networks.pdf),
  2013 : référence pour organiser un relief autour d'un réseau de drainage
  hiérarchique. Son domaine de construction est fini ; l'adaptation au streaming
  de notre monde doit être validée au lot 3.

## État actuel vérifié dans le code

| Domaine | Code actuel | Conséquence pour le plan |
| --- | --- | --- |
| Relief | [`generator.rs`](../crates/engine_voxel/src/generator.rs), `interpret`, `generate` | Hauteur par colonne, puis cavités 3D ; les montagnes et rivières proviennent de champs de bruit distincts. |
| Rivières | `interpret` | Le passage à zéro d'un bruit creuse vers le niveau marin ; aucun graphe de bassins ne relie les cours d'eau. |
| Surface | `layer` | Matériaux selon biome, altitude et profondeur ; pas de règle de pente du relief continu. |
| Calculs partagés | `columns`, `generate` | Avant le lot 1 : climat et hauteur recalculés pour chaque chunk d'une même verticale. Depuis : cache des colonnes (`generator/column_cache.rs`) ; les sites d'arbres ont leur cache. |
| Streaming | [`streaming.rs`](../crates/engine_voxel/src/streaming.rs) | Pool parallèle, tâches bornées, priorités par distance et étapes ; les dépendances de lumière précèdent le maillage. |
| Maillage | [`mesh.rs`](../crates/engine_voxel/src/mesh.rs) | Greedy, AO, lumière douce et sommets de 24 octets existent déjà. |
| GPU | [`renderer.rs`](../crates/engine_render/src/renderer.rs), `add_mesh_scaled`, `draw_scene` | Trois tampons par maillage et appels par maillage. |
| Transferts | [`main.rs`](../games/voxel/src/main.rs), `apply_stream_events` | Limite de 16 maillages par image, indépendamment de leur taille. |
| Lointain | [`far.rs`](../crates/engine_voxel/src/far.rs), [`far_terrain.rs`](../games/voxel/src/far_terrain.rs) | Surface simplifiée du générateur, deux workers dédiés et transferts sans budget partagé avec les chunks proches. |
| Persistance | [`save.rs`](../crates/engine_voxel/src/save.rs), `WorldMeta` | Graine enregistrée ; seuls les chunks modifiés sont sauvegardés. Depuis le lot 0 : version de génération et configuration effective (`generation`). |

## Contrats communs

1. Un ancien monde conserve sa génération. Une métadonnée absente sélectionne
   explicitement la version historique ; aucune migration silencieuse du relief.
2. Même graine, dimension, version et configuration effective : mêmes blocs,
   quels que soient l'ordre des chunks, le nombre de threads et l'état des caches.
3. Les coordonnées négatives, les frontières de régions et les coins de quatre
   régions doivent produire les mêmes résultats qu'une génération d'ensemble.
4. Les calculs régionaux ont des limites de mémoire, de tâches et de travail par
   demande. Un cache accélère une valeur ; il ne décide jamais de sa valeur.
5. Les champs restent continus aussi longtemps que possible. Arrondir en blocs
   seulement lors de la matérialisation, avec une règle déterministe explicite.
6. Les surplombs et les détails ne coupent pas un cours d'eau validé, ne ferment
   pas son exutoire et ne déplacent pas les côtes indépendamment du drainage.
7. Aucun changement d'identifiant ou d'ordre des blocs. Les formes, animations,
   fluides, machines, structures mobiles et mods restent pris en charge.
8. Aucune intervention dans `saves/`, `.idea/` ou les WASM existants. Les tests de
   persistance utilisent des répertoires temporaires. Les séances graphiques ne
   démarrent qu'avec un mécanisme d'isolation des sauvegardes validé au lot 0.

## Organisation cible

Les noms ci-dessous sont proposés. Ils ne désignent pas des API déjà présentes.

- Un contexte de région immuable décrit côtes, grandes lignes du relief, bassins,
  climat et références de configuration. Il est partagé entre tâches par `Arc`.
- Un contexte de colonne fournit hauteur indicative, pente, biome, matériaux,
  nappe et paramètres de vallée. Les six chunks verticaux réutilisent ses données.
- Un contexte par worker garde les tableaux de bruit, densité, matériaux et
  maillage ; il ne conserve pas de références de génération périmées.
- Des requêtes communes de densité, surface et eau alimentent chunks détaillés,
  placement des structures et terrain lointain.
- La géographie et le drainage sont définis en coordonnées monde. Les chunks
  consomment cette définition ; ils n'en sont pas les frontières de conception.

Découper progressivement `generator.rs` en modules ciblés, par exemple
`generator/geography.rs`, `hydrology.rs`, `density.rs`, `surface.rs` et
`context.rs`. Garder un générateur historique identifiable et testé. Chaque
fichier Rust reste sous la limite d'architecture de 2 500 lignes.

## Lots d'exécution

### Lot 0 — référence, version des mondes et outils d'observation (fait, voir le statut)

**Dépendance :** aucune. **Livrable :** une référence reproductible avant les changements.

- Enregistrer version de génération et configuration effective par dimension :
  paramètres, règles utilisées et empreinte des données de génération pertinentes
  des mods. Une empreinte seule ne permet pas de reconstruire une configuration :
  conserver les données ou une référence immuable réellement résoluble.
- Prévoir le choix explicite du nouveau générateur à la création. Garder le chemin
  historique pour les mondes anciens ; séparer version du format de sauvegarde et
  version de génération. Signaler une configuration de mod manquante ou modifiée.
- Capturer des empreintes de blocs de référence sur les graines 7, 42 et un corpus
  fixe d'autres graines : surfaces, sous-sol, bords et coordonnées négatives.
- Exploiter les benches `generation` et `meshing`, le parcours de
  [`bench.rs`](../games/voxel/src/bench.rs), les spans et les timestamps GPU existants.
- Mesurer le délai demande → génération → lumière → maillage → GPU → première
  image contenant le chunk ; relever files d'attente, octets, allocations, mémoire,
  draw calls et temps CPU/GPU séparés des attentes VSync.
- Ajouter un outil d'export de cartes sans fenêtre : altitude, pente, côtes,
  bassins, écoulements, humidité et biomes. Les noms et commandes de cet outil sont
  à définir lors de ce lot. Vérifier également l'isolation des sauvegardes pour
  les futurs tests graphiques : `VOXEL_WORLD` change actuellement le nom, pas la racine.

**Passage au lot suivant :** empreintes historiques identiques, ancien `WorldMeta`
relu avec la bonne version, mesures froides et chaudes séparées, cartes exportables.

### Lot 1 — calculs réutilisés, sans changement de paysage (fait, voir le statut)

**Dépendance :** lot 0. **Livrable :** base de génération plus efficace et mesurée.

- Calculer une fois les cartes de colonne et les plans réutilisables d'une
  verticale. Garder des instantanés immuables, un cache borné et éviter qu'un
  verrou global protège les boucles d'évaluation.
- Réutiliser les buffers de travail par worker ; garder le cache d'arbres existant
  et mesurer avant de changer sa politique d'éviction.
- Expliciter les clés : graine, dimension, version, configuration et coordonnées.
  Dédupliquer les demandes simultanées d'une même région sans bloquer le rendu.
- Vérifier les formules et leur ordre d'évaluation : la réorganisation des tâches
  ne doit pas changer la sortie du générateur historique.

**Passage :** aucune différence de blocs sur le corpus ; réduction mesurée du travail
répété, mémoire bornée et aucune attente ajoutée au thread de rendu.

### Lot 2 — continents, chaînes et familles de relief (fait : prototype macro, voir le statut B)

**Dépendance :** lots 0 et 1. **Livrable :** géographie lisible sur les cartes macro.

- Définir des échelles indépendantes pour continents, chaînes, vallées et détails.
  Régler leurs proportions sur la hauteur actuelle de 192 blocs ; augmenter la
  hauteur du monde reste une décision distincte.
- Produire un champ terre/mer cohérent et une distance au rivage. Distinguer
  plateaux continentaux, plages, estuaires réservés et côtes rocheuses.
- Construire des axes de chaînes, des crêtes secondaires, des cols et des
  contreforts. Leur orientation et leurs influences sont partagées entre régions.
  Les pics locaux ne doivent pas constituer à eux seuls l'organisation montagneuse.
- Utiliser des courbes de profil continues pour passer plaine → piémont → massif,
  avec des paramètres originaux. Réserver des zones constructibles et des passages.
- Le bruit fin module ces formes sans casser leur hiérarchie. Une simulation
  tectonique complète n'est pas nécessaire pour ce premier générateur.

**Passage :** chaînes raccordées, transitions sans marches artificielles entre régions,
côtes variées, absence d'écrêtage systématique des sommets et lecture convaincante
sur plusieurs graines, pas seulement une vue choisie.

### Lot 3 — bassins, affluents, lacs et vallées (fait : prototype macro, voir le statut B)

**Dépendance :** lot 2. **Livrable :** prototype hydrologique validé avant le remplissage des blocs.

- Construire un graphe hiérarchique de bassins et d'exutoires sur une grille macro
  peu coûteuse. Les embouchures et lacs terminaux structurent les bassins ; les
  affluents rejoignent un réseau existant au lieu d'être des lignes de bruit isolées.
- Définir les relations entre bassins à une échelle supérieure aux régions de
  calcul. Un contrat de bord donne position, identité, direction, largeur et niveau
  d'eau de chaque passage ; ses deux voisins lisent la même définition immuable.
- Valider une hiérarchie acyclique dont chaque chemin atteint un océan ou un lac
  terminal déclaré, en un nombre borné d'étapes. La définition des contrats et la
  résolution d'un bassin ne doivent pas exiger d'explorer un continent entier.
- Un halo partagé facilite les calculs locaux, mais ne suffit pas à garantir un
  drainage raccordé. Ne pas calculer séparément un remplissage de dépressions
  dans chaque tuile puis masquer les coutures. Les frontières découlent du graphe
  commun ; leurs directions ne sont pas tirées indépendamment.
- Affecter des profils d'altitude descendants aux cours d'eau, puis modeler leurs
  vallées. Autoriser paliers et cascades après quantification en blocs, jamais une
  remontée involontaire. Ajuster le relief autour des corridors hydrologiques.
- Déduire largeur et profondeur d'une accumulation de bassin approximée. Prévoir
  ruisseaux, confluences, cours principaux, plaines alluviales et embouchures.
- Définir niveau, berges et éventuel déversoir de chaque lac ; identifier les bassins
  fermés intentionnels. Différencier vallée étroite en pente forte, méandres de
  plaine et grandes vallées montagneuses.
- Commencer par une hydrologie statique de génération : ne pas simuler les pluies
  ou l'écoulement de tout le monde à chaque tick. Vérifier sa compatibilité avec
  les réservoirs de fluide stables du moteur et les modifications du joueur.

**Passage bloquant :** mêmes rivières depuis deux régions séparées ou une région
réunie, confluences de niveaux compatibles, aucun cycle, aucun cours interrompu
sans terminaison déclarée, raccords valides aux coins de quatre régions et coût
borné à froid. Si ces conditions échouent, revoir le modèle macro avant le lot 4.

### Lot 4 — relief 3D, surface et sous-sol cohérents (fait : monde de test v2, voir le statut du lot 4)

**Dépendance :** lot 3. **Livrable :** géographie matérialisée en blocs.

- Construire une densité `D(x, y, z)` à partir du relief et des corridors validés.
  Échantillonner sur une grille interpolée, avec caches par worker. Ajouter des
  surplombs et arches dans les familles de relief qui les justifient.
- Réserver les vallées, lits et exutoires dans les opérations de densité ; les
  petits détails ne doivent pas les obstruer. Vérifier à nouveau les profils après
  quantification et après les passes qui ajoutent des blocs.
- Appliquer des règles de surface selon pente continue, altitude, humidité,
  exposition et proximité d'eau : rocher de falaise, éboulis, sols, berges, neige.
- Produire des ensembles de strates régionaux continus et faire varier leur
  résistance apparente pour donner une identité aux falaises et vallées.
- Raccorder les cavernes, contrôler les ouvertures de surface et définir les
  niveaux des nappes en relation avec les bassins. Garder la cohérence eau/lave,
  les séparations nécessaires et les règles de génération des minerais.
- Recalculer sol réel et dégagement pour arbres, villages et autres structures :
  une ancienne hauteur indicative ne suffit plus avec des surplombs. Préserver
  leur accessibilité et éviter les constructions dans le lit des cours d'eau.

**Passage :** coupes de terrain et cartes en accord, rivières intactes après toutes
les passes, sols adaptés aux pentes, structures correctement posées, cavités et
nappes sans raccords artificiels. La simulation des fluides reste stable au chargement.

### Lot 5 — climat, biomes et végétation liés au paysage

**Dépendance :** lots 3 et 4. **Livrable :** ensembles écologiques cohérents.

- Définir une température régionale et son effet d'altitude, puis une humidité
  liée aux côtes, à l'eau et à l'exposition. Une estimation bornée des obstacles
  au vent sur la grille macro peut créer une ombre pluviométrique.
- Garder les grandes régions climatiques lisibles et introduire des variations
  locales contrôlées. Un marais doit correspondre à une zone basse et humide ;
  une plage doit correspondre au rivage ; un biome de rivière au réseau réel.
- Choisir les biomes après établissement du contexte géographique. Définir une
  adaptation documentée des paramètres `Relief` existants : ils peuvent moduler
  des familles compatibles, sans redessiner une vallée protégée après coup.
- Organiser lisières, bosquets, clairières et végétation riveraine selon sol,
  humidité, pente et altitude ; conserver les règles de ressources et d'apparition
  jouables. Garder le mélange de teintes de biomes déjà présent dans le maillage.
- Garder le calcul acyclique : une première hydrologie structure le relief ; la
  pluie peut ensuite moduler l'apparence et un débit indicatif. Toute rétroaction
  qui changerait les bassins exige une nouvelle étape bornée et testée.

**Passage :** cartes relief/eau/climat/biomes concordantes, absence de bandes de
biomes incohérentes sur les côtes, diversité conservée, ressources et zones de
construction accessibles sur le corpus.

### Lot 6 — priorité aux chunks proches et transferts régulés

**Dépendance :** lot 1. Peut avancer pendant les lots de géographie.

- Prioriser les groupes de dépendances utiles à la caméra et à la collision :
  génération des voisins nécessaires, lumière, puis maillage. Préférer la vue et
  le déplacement sans affamer les alentours ou rompre les conditions de lumière.
- Partager un budget CPU global avec le lointain. Réduire son activité pendant un
  chargement proche important, borner ses files et abandonner les demandes périmées.
- Réguler réception, attente des maillages et transferts selon temps CPU et octets,
  plutôt qu'un seul nombre de maillages. Mesurer les dépassements dus à une grosse
  opération indivisible ; ne pas instaurer un plafond d'octets qui la bloque indéfiniment.
- Garder les anciens maillages jusqu'au remplacement prêt. Conserver les tickets
  de validité et regrouper les invalidations pour éviter remaillages répétés et
  transferts déjà périmés au moment de leur consommation.

**Passage :** délai demande → visible amélioré dans les scénarios limités par ces
files, moins de pics de transferts, aucune file non bornée ni famine en téléportation
ou en vol. Vérifier avec lointain activé et désactivé.

### Lot 7 — maillage et rendu groupé, avec chemin Metal mesuré

**Dépendance :** lot 6. **Livrable :** moins de travail CPU pour préparer et afficher les chunks.

- Ajouter un chemin par masques binaires pour cubes opaques, avec buffers par
  worker. Garder une voie complète pour boîtes, plantes, raccords, fluides et
  animations ; préserver texture, AO et lumière lors des fusions greedy.
- Stocker la géométrie dans des arènes GPU paginées, avec plages réutilisables et
  transferts groupés via des tampons de staging réemployés. Borner fragmentation,
  croissance et compactage ; différer les réutilisations jusqu'à leur sûreté GPU.
- Décrire les origines et attributs des chunks dans une table commune. Grouper les
  dessins par pages et passes. Conserver l'ordre nécessaire au translucide.
- Comparer appels directs, multi-draw disponible et indices regroupés sur Metal.
  Horizon signale que le multi-draw peut être émulé sur ce backend : choisir selon
  capacités et mesures. Garder un chemin de repli, notamment pour les GPU Intel.
- Mesurer le parcours de visibilité ; passer à des structures denses si son coût
  le justifie. Conserver frustum et culling des grottes déjà fonctionnels.

**Passage :** image équivalente avant/après sur une géométrie gelée, coûts CPU/GPU
améliorés sur la cible concernée, pas de fuite mémoire après exploration et
téléportations. Couvrir animations, eau, feuillage, mods et structures mobiles.

### Lot 8 — terrain lointain issu du même monde

**Dépendance :** lots 4 à 7. **Livrable :** horizon cohérent et raffinement progressif.

- Utiliser les mêmes requêtes de relief, densité, eau et matériaux. Une simple
  hauteur ne représente pas tous les volumes : préserver des intervalles occupés
  ou une approximation volumétrique pour les surplombs visibles aux niveaux proches.
- Donner priorité aux résumés des blocs réellement observés et modifiés, puis à
  la génération détaillée disponible, puis aux estimations du générateur.
- Afficher progressivement végétation et structures connues. Garder les
  approximations de zones inconnues distinguables dans les outils de diagnostic.
- Prioriser l'erreur projetée et la visibilité, garder hystérésis et raccords entre
  niveaux, borner les caches et conserver une couverture jusqu'au remplacement.
- Appliquer les budgets de CPU et GPU des lots 6 et 7 au lointain aussi.

**Passage :** mêmes côtes, vallées et masses montagneuses de loin et de près,
constructions modifiées prises en compte, pas de trou temporaire ou de double
surface visible lors des transitions, exploration prolongée à mémoire bornée.

### Lot 9 — recette finale et activation

**Dépendance :** lots 2 à 8 validés. **Livrable :** nouveau générateur sélectionnable et documenté.

- Comparer plusieurs graines sur continents, montagnes, fleuves, lacs, forêts,
  falaises et zones constructibles ; conserver aussi les cas défavorables.
- Fournir cartes macro, profils de rivières, coupes et captures depuis les mêmes
  points de vue. Vérifier les packs 16 et 32, le jour et la nuit.
- Parcourir des mondes de test isolés : création, sauvegarde, réouverture, retour
  dans une zone modifiée, chargement dans un ordre différent et changement de dimension.
- Mesurer en release : distances 8, 16 et 32 chunks, parcours fixes et téléportation,
  CPU/GPU séparés, latence des chunks et temps d'image médian/p95/p99. Identifier
  machine, résolution et réglages ; comparer chaque optimisation sur le même
  générateur et les mêmes scènes, car v1 et v2 ne produisent pas la même géométrie.
- Exécuter les six contrôles du projet ci-dessous. Documenter les mesures et les
  vérifications graphiques impossibles. Activer v2 seulement après cette recette.

## Critères de réussite et validation continue

| Axe | Vérification requise |
| --- | --- |
| Déterminisme | Blocs et données macro identiques en séquentiel, parallèle, ordre inverse, caches vides/pleins/évincés. |
| Raccords | Même résultat pour régions séparées ou réunies, bords et coins, y compris aux coordonnées négatives. |
| Hydrologie | Graphe acyclique, terminaison déclarée, surface d'eau sans remontée accidentelle, confluences et lacs cohérents. |
| Coût | Budget et dépendances bornés à froid ; aucun travail proportionnel à un continent entier pour demander un chunk. |
| Naturel | Chaînes, piémonts, bassins et côtes lisibles aux cartes et en jeu ; plusieurs graines examinées. |
| Compatibilité | Génération historique et modifications du joueur conservées ; données de mods résolues explicitement. |
| Affichage | Latence demande → première image, p95/p99, draw calls, octets transférés, mémoire et coût GPU suivis. |
| Rendu fidèle | Images de référence avant/après pour les optimisations à géométrie constante ; vérification native sur Mac. |

Les cibles chiffrées de vitesse seront fixées après les mesures du lot 0. Aucun
facteur de gain ou nombre de FPS n'est déjà démontré. Distinguer amélioration
esthétique, débit de génération, délai d'apparition et fluidité d'image.

Contrôles avant livraison de chaque lot qui modifie le code :

```sh
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo xtask check-arch
cargo xtask hooks --check
cargo xtask docs --check
```

Chaque lot a un commit français, ses preuves et ses limites. Ne pas mettre à jour
des empreintes de référence uniquement pour faire passer un test : expliquer le
changement attendu et garder la référence historique intacte pour v1.

## Ordre et découpage de livraison

| Livraison | Lots | Résultat visible |
| --- | --- | --- |
| A | 0 puis 1 | Versions protégées, cartes de contrôle et calculs réutilisés. |
| B | 2 puis 3 | Géographie et drainage raccordés, validés avant les blocs. |
| C | 4 puis 5 | Relief volumétrique, matériaux, biomes et végétation cohérents. |
| D | 6 puis 7 ; à partir du lot 1 | Arrivée des chunks et coût d'affichage optimisés. |
| E | 8 puis 9 | Horizon fidèle, recette complète et activation de v2. |

Première tranche à exécuter : lots 0 et 1, puis prototype macro des lots 2 et 3.
Le graphe hydrologique est le point de passage critique. Les travaux de rendu
peuvent avancer ensuite sans dépendre des réglages artistiques du relief.

Après cette base seulement : compression de quads éligibles, culling GPU selon les
capacités et résultats mesurés, formes karstiques ou volcaniques supplémentaires,
érosion plus élaborée. Une simulation géologique complète, une érosion globale en
temps réel et l'augmentation de la hauteur du monde ne sont pas des prérequis.

## Statut de la livraison A (lots 0 et 1)

Exécutée le 18 novembre 2026, **non commitée** (en attente de validation). Les six contrôles du
projet passent (voir le rapport de livraison). Aucun bloc de la génération v1 n'a changé : les
empreintes de référence ont été capturées sur le générateur d'avant le lot 1 puis figées.

### Lot 0 : ce qui est fait

- **Version de génération** : [`generation.rs`](../crates/engine_voxel/src/generation.rs),
  `GenerationVersion` (seule `V1` existe), champ `WorldMeta::generation` (`Option`, `#[serde(default)]`),
  séparé de `format_version`. Absent : `GenerationVersion::LEGACY` (v1), explicitement, sans migration.
  Un numéro inconnu est refusé au chargement. Test : un `world.ron` écrit à la main sans le champ, et un
  `WorldMeta` sérialisé puis amputé du champ, sont relus en v1. Les nouveaux mondes prennent
  `GenerationVersion::FOR_NEW_WORLDS` (v1 aujourd'hui : le choix explicite d'un futur v2 se fera en
  changeant cette constante ou en passant la version à la création ; aucune interface de choix n'existe).
- **Configuration effective** : `GenerationMeta` garde, par dimension, la clé, le décalage de graine et
  le genre de monde en identifiants texte, et, pour tout le monde, l'empreinte (FNV-1a 64 bits) des
  fichiers `biomes.ron`, `ores.ron`, `structures.ron` du jeu de base (embarqués) et `biomes.ron`,
  `ores.ron`, `structures.ron`, `dimensions.ron` de chaque mod. À l'ouverture
  ([`dimensions.rs`](../games/voxel/src/dimensions.rs)), `GenerationMeta::compare` signale dans les
  logs un fichier retiré, modifié ou ajouté, et une dimension retirée ou modifiée. La référence
  enregistrée est conservée telle quelle (elle n'est pas réécrite quand un mod change : l'écart reste
  visible à chaque ouverture) ; un monde d'avant le champ adopte la configuration d'aujourd'hui comme
  référence à sa première sauvegarde.
- **Empreintes de référence** : [`generation_reference.rs`](../crates/engine_voxel/tests/generation_reference.rs)
  et sa table figée `tests/support/reference_v1.rs` : 36 colonnes de chunks (6 chunks chacune) sur les
  graines 7 et 42 (8 colonnes : origine, négatives, frontière des régions de sauvegarde, lointaines) et
  5 graines de corpus (1, 1337, 20260, 987654321, 4000000000 ; 4 colonnes), plus l'empreinte d'une carte
  33 × 33 de hauteurs et biomes par graine (coordonnées de -600 à +584). Le hash passe par les
  identifiants texte des blocs et des biomes, pas par leurs numéros.
- **Délais du pipeline** : [`pipeline_timing.rs`](../crates/engine_voxel/src/pipeline_timing.rs).
  `ChunkStreamer::set_pipeline_timing(true)` (en jeu : `VOXEL_PIPELINE=1`) mesure pour chaque chunk
  demande → généré → maillage lancé → maillé → envoyé au GPU (`note_uploaded`), soit les étapes
  génération, lumière (attente des 9 colonnes), maillage, envoi, total ; médiane, 95e centile et
  maximum sur une fenêtre de 4 096 mesures, journalisés toutes les 2 secondes avec les stats de
  l'image. Éteint par défaut (un test sur un `Option` par étape), sans dépendance nouvelle ; les spans
  d'`engine_trace` (`VOXEL_TRACE`) restent la mesure de durée de travail.
- **Cartes sans fenêtre** : `cargo xtask map` (exemple
  [`worldmap.rs`](../crates/engine_voxel/examples/worldmap.rs), module
  [`worldmap.rs`](../crates/engine_voxel/src/worldmap.rs), lancé en release). Altitude, pente, humidité,
  température, continentalité et biomes d'une zone, en PNG (ou PPM) ; légende des biomes dans
  `biomes.txt`.

  ```sh
  cargo xtask map                                        # graine 42, 512 × 512 pixels de 4 blocs, dans target/maps/42
  cargo xtask map --seed 7 --x -2048 --z -2048 --size 1024 --step 4 --maps altitude,biomes --out target/maps/essai
  ```
- **Isolation des sauvegardes** : vérifiée. `VOXEL_WORLD` ne change que le nom du dossier
  (`saves/<nom>`) ; la racine venait de `<dossier de assets>/../saves`, aussi codée en dur dans le menu
  (liste, création) et le décor du menu. Ajouté : **`VOXEL_SAVES=dossier`**, racine des sauvegardes
  (`persistence::saves_root`), utilisée partout. Les modes automatiques (`VOXEL_BENCH`, `VOXEL_DEMO`,
  `VOXEL_SCREENSHOTS`, `VOXEL_AUTOFLY`) n'ouvrent aucun monde sauvegardé sauf si `VOXEL_WORLD` est donné.
  Séance graphique isolée : `VOXEL_SAVES=$(mktemp -d) VOXEL_WORLD=essai cargo run --release -p voxel`.
  Reste non isolé : `settings.ron` et `mod_settings.ron` (écrits à la racine du projet quand on change
  une option) et la capture d'écran ; `VOXEL_ASSETS` déplace la racine du projet (donc aussi ces
  fichiers) mais pas les mods (`../mods`). Une séance graphique qui touche aux options modifie donc
  `settings.ron`.

### Lot 1 : ce qui est fait

- **Cache des colonnes** : [`generator/column_cache.rs`](../crates/engine_voxel/src/generator/column_cache.rs).
  Les 32 × 32 colonnes (hauteur, biome, températures), la hauteur maximale, les biomes en octets et
  l'altitude maximale des grottes sont calculés une fois par colonne de chunks et partagés par ses six
  chunks. Instantané immuable (`Arc<ColumnGrid>`), clé explicite (graine, empreinte du genre de monde et
  des biomes de terre = la dimension, version de génération, cx, cz), 1 024 entrées au plus (environ
  17 Mio), éviction de la plus ancienne. La table est sous `Mutex` mais le calcul se fait hors verrou,
  dans une `OnceLock` par clé : deux workers qui veulent la même colonne la calculent une fois (le
  second attend le premier ; le thread de rendu n'attend rien de plus qu'avant, il recalculait seul).
  Les mondes plats et vides (colonnes constantes) ne passent pas par le cache.
- **Bruits de grottes à la demande** : ils ne sont calculés que si le chunk a des blocs sous le plafond
  des grottes (`origin.y <= max_cave_ceiling`) ; les chunks du haut des reliefs et du ciel les évitent.
- **Tampons par worker** : les quatre tableaux de bruits de grottes (environ 12 Kio) sont réemployés par
  thread (`thread_local`), sans verrou, sans valeur conservée d'un chunk à l'autre.
- **Preuve** : `generation_reference.rs` vérifie les empreintes en ordre séquentiel, **inverse**
  (chunks du haut vers le bas), **parallèle** (4 threads se disputant les mêmes colonnes, ordres
  différents), cache **vide**, **plein**, **vidé** (`clear_caches`) et **évincé** (au-delà de 1 024
  colonnes), et que `columns` égale `column(x, z)` aux bords et en négatif. Aucune empreinte n'a été
  modifiée.

### Mesures (Apple M3 Pro, 12 cœurs, rustc 1.99.1, `cargo bench -p engine_voxel --bench generation`)

Milieu de l'intervalle criterion. « À froid » : `clear_caches()` avant chaque mesure (situation du
streaming dans une zone neuve) ; « à chaud » : l'itération répétée sur le même chunk, donc ses colonnes
sont déjà en cache après la première (avant le lot 1 il n'y avait rien à mettre en cache : ces lignes
montrent le gain maximal, pas celui du jeu).

| Mesure | Avant | Après | Écart |
| --- | --- | --- | --- |
| **Verticale de 6 chunks, à froid** | 2,843 ms | 1,595 ms | -44 % |
| **Zone 8 × 8 colonnes (384 chunks), à froid** | 128,65 ms | 83,98 ms | -35 % |
| Zone 8 × 8 colonnes, à chaud | 124,64 ms | 72,20 ms | -42 % |
| Chunk du fond, à chaud | 761 µs | 529 µs | -30 % |
| Chunk souterrain, à chaud | 753 µs | 532 µs | -29 % |
| Chunk de surface, à chaud | 679 µs | 459 µs | -32 % |
| Chunk du ciel, à chaud | 228 µs | 16,4 µs | -93 % |
| Surface forêt / jungle, à chaud | 502 / 527 µs | 308 / 338 µs | -39 % / -36 % |
| Maillage dense forêt / jungle (non concerné) | 1,649 / 2,033 ms | 1,705 / 2,114 ms | bruit (+3 à +4 %) |
| Lumière d'une colonne (non concerné) | 761 µs | 783 µs | bruit |

Un second passage des mesures à froid donne 1,615 ms et 86,7 ms. Le calcul des 1 024 colonnes d'un chunk coûte environ 250 µs sur les 650 µs d'un chunk d'altitude : le
cache en supprime 5 sur 6 par verticale. Un bench de jeu (`cargo xtask bench`) n'a pas été lancé (il
ouvre une fenêtre).

### Limites et ce qui reste

- **Lot 0** : pas de mesure du délai jusqu'à la première image contenant le chunk, ni de relevé GPU
  (timestamps), draw calls, allocations ou mémoire du pipeline : seul le chemin jusqu'à l'envoi au GPU
  est chronométré. Les cartes n'exportent pas côtes, bassins ni écoulements (ils n'existent pas encore :
  lots 2 et 3). Les données des mods ne sont **pas copiées** dans la sauvegarde : l'empreinte détecte
  un changement mais ne reconstruit pas l'ancienne configuration. Pas d'option de choix de version à
  la création d'un monde (une seule version). Le chargement du monde se rabat sur la génération sans
  mods si leurs données sont invalides (`modding::terrain`) : la configuration effective décrite
  dans `world.ron` est alors celle des fichiers, pas celle réellement utilisée.
- **Lot 1** : seules les colonnes sont réutilisées ; les plans de structures (sites, blueprints), les
  sites d'arbres (cache existant, politique d'éviction inchangée : vidange complète à 8 192 entrées) et
  les points de grille partagés entre chunks voisins (environ 21 % des évaluations de bruit d'une grille
  de 9 × 9) ne sont pas mis en cache. La clé « configuration » est l'empreinte du genre de monde et des
  biomes de terre ; chaque générateur étant construit une seule fois avec sa configuration, cela suffit,
  mais le cache n'est pas partagé entre générateurs. Les mesures de mémoire en jeu et de latence réelle
  du streaming restent à faire en séance graphique.
- **Trouvaille** : une graine proche de `u32::MAX` (par exemple 4294967295) fait paniquer le bruit en
  debug (`noise` additionne l'indice d'octave à la graine sans wrapping) ; en release le calcul déborde
  silencieusement. Préexistant, non corrigé ; le corpus utilise donc 4000000000.

## Statut de la livraison B (lots 2 et 3)

Exécutée le 18 novembre 2026, **non commitée**. C'est un **prototype macro** : géographie et drainage
définis en coordonnées monde, validés par des cartes et des tests, **sans aucun bloc** (lot 4) ni biome
v2 (lot 5). Le générateur v1 est strictement inchangé (`generation_reference` passe, table
`reference_v1.rs` intacte) ; il n'existe pas de variante `GenerationVersion::V2` (donc rien n'active v2,
`FOR_NEW_WORLDS` reste V1) ; seuls `cargo xtask map --generator v2`, les tests et un bench l'utilisent.

Fichiers : `generator/macro_noise.rs` (bruit à nous, sans dépendance), `generator/geography.rs` (lot 2),
`generator/hydrology.rs` (lot 3 : types, cellules, niveaux, accumulation, lacs), `generator/hydrology/regions.rs`
(bassins de forme libre, graphe, arbres), `generator/hydrology/rivers.rs` (tracé des cours à l'échelle des
blocs), `generator/context.rs` (`MacroWorld` : requête de point avec vallées et lacs), `worldmap_macro.rs` et
l'exemple `worldmap.rs` (cartes), `tests/macro_world.rs` (16 tests), `benches/hydrology.rs`.

**Révision du réseau (18 novembre 2026, après les premières cartes).** La première version du lot 3 passait
tous ses tests mais se lisait mal : bassins carrés de 512 blocs, cours en peignes de tronçons alignés sur la
grille de 64 blocs, aucun fleuve. Le modèle hydrologique a été refait (bassins de forme libre, arbres de
moindre coût, courbes) ; la conception ci-dessous est la version révisée, la géographie du lot 2 n'a changé
que par l'atténuation des tourbillons des fonds marins.

### Lot 2 : la géographie (`Geography`)

- **Échelles indépendantes** (blocs) : continents 6 400 de longueur d'onde (avec un « warp » lent de 700) ;
  détail des côtes 520 ; chaînes 2 400 en travers (étirées ×2,2 le long de l'axe) et crêtes secondaires
  1 700 ; détail fin 150. Le relief va de la mer (niveau 62) à 62 + 120 au plus, sous la hauteur de 192
  blocs (écrêtage **doux** exponentiel à partir de 85 : pas de sommet tronqué, test : moins de 0,5 % des
  terres au plafond).
- **Terre/mer et distance au rivage** : champ continental `C` (terre où `C > 0`), distance **estimée**
  `C / |∇C|` (gradient de la part lisse, plancher doux), signée, bornée à ±2 600 blocs. Le détail des
  côtes ne s'ajoute qu'au voisinage du rivage, pour garder fonds marins et plateaux lisses. Courbe de
  base cubique monotone (Fritsch-Carlson) : plateau continental, plage, plaine côtière, plateau intérieur.
- **Chaînes** : axe = zéro d'un bruit lent (orientations fondues entre deux directions **fixes** : une
  rotation dépendant de la position tourne autour de l'origine et tord le monde, première version
  rejetée), distance à l'axe estimée, intensité par un champ séparé (extrémités de chaînes), **cols** par
  un troisième champ le long de l'axe, **crêtes secondaires** (second axe limité au voisinage du
  principal). Profil perpendiculaire cubique monotone plaine → piémont → massif, avec crête. Le bruit
  fin (amplitude croissant avec la hauteur de chaîne) module sans changer la hiérarchie.
- **Familles de relief** (`ReliefFamily`) : mer profonde, plateau continental, côte, plaine, plateau,
  piémont, massif.
- Tout est une fonction pure de `(graine, x, z)` : aucune région, aucun état.

### Lot 3 : conception de l'hydrologie (`Hydrology`) et pourquoi elle tient ses promesses

Trois niveaux : des **cellules fines** de 128 blocs (nœud jitteré entre 0,3 et 0,7 de la cellule, altitude
macro), des **bassins** de forme libre (une soixantaine de cellules en moyenne, de 4 à 150) et les
**bassins versants** (l'ensemble des bassins qui se vident au même exutoire).

1. *Des bassins sans grille.* Chaque super-cellule de 8 × 8 cellules (1 024 blocs) a une **graine** : un
   point tiré au hasard dans la super-cellule, ramené à la cellule la plus basse de son voisinage de 3 × 3.
   (Première version : la cellule la plus basse d'un cœur de 6 × 6. Sur un terrain en pente, elle rangeait
   toutes les graines au même coin et redessinait un réseau carré ; le centre aléatoire l'évite.) Chaque graine a
   un poids de distance entre 0,7 et 1,4 (bassins de tailles variées, frontières courbes). Une cellule a pour
   attracteur la graine la plus proche (distance pondérée, parmi les 7 × 7 super-cellules voisines par
   couronnes élaguées ; la borne de distance qui justifie cette fenêtre est démontrée dans la documentation du
   module et recoupée par un test contre une recherche plus large). Une cellule qui n'est pas une graine
   **descend** : parmi ses voisins strictement plus proches de son attracteur, elle prend le plus bas. Son bassin
   est celui où la descente aboutit. Les frontières suivent donc un peu le relief (la pente décide près d'une
   bissectrice) et ne suivent plus aucune grille.
   - *Connexe par construction* : chaque cellule est reliée à sa graine par des cellules du même bassin, donc
     jamais d'orphelin (c'était l'écueil des régions « par distance » : une cellule sans voisin plus proche dans son
     bassin). Le parcours depuis la graine retrouve tout le bassin.
   - *Terminaison bornée* : la distance pondérée à la graine la plus proche décroît d'au moins 0,49 en carré à
     chaque pas, depuis 192 au plus : 392 pas au plus (`MAX_OWNER_STEPS`) ; l'appartenance d'une cellule ne regarde
     que des cellules voisines et des graines proches, donc c'est une fonction pure de `(graine du monde, cellule)`.
2. *Graphe des bassins.* Clé totale d'un bassin : `(altitude de la graine + 0,012 × distance au rivage, hachage,
   cellule)`. Un bassin s'écoule vers celui de ses voisins (en contact par une arête de cellules) de clé
   strictement plus basse, par la **passe la plus basse** de leur frontière : acyclique, car la clé décroît strictement
   le long de chaque arête. Sans voisin plus bas : **puits déclaré**, lac terminal. Un bassin dont une cellule est en
   mer s'y vide (puits océanique : ses cellules de mer sont les racines de son arbre, ce qui supprime les détours
   le long de la côte de la première variante). La pente vers le rivage fait préférer la mer aux cuvettes
   fermées (4 à 5 % de cellules terrestres vers un lac terminal, contre 21 % avant).
3. *Profondeur bornée par définition.* Une marche d'au plus `MAX_BASIN_HOPS` + 1 = 49 bassins mesure la
   profondeur ; un bassin plus profond devient lui-même un puits (« lac plafonné », ses amonts aussi, ses avals
   gardent leur arête). Résoudre un bassin ne regarde jamais plus de 49 bassins : pas d'exploration d'un continent.
4. *Arbres de moindre coût.* Dans un bassin, les cellules forment un arbre enraciné à l'exutoire (ou, pour un
   puits, à la cellule la plus basse du bassin : le lac y est centré, l'eau ne remonte jamais vers la graine), plus
   les cellules de mer. Le coût d'un pas pénalise l'altitude relative de la cible (`longueur × (1 + 5 × altitude relative)
   × (1 + 0,45 × tirage)`) : les cours suivent les fonds de vallée, les affluents descendent les versants et se
   fondent dans le cours principal (dendrites arborescentes, pas de peigne). L'arbre est un arbre couvrant d'un
   bassin connexe : ni cycle ni minimum local. La cellule de sortie passe à une cellule voisine du bassin aval
   (l'entrée), qui est une cellule ordinaire de son arbre : le fleuve traverse chaque bassin par le chemin le
   moins coûteux et les affluents s'y raccordent.
5. *Contrat de bord* (`EdgeContract`) : fonction pure de l'arête ordonnée (A, B) : cellules de sortie et d'entrée
   (voisines, 8 voisins), identité du bassin versant, accumulation, largeur, niveaux d'eau de part et d'autre
   (`level_out >= level_in`). Les deux voisins appellent la même fonction.
6. *Profil d'eau.* Niveau d'une cellule = maximum, sur son chemin vers l'exutoire, de `altitude du nœud - 1,5`,
   plancher à la mer ou au niveau du lac terminal : il **ne remonte jamais vers l'aval** (flottant et après
   `floor`). Si le terrain monte vers l'aval, le terrain est rehaussé au lot 4 (remblai), pas l'eau ; en attendant,
   `MacroWorld::sample` plafonne le remblai à 2 blocs (`MAX_FILL`) pour ne pas dessiner de digues sur les cartes.
7. *Accumulation* : nombre de cellules en amont (elle comprise, les bassins amont inclus par leur total à la cellule
   d'entrée), exact jusqu'à `ACCUMULATION_CAP` = 1 024 puis saturé : croissante vers l'aval, couvre la somme des
   affluents (vérifié). Le total d'un bassin amont se calcule en s'arrêtant dès la saturation (le résultat, un
   minimum, ne change pas) : le travail est borné par la saturation. Largeur = `2,5 + 1,3 × acc^0,55` (au plus 64
   blocs) ; une embouchure (près du rivage, niveau proche de la mer) s'évase jusqu'au double ; un ruisseau naît
   progressivement (étroit, sans vallée à sa source).
8. *Lacs.* **Terminaux** : tout puits non océanique (rayon 130 à 260, niveau de sa cellule la plus basse).
   **De passage** : cellule de rivière tirée (5 %), isolée dans un voisinage de 7 × 7, rayon 0,4 à 0,95 cellule, au
   niveau de la rivière ; son déversoir est l'arête aval (palier). Rivage déformé par un bruit.
9. *Tracé des cours* (`hydrology/rivers.rs`). L'arête `g → n` est une courbe cubique d'Hermite entre les nœuds : elle
   part dans la direction de sa propre arête et arrive dans celle de l'arête suivante de `n` (un cours continue
   sans cassure à chaque nœud, un affluent rejoint le cours principal tangentiellement ; sur un virage serré, la
   tangente se rapproche de la corde pour éviter un crochet). Un décalage perpendiculaire borné (10 % de la corde,
   nul aux nœuds) y met des méandres. Fonction pure de `g`, `n` et de l'arête suivante. Les cours ne se croisent
   pas (testé : aucun croisement sur plus de 1 000 paires de cours voisins par graine).
10. *Vallées* (`MacroWorld::sample`) : bords de lit et berges selon la distance à la rivière (pente douce en plaine
    alluviale, raide en vallée encaissée), cuvette de lac ; l'altitude se fond dans le terrain sur 56 blocs (moins
    d'une cellule : les 5 × 5 cellules voisines suffisent), atténuée à la source des ruisseaux.
11. *Bassin d'un point* (`Hydrology::basin_at`) : le bassin de la cellule dont le nœud est le plus proche du point
    (déplacé d'au plus ~70 blocs par un bruit), pour que les frontières des cartes ne montrent ni marches de
    128 blocs ni arêtes droites.

### Tests du lot 3 (`tests/macro_world.rs`, 5 graines : 7, 42, 1337, 20260, 987654321)

| Exigence | Test | Résultat |
| --- | --- | --- |
| Déterminisme (séquentiel, inverse, 4 threads, caches vides, vidés en route, pleins, évincés à 64 entrées) | `samples_are_identical_in_sequence_reverse_parallel_and_any_cache_state`, `hydrology_cells_do_not_depend_on_evaluation_order` | passent, égalité exacte |
| Raccords (régions séparées/réunies aux bords et coins de quatre régions, négatifs compris ; contrat lu par les deux voisins) | `separate_regions_agree_with_a_united_region_at_edges_and_four_region_corners`, `edge_contracts_are_read_identically_by_both_neighbours` | passent (5 coins dont 3 en négatif ; plus de 200 arêtes par graine) |
| Acyclicité, terminaison déclarée et bornée | `every_path_is_acyclic_and_ends_at_a_declared_sink_within_the_bound` | 1 500 cellules terrestres par graine : aucun cycle ni cours interrompu ; chaque pas intérieur descend l'arbre d'exactement 1 ; profondeur max 5 à 8 bassins (borne 48) ; plus long chemin 38 à 59 cellules ; aucun puits plafonné |
| Aucune remontée, confluences, lacs | `water_never_climbs_downstream_even_after_rounding_to_blocks`, `lakes_have_a_level_and_an_outlet_or_are_declared_closed`, `basin_ids_label_drainage_basins_consistently` | passent |
| Coût borné | `a_cold_query_evaluates_a_bounded_number_of_cells`, `resolving_a_basin_looks_at_a_bounded_number_of_basins` | voir les mesures |
| Géographie (continuité, terres 20-60 %, sommets non écrêtés, massifs) | `geography_is_continuous_across_region_borders_and_in_negative_coordinates`, `land_sea_chains_and_summits_look_like_a_world` | passent |
| Bassins de forme libre, connexes | `basins_are_free_form_and_connected_not_grid_squares` | 92 à 126 bassins par graine : au plus 1 rectangle, remplissage moyen 0,64 à 0,67, 37 à 40 % des cellules hors de leur super-cellule, tailles de 4 à 152 cellules ; tout chemin d'un bassin reste dans l'arbre |
| Cours organiques, sans croisement | `rivers_are_not_aligned_on_the_grid_and_do_not_cross` | 14,9 à 16,9 % de segments à moins de 5 degrés d'un axe (répartition uniforme : 11 % ; première version : 24 à 26 %) ; aucun croisement |
| Fleuves et affluents arborescents | `main_rivers_collect_tributaries_and_widen_downstream` | plus grand fleuve de 296 à 652 cellules ; cours principal de 18 à 46 cellules, 8 à 28 affluents ; largeur croissante vers l'aval |
| Embouchures larges | `estuaries_widen_and_inland_rivers_do_not` | 109 embouchures évasées sur 132 ; 56 cours intérieurs sur 56 sans évasement |
| Briques (modules) | tests unitaires de `regions.rs` et `rivers.rs` | graine la plus proche recoupée (11 × 11 super-cellules), bassins connexes, frontières symétriques, courbes pures et bornées |

### Mesures (Apple M3 Pro, release, `cargo bench -p engine_voxel --bench hydrology`, contexte neuf)

| Mesure | Avant (révision) | Après |
| --- | --- | --- |
| Requête de point à froid, massif / plaine côtière / intérieur | 1,55 ms / 2,27 ms / 2,14 ms | 0,74 ms / 1,38 ms / 0,99 ms |
| Géographie seule (un point) | environ 0,8 µs | environ 0,64 µs |
| Résolution d'un bassin à froid | 0,43 ms | 0,28 ms |
| Zone de 1 024 × 1 024 blocs à froid (4 096 points, pas de 16) | 12,2 ms | 14,6 ms |
| Altitudes évaluées par une requête à froid (600 points) | 2 443 en moyenne, 5 375 au pire | 1 732 en moyenne, 4 504 au pire |
| Résoudre un bassin (altitudes, au pire sur 1 500 bassins) | 1 764 | 1 354 |

Le coût n'augmente pas : les bassins sont plus grands et moins nombreux le long d'un chemin (5 à 8 au lieu de
16), un bassin se parcourt en une fois et les arbres sont partagés par toutes les cellules. La borne analytique
(constantes : 49 bassins par chemin, 10 voisins amont saturés, `MAX_BASIN_CELLS` cellules par bassin) est
astronomique (3 × 10^12) : le test fige une borne mesurée de 10 000 altitudes par requête et 3 000 par bassin, des
constantes environ 2,2 fois supérieures au pire observé. Une carte de 16 384 blocs de côté (1 024 × 1 024 pixels) se
calcule en 2,5 à 3,5 s sur 12 cœurs, comme avant.

### Lisibilité des cartes (jugement)

Trois graines (42, 7, 1337), vue large de 16 384 blocs et zoom de 4 096 blocs ; cartes `bassins`, `drainage`,
`altitude`, `lacs` regardées à chaque itération (voir `scratchpad/drainage/avant` et `apres`).

Ce qui est résolu : (1) les bassins sont des blobs irréguliers de 4 à 150 cellules, qui ressemblent à des
bassins versants, plus aucun carré ni aucune marche de grille de 512 blocs ; (2) les cours sont des courbes
continues avec des confluences tangentielles et des méandres, 15 % de segments seulement proches d'un axe
(au lieu de 25 %, et de 66 % de pas entre cellules voisines par un côté) ; (3) un réseau arborescent : de vrais cours
principaux de plusieurs dizaines de cellules qui collectent 10 à 28 affluents, des ruisseaux qui s'élargissent vers
l'aval, des embouchures évasées, quelques fleuves par continent ; (4) les digues dessinées par le remblai ont
disparu des cartes d'altitude (plafond de 2 blocs) ; (5) les tourbillons des fonds marins sont presque effacés.

Ce qui reste : (a) les frontières de bassins sont des chaînes de cellules de 128 blocs : à fort zoom (4 blocs
par pixel) elles restent dentelées ; elles suivent le relief seulement près des bissectrices entre graines, pas les
crêtes à proprement parler ; (b) la largeur est une fonction continue de l'accumulation, qui croît à chaque
confluence : l'ordre de Strahler n'est pas calculé, seulement approché ; (c) les plus grands fleuves ont 300 à 650
cellules amont (continents de 6 000 à 10 000 blocs, chaque bassin côtier a son embouchure) : pas de fleuve de la
taille d'un continent ; (d) un cours qui traverse une crête creuse un canyon, et le profil d'eau ne remonte jamais : les
dépressions en amont d'une passe devraient être des lacs de remplissage (ce n'est pas modélisé, le remblai est
plafonné) ; (e) pas de deltas ni de méandres de plaine au-delà de la courbe ; (f) les chaînes sont plutôt des
massifs en arcs que de longues cordillères continues (lot 2) et quelques traînées subsistent sur les fonds marins
près des îles.

### Limites et ce qui n'est pas fait

- Aucun bloc (lot 4), aucun biome ni climat (lot 5), aucun lien avec les structures, les fluides stables
  ni les modifications du joueur : l'hydrologie statique n'est qu'un champ interrogeable.
- Pas de variante `GenerationVersion::V2` ni de choix de générateur à la création d'un monde (ajoutés au lot 4).
- La borne de profondeur (48 arêtes de bassin, environ 40 000 blocs) est une définition, pas un phénomène
  observé : elle n'est jamais atteinte sur l'échantillon (profondeur maximale 8). Un bassin plafonné deviendrait un
  lac terminal ; aucun test n'en a vu. De même `MAX_OWNER_STEPS` (392) et `MAX_BASIN_CELLS` sont des bornes
  démontrées mais très larges ; les tailles réelles (au plus 155 cellules, chemins d'au plus 60 cellules) sont
  figées par des tests mesurés.
- La largeur est limitée à 64 blocs (accumulation saturée à 1 024 cellules) : pas de fleuve plus large.
- Le total d'un bassin amont s'arrête à la saturation, mais une requête près d'un grand fleuve peut parcourir
  plusieurs dizaines de bassins amont à froid (non mesuré comme cas séparé : le bench le couvre seulement
  quand le point tombe près d'un cours).
- Pas de mesure de mémoire en jeu (le prototype n'est pas branché au streaming) ; les tables de cache sont
  vidées au-delà de 262 144 entrées chacune (12 tables).

## Statut du lot 4 (relief 3D, surface et sous-sol)

**Ce qui est livré :** `GenerationVersion::V2` et un générateur de blocs v2 (`generator/v2.rs`,
`v2_context.rs`, `scarp.rs`, `density.rs`, `surface.rs`, `underground.rs`), des cartes et coupes issues des **blocs**
(`worldmap_blocks.rs`, `cargo xtask map --generator v2`), des tests dédiés (`tests/generation_v2.rs`,
`tests/relief_v2.rs` pour les parois, surplombs, arches et la forme des parois, avec `tests/support/walls.rs`) et des empreintes v2 figées
(`tests/support/reference_v2.rs`). `FOR_NEW_WORLDS` reste **v1** : rien n'est activé pour
les joueurs (lot 9).

### Créer un monde de test v2 et aiguillage

- À la **création** d'un monde, `GenerationVersion::for_new_world()` lit la variable d'environnement
  `VOXEL_GENERATION` (`2` ou `v2` : monde de test v2 ; absente, `1` ou une valeur inconnue : v1). Elle n'est lue
  qu'à la création ; la version est écrite dans `world.ron` (`generation.version`).
- À l'**ouverture**, `WorldMeta::generation_version()` décide : monde enregistré v2 -> v2 (la variable est sans
  effet) ; monde v1 ou sans métadonnée de génération -> v1 ; numéro inconnu -> refus. Le jeu passe la version à
  `modding::terrain` (`games/voxel/src/main.rs`), qui appelle `TerrainGenerator::with_dimension_versioned`.
  Testé avec des dossiers temporaires dans `games/voxel/src/dimensions.rs` et dans `generation.rs`.
- Pour l'API : `TerrainGenerator::with_version(GenerationVersion::V2)` ; les outils (`worldmap`, benchs,
  tests) l'utilisent directement.

### Conception

1. **Contexte de colonne** (`ColumnContext`, `v2_context.rs`) : calculé une fois par colonne de chunks (32 x 32
   plus 8 blocs de marge) et partagé par ses six chunks, les arbres et les structures. Cache borné (512 entrées,
   clés explicites graine/dimension/version/cx/cz, évincé du plus ancien, calcul hors verrou), vidé par
   `clear_caches`. Chaque valeur est une fonction pure de la position.
2. **Relief** : géographie macro tous les 4 blocs (interpolée), cours et lacs lus une fois par zone
   (`Hydrology::river_candidates`), vallées et lits comme `MacroWorld::sample`, mais remblai des berges jusqu'à
   10 blocs et lits creusés **en blocs**. Détail de massif là où la chaîne est forte (atténué dans les
   vallées) : des bosses douces (une vingtaine de blocs de haut, 90 blocs de long) et un relief rugueux plus
   fin, d'intensité lentement variable. Ni gradins réguliers (leurs paliers de 8 blocs donnaient l'allure de
   courbes de niveau), ni arêtes tirées d'un bruit (le « ridged » du premier jet serpentait en boucles et
   couvrait les cartes de bandes de roche).
   **Escarpements** (`scarp.rs`) : dans les massifs et les contreforts (chaîne >= 24 qui monte vers 46,
   altitude à plus de 8 blocs au-dessus de la mer), le relief **saute de 10 à 44 blocs** sur des parois qui
   suivent la **structure macro** de la géographie, jamais un bruit isotrope (une isoligne de bruit boucle
   partout où il oscille : vers, anneaux, murs doublés, c'était le défaut du premier jet). Trois familles :
   (1) les **flancs de chaîne**, lignes parallèles à l'axe (distance à l'axe `GeoSample::axis_distance`,
   déjà une distance) à 35 blocs de l'axe puis tous les 84 blocs, décalées lentement de ±28 blocs ; chaque
   ligne n'existe que par **plages** de quelques centaines de blocs (un masque lent propre à la ligne) et
   s'éteint aux bouts (la hauteur décroît avec le masque) ; (2) les **crêtes secondaires** (nouvelle valeur
   `GeoSample::ridge_distance`) : deux parois face à face de part et d'autre de l'axe d'une crête, qui la
   relèvent entre elles, large de 50 à 110 blocs et, par zones de 140 blocs, affinée jusqu'à 14 blocs (les
   arches) ; (3) de rares **failles** droites de 280 à 540 blocs (une case de 1 000 blocs sur trois environ,
   jamais à moins de 160 blocs d'une autre). Un zonage de 700 blocs réserve chaque zone aux flancs **ou** aux
   crêtes (les poids se croisent à zéro à la frontière) ; une faille et une autre paroi s'effacent toutes deux
   à moins d'une trentaine de blocs l'une de l'autre : pas de croisement en grille, pas de mur doublé hors des
   deux faces d'une crête fine. Le profil : une marche franche de `H` blocs sur la ligne (un à deux blocs de
   large) compensée par une rampe de 30 blocs de part et d'autre (côté bas seulement pour une crête), si bien
   que l'effet s'éteint à 30 blocs de la ligne (une falaise suivie d'un glacis, comme une cuesta) ; les
   déplacements des deux parois d'un point se somment (aucune distance ne saute quand l'une cède la place à
   l'autre) ; les 3 premiers blocs de `H` sont retranchés (aucune marche de moins de 3 blocs). Le pied n'est
   jamais ramené à moins de 12 blocs de la mer, ni le sommet au-dessus du plafond du monde. `H` croît avec la
   chaîne. Jamais dans une vallée (le ressaut s'efface avant 40 blocs d'un cours ou d'un lac), un lit ni une
   zone réservée.
3. **Eau** : l'eau d'un cours est à `floor(niveau)` (le niveau ne remonte pas vers l'aval, l'arrondi le
   conserve) ; celle d'un lac à `floor(niveau du lac)` partout. Toute colonne de terre voisine d'une eau plus
   haute est relevée à son niveau : l'eau ne déborde jamais d'un côté, le seul écoulement est vers l'aval.
4. **Réservation** : un poids 0..1 (plein dans le lit, les berges et les lacs, décroissant sur 6 blocs). Il
   annule les escarpements et leurs entailles, impose 10 blocs de roche pleine sous un lit, et fixe la distance de
   dégagement des arbres et des structures (Chebyshev, 9 au plus).
5. **Densité et entaille du pied des parois** (`density.rs`) : le bloc `(x, y, z)` est solide quand `y <= top`,
   sauf dans l'**entaille** du pied d'une paroi. Une colonne d'escarpement connaît le pied de la paroi (`base`,
   relief sans le ressaut), la hauteur du ressaut (`jump`), la distance à la ligne de rupture (`dist`, positive du
   côté haut), la profondeur d'entaille `reach` (0 presque partout), l'exposant `shape` du profil et la fraction
   `lid` de la paroi qu'occupe l'entaille. Avec `t = (y - base) / jump` entre 0 et `lid`, le bloc est de l'air
   quand `dist < reach (1 - (t / lid)^shape) - 0,4 saillie(y)` : l'entaille est la plus profonde au pied
   (jusqu'à 10 blocs derrière la paroi) et se referme jusqu'à `t = lid` ; le **toit** reste plein et avance donc
   de plusieurs blocs au-dessus du pied. La **variété** : `shape` (de 0,7, voûte basse et longue qui se resserre
   vite, à 2,2, porche haut qui reste profond avant de se fermer) et `lid` (de 0,45 à 0,75) varient lentement le
   long de la paroi (bruits de 60 et 80 blocs), la profondeur par plages de 70 blocs avec une rugosité fine, et
   la saillie (de -1 à 1, par tranche de 3 blocs d'altitude, fonction de `y` seul) fait reculer ou saillir des
   couches : des marches sur la voûte. Le profil reste décroissant avec `dist` : l'argument « rien ne flotte »
   ne change pas. L'entaille n'existe que sur des parois de 12 blocs ou plus, jamais sous une zone réservée, un
   lit ou l'eau, jamais à moins de 6 blocs au-dessus de la mer, jamais plus profonde que la moitié de la paroi.
   Une **arche** apparaît sur une crête affinée : la profondeur (jusqu'à 2 fois plus grande, plafonnée à la
   moitié de la paroi) est partout forte sur la zone affinée, les deux entailles se rejoignent sous la crête dont
   la roche reste en toit (au moins le quart de la paroi) ; `reach` est plafonné en chaque point par l'épaisseur
   locale de la masse haute (le plus grand `dist` à moins de 8 blocs, moins 2,5 blocs, mais plus 6 blocs sous
   une crête affinée), si bien qu'un pilier ou une dalle isolés ne sont jamais évidés de part en part ; le toit
   d'une arche est tenu par les deux bouts de la zone. Sans escarpement (plaines, plateaux, côtes, mers,
   vallées, zones réservées) `y <= top` est la seule règle. Il n'y a plus de bruit 3D ni de poches sous la
   peau : les « tunnels près de la surface » du premier jet ont disparu, et la densité ne coûte plus une grille
   de bruit par verticale.
6. **Surface** (`surface.rs`) : règles ordonnées par colonne, sur la pente de la hauteur de surface (+-2 blocs)
   : lit (gravier, sable, argile), fond marin, falaise (pente >= 1,0 ou marche >= 3 : roche nue), éboulis
   (0,75 à 1,0 : gravier en altitude, terre grossière), neige (température ressentie : altitude, ombre des
   pentes exposées au pôle), berges (gravier, sable, argile), roche affleurante, sol du biome. Blocs existants
   uniquement, aucun identifiant changé.
7. **Strates** : couches de 3 blocs dont l'altitude ondule en continu (deux bruits de valeurs) et dont la
   nature (pierre, grès, roche volcanique) suit deux champs régionaux lents ; la saillie des couches entre dans
   l'entaille des parois (voir 5). Posées après les minerais (`ores.ron`, fonction de la v1 inchangée).
   Un **éventail** (±0,25 bloc par bloc autour de y = 96, bruit de 83 blocs) écarte ou resserre les couches
   avec la profondeur : sans lui, toutes les couches d'une colonne sautaient d'un bloc à la même abscisse et
   les coupes montraient de fausses failles verticales. Les sites de structures refusent toute colonne à
   surplombs et tout point à moins de 8 blocs d'une ligne de rupture dans leur emprise (grille de 2 blocs) ; les
   arbres refusent toute colonne à surplomb.
8. **Sous-sol** (`underground.rs`) : formes de la v1 (spaghetti et fromage, mêmes tampons par worker `CaveGrid`),
   règles nouvelles : plafond 4 blocs sous le relief (9 sous l'eau, plus de 10 sous une zone réservée), entrées
   contrôlées rares sur la roche nue, nappes liées au niveau des cours et lacs voisins (2 blocs sous leur
   niveau, +0,1 par bloc d'éloignement, effacées à 140 blocs), lave sous `LAVA_LEVEL`, eau jamais sous
   `LAVA_LEVEL + 4` : aucun contact eau/lave. À moins de 16 blocs derrière une paroi (de 4 blocs ou plus) et de
   3 blocs devant, aucune caverne ne monte au-dessus du pied de la paroi : elle trancherait la face ou le toit
   d'une entaille et en détacherait des blocs (vu au premier essai : quelques blocs flottants).
9. **Biomes provisoires** (lot 5 les remplacera) : le climat de la v1 (température, humidité, variété) lu
   tous les 16 blocs, humidité relevée près de l'eau ; la **famille** vient de la géographie (lit -> rivière,
   sous la mer -> océan ou océan profond, rivage bas -> plage, massif -> montagnes ou pics, sinon terre) ; dans
   la famille, le biome est choisi par son climat comme en v1. Les `relief` des biomes ne servent plus. Les
   biomes, minerais et structures des mods sont lus comme en v1 (même registre).
10. **Arbres et structures** : le sol est celui du bloc de surface calculé (`ground_block`), la base du tronc est
    `surface + 1` ; un arbre reste hors de l'eau et à une distance du lit égale au rayon de son feuillage, hors
    de l'emprise de toute structure ; une structure n'est posée que si une grille de 6 blocs sur son emprise
    n'approche aucune zone réservée. Les plantes aquatiques utilisent le niveau d'eau réel.

### Tests (`tests/generation_v2.rs`, résultats de cette exécution)

Tous passent (18 tests, 1 ignoré qui imprime la table) : v1 inchangée (`generation_reference` passe ; sélectionner
V1 explicitement donne les mêmes blocs ; v1 et v2 ne partagent jamais de cache), déterminisme v2 (ordre inverse,
parallèle sur 4 threads, caches vide/plein/vidé/évincé, bords et coins de quatre colonnes avec voisins en cache
ou non, coordonnées négatives), rivières (4 graines : eau non remontante, cuvette sans sortie impossible, lit
ouvert, berges fermées), lacs (niveau constant, rives fermées sauf déversoir ou entrée de cours), arbres (sol
réel, jamais dans l'eau, jamais flottants avec leurs branches) et structures (jamais près d'un lit ou d'un lac),
surface/pente (aucune herbe, neige ou sable sur une pente >= 1,0), surplombs (rares, jamais en zone réservée, ni
dans un lit : une entaille n'existe qu'au pied d'une paroi de 12 blocs ou plus, côté haut, dans un massif ou un
contrefort), sous-sol (dix blocs de roche sous un lit, jamais d'eau contre la lave), fluides (la simulation
s'endort sur l'eau d'un cours et d'un lac), couture entre chunks, empreintes figées (7 graines, **régénérées au
lot des surplombs puis à celui des escarpements structurels** : la v2 n'est portée par aucun monde réel ;
deux générations successives donnent la même table).

### Tests du relief vertical (`tests/relief_v2.rs`)

Tous passent (9 tests, 2 ignorés qui impriment le tableau chiffré et listent les surplombs les plus avancés).
Les mesures lisent les **blocs** d'un générateur `relief_only()` (ni cavernes, ni arbres, ni structures : les
cavernes ne comptent pas comme surplombs) sur douze zones de massif de 160 x 160 blocs (trois par graine) et sur
un balayage de 81 zones de 96 x 96 par graine. Définitions : *colonne à surplomb* = au moins un bloc d'air sous un
bloc plein (hors bois et feuillage) à moins de 30 blocs sous le sol, ouvert sur le côté ; *avancée* = le plus long
alignement d'air abrité sous un toit, mesuré vers l'intérieur depuis l'ouverture ; *colonne de paroi* = sol à 8
blocs ou plus au-dessus de celui d'une colonne à un ou deux blocs (les marches des lits et des lacs, à moins de
3 blocs d'une zone réservée ou d'eau, sont de l'hydrologie et ne comptent pas) ; *arche* = passage d'air sous un
toit, ouvert aux deux bouts ; *famille autorisée* = massif, contrefort ou côte haute (macro à plus de 25 blocs
au-dessus de la mer), hors zone réservée, lit et eau. Vérifications : parois et surplombs nombreux mais
localisés (moins d'une colonne sur dix pour les parois, sur cent pour les surplombs), **100 % dans les familles
autorisées et zéro** dans les plaines, plateaux, côtes basses, mers, vallées, lits et zones réservées (balayage
compris) ; **rien ne flotte** (chaque composante solide du relief touche le fond du monde ou le bord de la zone ;
avec cavernes, minerais, arbres et structures, aucun bloc flottant à hauteur d'une entaille) ; les faces de paroi
sont en roche (97 % au moins, les filons de terre ou de gravier affleurent parfois) ; aucun arbre sur une colonne
à surplomb ; déterminisme autour d'un surplomb avancé (ordre inverse, caches vidés chunk par chunk, quatre
threads) ; cohérence des pentes et de la distance à la ligne de rupture aux joints de colonnes de chunks (entre
deux colonnes voisines d'une paroi, à moins de 3 blocs de sa ligne, la distance varie d'au plus 2,5 blocs) ;
**forme des parois** (`wall_shapes_are_long_straight_and_do_not_loop`, voir plus bas).

Relief vertical sur les douze zones de massif, relief seul (lot 4 initial -> premier jet des escarpements,
isolignes d'un bruit -> escarpements posés sur la structure macro) :

| Graine | Parois | Colonnes à surplomb | Blocs de surplomb | Avancée moyenne (max) | Colonnes à avancée >= 3 | Arches |
| --- | --- | --- | --- | --- | --- | --- |
| 7 | 16 -> 610 -> 486 | 68 -> 237 -> 205 | 69 -> 244 -> 205 | 1,5 -> 9,0 -> 16,5 (40) | 9 -> 215 -> 192 | 2 -> 1 -> 0 |
| 42 | 0 -> 630 -> 720 | 6 -> 184 -> 286 | 6 -> 184 -> 286 | 2,0 -> 7,4 -> 5,6 (40) | 0 -> 173 -> 243 | 0 -> 0 -> 0 |
| 1337 | 0 -> 570 -> 366 | 12 -> 210 -> 163 | 12 -> 221 -> 166 | 1,7 -> 8,5 -> 7,2 (13) | 0 -> 204 -> 149 | 2 -> 3 -> 0 |
| 20260 | 1 -> 994 -> 645 | 3 -> 394 -> 305 | 3 -> 409 -> 305 | 2,0 -> 9,3 -> 16,1 (40) | 0 -> 368 -> 302 | 0 -> 6 -> 15 |
| **Total** | **17 -> 2 804 -> 2 217** | **89 -> 1 025 -> 959** | **90 -> 1 058 -> 962** | **1,6 -> 8,7 -> 11,5** | **9 -> 960 -> 886** | **4 -> 10 -> 15** |

Dont parois de 16 blocs ou plus : 1 653 -> 1 375 ; de 24 blocs ou plus : 741 -> 770 ; à moins de 200 blocs du
rivage : 602 -> 550. Balayage des 4 graines (324 zones de 96 x 96) : parois 2 195 -> 2 279, colonnes à surplomb
652 -> 975, arches 5 -> 25, toujours 100 % dans les familles autorisées et zéro ailleurs. Les parois sont un
cinquième moins nombreuses dans les zones de massif (elles sont plus longues, plus hautes, et ne bouclent plus),
les surplombs du même ordre, l'avancée plus grande et les arches plus nombreuses (mais concentrées : la graine
20260 en porte 15 sur les douze zones, trois graines n'en ont aucune dans leurs zones ; le balayage en trouve
dans trois graines). L'avancée mesurée est celle de l'air abrité : une alcôve profonde de 10 blocs sous une voûte
de 6 à 20 blocs compte jusqu'à 40, et le tunnel d'une arche tient la limite de 40 de la mesure.

**Forme des parois** (`tests/support/walls.rs`, carrés de 400 blocs autour des douze zones de massif, hauteurs de
sol lues dans les blocs). *Composante* : colonnes de paroi (dénivelé >= 8 blocs) dilatées de 2 blocs et connexes,
d'au moins 60 colonnes. *Anneau* : région de sol d'au moins 150 colonnes entièrement cernée de parois (sans
toucher le bord de la zone). *Doublage* : deux parois séparées de 3 à 23 blocs sur une même ligne ou colonne de
blocs (les paires dont l'intervalle est entaillé sont comptées à part : ce sont les deux faces d'une crête fine,
c'est-à-dire des arches voulues). *Sinuosité* : périmètre des composantes sur le double de leur étendue (1 pour
une paroi droite, davantage pour une boucle ou un ver). Les mêmes mesures sont prises sur les **bandes de
falaise** (pente de 1,0 ou plus : la roche nue des cartes de matériau, ce que l'oeil voit).

| Mesure (douze carrés de 400) | Parois, avant | Parois, après | Falaises, avant | Falaises, après |
| --- | --- | --- | --- | --- |
| colonnes | 16 545 | 13 112 | 188 936 | 54 526 |
| composantes | 87 | 59 | 330 | 150 |
| anneaux | 0 | 0 | 54 | 2 |
| doublages pour 1 000 colonnes | 26 | 3 (plus 643 paires de crêtes à arche) | 129 | 53 (plus 1 288) |
| sinuosité | 1,29 | 1,10 | 2,50 | 1,39 |

Par graine (parois, avant -> après) : sinuosité 7 : 1,29 -> 1,19 ; 42 : 1,32 -> 1,03 ; 1337 : 1,35 -> 1,08 ;
20260 : 1,23 -> 1,13. Bandes de falaise : anneaux 8 -> 1, 5 -> 0, 20 -> 0, 21 -> 1 ; sinuosité 2,23 -> 1,39,
2,05 -> 1,26, 3,11 -> 1,40, 2,83 -> 1,53. Bornes testées (`wall_shapes_are_long_straight_and_do_not_loop`) :
parois : au plus 2 anneaux, 20 doublages pour 1 000 colonnes, sinuosité 1,3, moins d'une composante pour 100
colonnes de paroi ; falaises : au plus 12 anneaux, 100 doublages pour 1 000, sinuosité 1,9. (Si l'on compte les
paires de crêtes à arche comme des doublages, les parois passent de 26 à 52 pour 1 000 : ce sont les arches
voulues, pas des murs minces isolés ; les bandes de falaise de 129 à 76.)

### Mesures (Apple M3 Pro, release, `cargo bench -p engine_voxel --bench generation`)

| Cas | v1 | v2 |
| --- | --- | --- |
| chunk de surface, à chaud | 0,48 ms | 0,53 ms |
| verticale de 6 chunks à froid | 1,63 ms | 5,1 ms (x 3,1) |
| zone 8 x 8 à froid (384 chunks) | 86 ms | 173 ms (x 2,0) |

Le « froid » v2 comprend l'hydrologie macro et le contexte de colonne (`clear_caches` vide tout).

Après les escarpements (même machine, même jour, avant -> après) : surface à chaud 0,56 -> 0,56 ms ; verticale
à froid 3,13 -> 3,35 ms ; zone 8 x 8 à froid 178 -> 176 ms. Deux cas ajoutés dans un **massif** (graine 42, vers
x = -256, z = -512) : verticale à froid 7,7 -> 6,5 ms et zone 8 x 8 à froid 273 -> 255 ms. Le coût ne monte pas :
la grille de bruit 3D par verticale, qui servait aux poches, disparaît, et les escarpements ne coûtent que trois
évaluations de bruit par point de la grille de 4 blocs, là où une chaîne forte les autorise.

Après les escarpements posés sur la structure macro (mêmes cas, même machine, avant -> après) : surface à chaud
0,56 -> 0,55 ms ; verticale à froid 3,11 -> 3,08 ms ; zone 8 x 8 à froid 173,7 -> 176,8 ms (+ 2 %) ; massif :
verticale à froid 6,5 -> 7,9 ms (dispersion large sur ce cas : 6,1 à 7,3 ms puis 7,3 à 8,9 ms) et zone 8 x 8 à froid
253 -> 259 ms (+ 2 %). Pas de surcoût important : le calcul d'une paroi (deux distances déjà connues de la
géographie, quelques bruits lents, neuf cases de failles hachées) se fait une fois par point de la grille de 4
blocs.

### Lisibilité des cartes et coupes (jugement)

Cartes `hauteur-blocs`, `materiau`, `eau-blocs`, `surplombs`, `parois` (nouvelle : dénivelé du sol sur un pixel
en rouge-jaune de 8 à 40 blocs, colonnes à vide ouvert en bleu ; à lire en `--step 1` ou `2`) et coupes colorées
(`--section`, `--relief` pour le relief seul), graines 42, 7, 1337 et 20260. Les rivières, confluences, deltas et
lacs lus dans les blocs concordent avec le relief ; les falaises et éboulis suivent la pente ; les strates se
suivent d'un chunk à l'autre et changent par régions.

*Ce qui est net.* Le défaut du premier jet des escarpements (parois en boucles serrées, vers et murs doublés
sur les grands massifs, isolignes d'un bruit de 140 blocs) est corrigé. Sur les cartes `parois` et `materiau`
(graines 1337, 42, 7, 20260, vues de 320 et 1 000 blocs), les lignes de rupture sont maintenant **peu nombreuses,
longues (une à quelques centaines de blocs), droites ou courbes comme les flancs de la chaîne** (graine 42 : un
éventail de parois parallèles au flanc sud du massif ; graine 20260 : des arcs parallèles aux axes de la chaîne),
et elles **s'éteignent par le bout** (la hauteur décroît, on voit leur ombre portée s'effacer sur la carte de
hauteur). Les cartes de matériau ne montrent plus le réseau de bandes de roche nue de l'ancien massif : 54
anneaux de falaise contre 2 sur les douze carrés de 400 blocs, 189 000 colonnes de falaise contre 55 000. Les
coupes à travers les parois montrent de vrais reliefs verticaux : ressauts de 10 à 44 blocs, des toits qui avancent
de plusieurs blocs au-dessus d'une alcôve avec les couches de grès lisibles sur la voûte (graine 7 vers
x = -1 663, z = -1 609 ; graine 42 vers x = -256, z = -551), des porches hauts et des voûtes basses (la forme varie
le long d'une paroi), et des **arches** longues sous des crêtes affinées (graines 20260 vers x = 13, z = -14 ; 42
vers x = -42, z = -37 ; 1337 vers x = 1 619, z = 186). Les faces sont en roche, le sommet en herbe ou en neige.
Rien de tout cela n'apparaît dans les plaines, côtes basses, vallées ni lits.

*Ce qui l'est moins.* (a) Les parois de flanc sont **régulières** : parallèles à l'axe de la chaîne et espacées de
84 blocs (le décalage de ±28 blocs ne suffit pas à casser l'effet d'éventail sur les graines 42 et 20260) ; c'est
la structure demandée, mais on y lit la règle. (b) Les massifs sont plus **lisses** qu'avec les arêtes : sans les
isolignes de bruit, le détail de massif ne fait plus que des bosses douces, le relief entre deux parois est
monotone. (c) Les arches sont des **tunnels longs** (40 blocs et plus, la longueur de la zone de crête affinée) à
toit épais plutôt que des ponts minces, et elles sont concentrées (15 sur la graine 20260 sur les douze zones, 0 sur
trois graines ; le balayage en trouve 25 sur trois graines). Une crête affinée au-delà de l'entaille laisse un
toit de 3 à 4 blocs sur ses bords (coupe de la graine 42 ci-dessus). (d) Les paires de parois d'une crête affinée
(14 blocs de large) comptent encore comme doublages quand on ne les écarte pas : 643 paires de lignes de blocs sur
les douze carrés. (e) De grandes taches plates de roche nue subsistent sur les plus hautes chaînes (graines 7 et 20260) : elles
ne viennent pas des parois, et leur cause (altitude, écrêtage du relief) n'a pas été analysée. (f) Les falaises côtières n'existent
que là où un massif atteint le rivage (graine 1337) : une côte basse n'en a pas, par construction. (g) Les strates
restent des couches de 3 blocs assez régulières. (h) La lecture « à hauteur de joueur » repose sur des coupes et des
cartes : aucune capture du jeu (consigne : ne pas lancer le jeu graphique).

### Limites et ce qui n'est pas fait

- Les escarpements sont des ressauts sur des lignes de rupture posées sur la structure macro (flancs de chaîne,
  crêtes secondaires, failles) : pas de falaise en arc surplombant sur toute sa hauteur (l'entaille occupe de 45 à
  75 % de la hauteur d'une paroi de 12 blocs ou plus), pas de plafond de caverne naturelle ni de pilier détaché.
  Les parois de moins de 12 blocs n'ont pas d'entaille. Seules les zones où `chain >= 24` et le sol à plus de 8
  blocs de la mer en portent : une paroi n'existe jamais sur un plateau ni une plaine.
- Les failles sont d'orientation quelconque (la géographie n'expose pas la direction de l'axe : elles ne sont pas
  « transverses » au sens strict) ; leur distance à une autre paroi est garantie par effacement mutuel, pas par
  construction.
- Une arche ne peut se former que sur une crête secondaire affinée (14 blocs de large, paroi d'au moins 20 blocs) ;
  elle est un tunnel long. Pas de pont mince, pas d'arche sur un flanc de chaîne ou une faille.
- Les parois d'escarpement s'arrêtent à 40 blocs d'un cours ou d'un lac (le ressaut s'efface dans les vallées) :
  les gorges taillées par une rivière ne sont pas des parois d'escarpement ; les marches d'hydrologie des lits
  restent celles du lot 3 et ne sont pas comptées comme parois.
- Les cavernes ne montent pas au-dessus du pied d'une paroi à moins de 16 blocs derrière elle : une paroi n'a donc
  jamais d'entrée de caverne en hauteur.
- Biomes, climat et végétation élaborés : lot 5. Rendu et streaming : lots 6 et 7. Terrain lointain v2 : lot 8
  (le lointain lit les colonnes v2 sans volume ; les surplombs n'y figurent pas).
- Les structures lisent toujours `column(x, z).height` et leur propre règle de nivellement : un village sur une
  pente raide reste possible ; seul le dégagement des cours d'eau est garanti.
- Le rayon de dégagement des structures utilise le rayon déclaré (grand pour une mine) : des sites sont
  écartés alors que leur emprise réelle est plus petite.
- La stabilité des fluides est vérifiée par une simulation headless sur deux zones (réveil de toute l'eau en
  contact avec l'air) ; une recette en jeu reste à faire (lot 9). Une marche d'eau d'un bloc existe à l'entrée
  d'un cours dans un lac et sur les paliers : elle s'écoule vers l'aval, jamais vers l'amont.
- Les nappes sont liées au niveau des cours et lacs voisins, pas à un modèle d'eau souterraine.
- Aucun monde v2 n'est jouable par défaut : créer un monde de test avec `VOXEL_GENERATION=2`.

## Statut du lot 5 (climat, biomes et végétation liés au paysage)

Le climat et les biomes de la génération v2 sont déduits de la géographie. Le calcul reste **acyclique** :
relief et hydrologie -> climat -> biomes -> végétation ; la pluie n'agrandit aucun bassin et ne remplit aucun
lac. `FOR_NEW_WORLDS` reste v1 ; v2 se crée avec `VOXEL_GENERATION=2`.

### Conception

- **Température** (`generator/climate.rs`) : un champ régional lent (8 000 blocs), tempéré près des côtes
  (-30 % d'amplitude au rivage), refroidi par l'altitude du relief macro (0,0065 par bloc au-dessus de
  `SEA + 25`). La température au niveau de la mer est gardée à part : elle règle la ligne des arbres.
- **Humidité et ombre pluviométrique** : un vent dominant par graine, qui dérive lentement (12 000 blocs). Trois
  chemins d'air de front (150 blocs d'écart), 12 pas de 107 blocs (1 280 blocs amont, 36 lectures de la
  géographie macro : coût borné). On en tire l'apport maritime (+0,55 au plus), la continentalité (-0,55 loin
  des côtes), l'**ombre pluviométrique** (-0,9 au plus : un relief amont plus haut d'au moins 20 blocs retient la
  pluie, pleinement à 90) et le surcroît des versants au vent (+0,45). L'indicateur de mer est lissé (SEA ± 8) :
  un chemin qui effleure une côte ne fait pas d'à-coups. Un premier essai à 8 pas et un seul chemin traçait des
  sillages étroits derrière chaque île ; un autre, avec un indicateur binaire, faisait osciller le biome
  d'une centaine de blocs à l'autre : tous deux corrigés.
- **Lecture** : le climat n'est évalué qu'aux quatre coins de la colonne de chunks (grille absolue de
  32 blocs) puis interpolé : la colonne voisine lit les mêmes valeurs à sa frontière.
- **Effets locaux** (`v2_context.rs`) : les **marais** sont une terre basse, plate, humide, en taches, loin du
  rivage ; les biomes de zone humide (climat d'humidité de 0,7 ou plus) ne sont candidats que là où cette zone
  dépasse 0,4, le climat seul n'en fait pas. La **plage** passe jusqu'à 70 blocs du rivage sur sol bas et doux ;
  la côte devient rocheuse là où la pente, lue sur 8 blocs, passe de 0,22 à 0,45. Le **biome de rivière** suit le
  réseau réel (lit d'un cours ou d'un lac). L'eau proche (+0,45) ne compte que pour la végétation.
- **Variété** : amplitude réduite (1,1) et longueur d'onde de 1 100 blocs : les variantes (prairie fleurie,
  forêt sombre...) sont des variations locales, pas la trame des régions.
- **Végétation** (`canopy_factor`) : un coefficient par colonne, de 0 à 2,6, qui multiplie la densité d'arbres
  du biome. Il dépend de l'humidité relative au climat du biome (forêt plus dense en zone plus humide, éclaircie
  en zone plus sèche : une lisière naturelle), de la pente (aucun arbre au-delà de 1,1), de la ligne des arbres
  (`SEA + 100 + 28 × température au niveau de la mer`) et des berges (+80 % entre 5 et 9 blocs d'une zone
  réservée). Essences, sols, bosquets et règles d'apparition restent ceux de la v1 ; le tirage v1 est inchangé
  (la borne de rejet rapide ne s'élargit qu'en v2).
- **Paramètres `Relief` des biomes** : non utilisés en v2. Le relief se décide avant le biome ; un biome qui le modulerait
  fermerait un cycle (le biome dépend de l'altitude) et redessinerait des vallées protégées. Adaptation
  possible plus tard : un biome compatible module l'amplitude du détail fin dans une famille donnée, jamais en
  zone réservée.
- **Outils** : `cargo xtask map --generator v2 --maps ...` gagne `ombre-pluie`, `maritime`, `humidite-regionale`
  (géographie seule) et `biomes-v2`, `temperature-v2`, `humidite-v2`, `canopee` (colonnes du générateur ; la
  légende des biomes est écrite dans `biomes-v2.txt`).

### Tests (`tests/climate_v2.rs`, 15 + 2 ignorés ; `generator/tests.rs`, 4)

Pureté et continuité du climat (ordre, graine, pas de 16 blocs) ; altitude qui refroidit ; vent par graine ;
**ombre pluviométrique** (211 points d'ombre à -0,64 d'humidité moyenne, contre -0,11 pour 8 446 points
dégagés) ; biomes d'accord avec le relief et l'eau (océan sous la mer, pics et montagnes en altitude, rivière
exactement sur les lits, plage en bas, marais bas, plat, humide, jamais gelé) ; biome de terre proche de son
climat ; tout rivage bas et doux est une plage ; au plus 6 changements de biome de 120 blocs en mer à
250 blocs dans les terres (les cours exclus) ; diversité (au moins 8 biomes de terre, aucun au-delà de 45 %) et
régions étendues (plus de 250 blocs en moyenne) ; terres constructibles (au moins 40 % de colonnes plates) ;
coefficient de végétation (bornes, falaises, ligne des arbres, berges) ; **chaque tronc généré pousse là où la
végétation est permise** ; du bois à moins de 200 blocs du point d'apparition ; déterminisme (ordre inverse,
caches vidés, générateur neuf) ; continuité de l'humidité et de la température aux frontières de colonnes de
chunks. `generation_reference` (v1) est inchangé ; les empreintes v2 ont été régénérées (v2 n'est portée par
aucun monde) et deux générations successives donnent la même table.

### Mesures (M3 Pro, release)

- Marais : 0 à 1,4 % des terres échantillonnées selon la graine (0 sur la graine 7, une région froide).
- Terres : 11 à 17 biomes par graine sur 4 graines.
- Coût (`cargo bench -p engine_voxel --bench generation -- v2`) : verticale à froid 3,08 -> 3,44 ms (+11 %),
  zone 8 × 8 à froid 177 -> 180 ms (+2 %), massif zone 8 × 8 259 -> 296 ms (+14 %). Une première version
  qui lisait le climat tous les 16 blocs coûtait +18 %, +12 % et +24 % : la grille de 32 blocs suffit, les
  champs variant sur des centaines de blocs.

### Jugement sur les cartes (graines 42, 7, 1337, 20260 ; images dans le dossier de travail de la séance)

Les régions sont étendues et cohérentes (une plaine, une forêt ou une taïga couvre des centaines à des milliers
de blocs) ; la plage forme un ruban continu le long du rivage ; un marais apparaît dans la plaine basse d'un
estuaire ; les rivières traversent plusieurs biomes sans en changer ; le couvert végétal s'éclaircit vers les
pentes et les sommets et s'épaissit le long des cours.

Défauts assumés : (a) les frontières de biome restent des courbes lisses mais parfois **rectilignes** sur des
centaines de blocs (iso-lignes du climat interpolé) ; (b) les marais sont rares (au plus 1,4 % des terres) et
petits ; (c) la graine 7 est presque entièrement froide : le climat régional d'une graine peut ne pas couvrir
toute la palette ; (d) aucune rétroaction de la pluie sur le relief ou les cours (voulu) ; (e) la direction du
vent est unique par région : pas de moussons ni de saisons ; (f) pas de capture à hauteur de joueur : le jeu
graphique n'a pas été lancé.

## Statut du lot 6 (priorité aux chunks proches et transferts régulés)

Le lot 6 touche le streaming et le jeu, pas le générateur : v1 et v2 sont inchangés (empreintes de référence
intactes). Les nouvelles politiques sont les valeurs par défaut ; l'ancien ordre reste accessible par
`StreamingConfig` pour comparer.

### Ce qui change

- **Ordre de chargement** (`streaming.rs`) : des colonnes de chunks complétées de la plus proche à la plus
  lointaine, avec un biais pour ce que la caméra regarde (`update_view`, `view_bias` 0,6 : la distance d'une
  colonne derrière compte jusqu'à 1,6 fois plus ; rien au-dessous de 3 chunks, donc le sol sous les pieds ne dépend
  jamais du regard). Un demi-tour de plus de 25 degrés retrie la file. L'ancien tri par distance 3D mélangeait les
  colonnes : le haut d'une colonne proche passait après le bas d'une colonne plus loin, alors que la lumière et le
  maillage attendent la colonne entière.
- **File des maillages** : le plus récent de chaque chunk (plus d'envoi périmé), du plus proche au plus lointain ;
  les maillages d'un bloc modifié passent en tête, hors budget ; une libération en attente est annulée si le chunk
  revient avec un maillage (sinon elle effacerait le nouveau côté GPU).
- **Remaillage coalescé** : un chunk modifié pendant qu'on le maille n'est remaillé qu'une fois de plus à la fin,
  au lieu de lancer une tâche par modification et de jeter chaque résultat. Un chunk qui change à chaque tick
  (fluides, machines) ne s'actualisait jamais tant que ça durait.
- **Budget de transfert** (`transfer.rs`, `games/voxel/src/stream_upload.rs`) : un seul budget par image pour les
  chunks, les libérations et le terrain lointain, en octets (3 Mio) **et** en temps mesuré (3 ms), au lieu de 16
  maillages. Le premier envoi de l'image passe toujours, quelle que soit sa taille : une grosse opération n'est
  jamais bloquée ; les dépassements sont comptés (`envois : ...` dans les logs). Réglages : `VOXEL_UPLOAD_MB`,
  `VOXEL_UPLOAD_MS`.
- **Terrain lointain** (`far.rs`) : une allocation de CPU (`far_allowance`) calculée d'après le travail des chunks
  proches (tous les threads au calme, un seul pendant un chargement soutenu au-delà de 300, aucun au-delà de
  1 500) ; des demandes au compte-gouttes (deux par thread autorisé) recalculées à chaque image, donc une tuile
  devenue inutile avant son départ est abandonnée sans avoir coûté ; les changements d'une région forment un lot
  (retraits puis ajouts) livré d'un bloc, du plus proche au plus lointain, sous le budget commun, avec un lot forcé
  toutes les 6 images pour qu'un long vol ne l'affame pas.

### Mesures (`cargo run --release -p engine_voxel --example streaming_scenarios`, M3 Pro)

Images simulées à 60 par seconde en temps réel (les threads de fond tournent pour de vrai), monde v1, rayon de vue
de 10 chunks, graine 42. **L'envoi au GPU est simulé** (25 µs + 0,7 ns par octet, 11 µs par libération) : c'est une
hypothèse qui sert à comparer des politiques sur la même charge, pas une mesure du pilote. Les délais sont ceux où
toutes les colonnes d'un rayon sont visibles (six chunks générés, chaque chunk non vide maillé et envoyé), « devant /
tout » (devant : le demi-plan regardé).

| Scénario (ms) | rayon 2 | rayon 4 | rayon 6 | rayon 10 |
| --- | --- | --- | --- | --- |
| Démarrage, ancienne | 726 | 1 349 | 2 435 | 5 168 |
| Démarrage, colonnes seules | 613 | 1 290 / 1 315 | 2 423 | 5 210 |
| Démarrage, nouvelle | 677 / 714 | 1 304 / 1 472 | 2 280 / 2 804 | 4 776 / 5 185 |
| Téléportation, ancienne | 697 | 1 292 | 2 359 | 5 171 |
| Téléportation, nouvelle | 647 / 692 | 1 272 / 1 401 | 2 186 / 2 688 | 4 656 / 5 127 |

Vol saturé à 300 blocs/s pendant 25 s (le chargement ne suit plus ; colonnes manquantes dans un rayon de 8) :

| Politique | devant : moyenne, p95 | derrière : moyenne, p95, max | entrée -> visible p50 / p95 | pic d'une image |
| --- | --- | --- | --- | --- |
| ancienne | 79,5 ; 84 | 22,4 ; 28 ; 28 | 689 / 1 194 ms | 4,20 Mio, 3,6 ms |
| colonnes seules | 80,6 ; 85 | 22,5 ; 28 ; 28 | 720 / 1 318 ms | 4,33 Mio, 3,6 ms |
| nouvelle | 72,2 ; 76 | 21,5 ; 28 ; 28 | 582 / 1 105 ms | 2,95 Mio, 3,0 ms (8 images au-dessus du budget) |

Terrain lointain activé (48 chunks, démarrage) : avec la politique ancienne, 340 tuiles se construisent en même temps
que les chunks proches (rayon 10 visible à 5 258 ms) ; avec la nouvelle, 4 tuiles seulement pendant le chargement
(4 801 ms devant, 5 235 ms en tout), puis le lointain se complète **483 ms après** les chunks proches. À 120 blocs/s le
chargement suit sans retard dans les trois politiques (aucune colonne manquante).

### Jugement honnête

- Le **débit** de génération, lumière et maillage borne les délais : l'ordre ne le change pas. Au démarrage et en
  téléportation, le gain est faible : la moitié regardée est visible 6 à 10 % plus tôt aux rayons 6 et 10, mais celle de
  derrière 15 % plus tard au rayon 6 (c'est le prix du biais). Le tri en colonnes seul ne change rien de net.
- En **vol saturé**, les gains sont réels : 9 % de colonnes manquantes en moins devant, entrée -> visible de 689 à
  582 ms en médiane (-16 %), de 1 194 à 1 105 ms au p95 (-7 %), et les pics de transfert baissent de 30 %. Rien n'est
  affamé : le pire cas derrière est identique (28 colonnes).
- Le terrain lointain ne gêne plus le chargement proche, au prix d'arriver environ une demi-seconde après lui. Le
  coût de la concurrence du lointain avec le pool était de l'ordre de 2 à 5 % sur les délais proches : dans le bruit
  des mesures, mais le démarrage n'a plus de tuiles construites en même temps que les chunks.
- Les **8 images au-dessus du budget** du vol saturé sont le dépassement d'une opération indivisible (le dernier
  envoi franchit les 3 ms) : voulu, compté, jamais bloquant.

### Limites

- Le coût d'envoi au GPU est simulé ; le budget par défaut (3 Mio, 3 ms) n'est pas calé sur un pilote réel. À
  régler avec `VOXEL_UPLOAD_MB` / `VOXEL_UPLOAD_MS` en jeu avec les journaux `envois : ...`.
- Les améliorations toujours actives (file des maillages, coalescence, lots de tuiles) ne sont pas isolées dans les
  mesures ; elles ont leurs tests (`streaming.rs`, `far.rs`, `transfer.rs`).
- Le biais de vue au démarrage suit la direction initiale de la caméra, arbitraire tant que le joueur n'a pas
  tourné. Le jeu n'a pas été lancé : aucune vérification à hauteur de joueur, ni du filtrage de colonnes à
  l'envoi sous VSync.
- Un fantôme possible, déjà présent avant ce lot : un chunk modifié qui devient entièrement vide garde son ancien
  maillage côté GPU (aucun événement n'est émis pour un chunk absent du monde).

## Statut du lot 7 (maillage et rendu groupé) — première partie : le maillage

**Périmètre de cette livraison.** Le lot 7 touche aussi le rendu (arènes GPU paginées, dessin groupé, comparaison
Metal, parcours de visibilité). Ces parties modifient `crates/engine_render/src/renderer.rs`, que le travail sur le ciel
transforme en parallèle (non commité), et elles exigent un GPU réel pour valider « image équivalente » et les
coûts Metal : **elles ne sont pas faites** et restent à livrer. Cette livraison couvre le côté CPU, qui ne dépend
pas du renderer.

### Ce qui change

- **Maillage par masques binaires** (`mesh/binary.rs`, `Mesher::Binary`, `mesh_binary`) : le même ensemble de quads que
  le greedy, calculé autrement. Le greedy interroge chacune des 6 × 32 768 faces possibles d'un chunk, puis relit les
  cellules d'alentour à travers les 27 chunks pour l'occlusion et la lumière de chaque coin. Le chemin binaire :
  1. copie le chunk et deux blocs de chaque côté à plat (36³), rangée par rangée, depuis les 27 chunks ;
  2. construit, par axe, des rangées de 32 bits : cubes **opaques** (voie rapide), cubes **cachant la face de leur
     voisine**, **autres cubes** (feuilles, eau, verre, glace : voie complète). Les faces visibles d'une rangée
     entière sont `opaques & !cache(voisine)`, une opération ;
  3. ne calcule l'apparence (occlusion, lumière douce, teinte) que des faces visibles, depuis la copie à plat, puis
     fusionne comme le greedy.
  Les fluides gardent `Neighbourhood::face_look` (hauteur de surface par coin) ; les plantes, rails, dalles, boîtes
  et animations restent à `plants()`. Les feuilles enfouies, le culling de feuilles et les flammes de four sont
  reproduits. Les tampons (copie, masques, table des types de blocs) vivent dans un tampon par thread, réutilisé
  d'un chunk au suivant ; la table est refaite si le registre change.
- **Équivalence testée** (`mesh/binary.rs`) : mêmes triangles (sommets octet par octet, ordre ignoré) que le greedy sur
  les chunks de 9 régions générées (graines 7, 42, 1337, éclairés comme le streamer ; eau et feuillage présents), sur une
  région v2, sur des mélanges aléatoires de pierre, verre, glace, eau, feuilles, herbe haute, planches, four et
  pierre lumineuse à travers les frontières de chunks, sur des couches pleines avec feuilles enfouies et trous, sans
  voisins et avec un registre refait, avec et sans culling des feuilles. Deux mutations (une règle d'occlusion,
  une règle de visibilité) font échouer ces tests.
- **Connexions de faces** (`visibility.rs`) : `FaceConnections::of_chunk`, appelé à chaque maillage, n'alloue plus rien
  (tampons par thread) et calcule les voisins sans tableau. Une copie de l'ancienne version sert de référence à
  un test sur 288 chunks aléatoires.
- **Par défaut** : le streamer maille avec `Mesher::Binary` ; les structures mobiles (`ships.rs`) aussi ; F1 fait
  tourner culling, greedy, binaire.

### Mesures (`cargo bench -p engine_voxel --bench meshing`, M3 Pro, un chunk)

| Scène | culling | greedy | binaire | gain sur le greedy |
| --- | --- | --- | --- | --- |
| terrain (surface) | 1,83 ms | 1,79 ms | **0,42 ms** | ×4,2 |
| forêt (feuillage dense) | 1,24 ms | 1,43 ms | **0,78 ms** | ×1,8 |
| mer (eau, translucide) | 2,06 ms | 2,00 ms | **0,65 ms** | ×3,1 |
| damier (pire cas, rien à fusionner) | 13,2 ms | 13,8 ms | **10,9 ms** | ×1,3 |

Où va le temps restant d'un chunk de surface (environ 0,8 ms pour la scène la plus chargée du test de coûts) : la copie
6 µs, les classes 53 µs, les rangées de bits 70 µs, les apparences des faces visibles 380 µs, la fusion et les quads
180 µs, les connexions de faces 60 µs (167 µs en forêt), les plantes 45 µs. Le coût suit désormais les faces **visibles**
(environ 150 ns d'apparence et 90 ns de quad par face), pas le volume du chunk.

**Mesure de bout en bout** (`streaming_scenarios --mailleur`, démarrage, rayon 10, trois répétitions) : 5 079 ms
avec le greedy, 4 984 ms avec le binaire (-2 %, dans le bruit). Le maillage n'est qu'une partie du travail d'un chunk
(la génération et la lumière dominent) : le gain est réel au niveau du maillage, faible sur le délai de chargement
global ; il profite surtout aux remaillages (modifications, fluides, machines) et libère du CPU.

### Limites et reste du lot 7

- **Arènes GPU, table d'origines et dessin groupé** : livrés ensuite, voir la seconde partie. Multi-draw, indices
  regroupés et parcours de visibilité y sont mesurés ou écartés avec leurs raisons.
- Le greedy reste le chemin de référence : `Mesher::Greedy` et `Mesher::Culling` sont conservés, et les tests
  d'équivalence les utilisent comme oracle.
- La voie rapide ne couvre que les cubes pleins sans découpe ni fluide ; le feuillage dense et l'eau passent par la
  voie complète (apparence calculée sur la copie à plat, mais règles de visibilité plus longues) : le gain y est plus
  faible (×1,8 en forêt).
- Le damier montre que la fusion et les quads restent le coût d'une géométrie très fragmentée.
- Non mesuré : l'effet sur la mémoire du tampon par thread (environ 0,3 Mio par thread de fond).

## Statut du lot 7 — seconde partie : arènes GPU et dessin groupé

`renderer.rs` est libre depuis le commit du ciel ; cette partie livre le stockage et le dessin des chunks. Mesuré sur la
machine du développeur (M3 Pro, release, fenêtre 1280 × 720, banc `VOXEL_BENCH`, graine 42).

### Ce qui change

- **Arènes paginées** (`engine_render/src/arena.rs`). Avant : trois tampons wgpu par chunk (sommets, indices,
  origine) et trois liaisons par dessin. Maintenant, la géométrie vit dans des **pages** de 2¹⁸ sommets (6 Mio) et 2¹⁹
  indices (2 Mio) ; un maillage occupe une plage de sommets et une d'indices **de la même page** (indices relatifs au
  premier sommet : on dessine avec `base_vertex`). Un maillage plus grand qu'une page a la sienne.
  `RangeAllocator` (meilleur ajustement, fusion des plages voisines) gère les plages ; testé sans GPU (5 tests, dont un
  tirage aléatoire de 5 000 opérations sans chevauchement, qui finit en une seule plage).
- **Table d'origines commune** (`OriginTable`) : origine, lacet, base de texture et échelle de tous les chunks (32 octets
  chacun) dans **un seul tampon** lu comme des instances ; le chunk n se dessine avec `first_instance = n`. Le shader n'a
  pas changé. La table double quand elle est pleine (copie de l'ombre côté processeur). `set_mesh_transform` (structures
  mobiles) ne réécrit que son entrée.
- **Dessin groupé** : en opaque, les chunks visibles sont triés par page puis par position : une page se lie une fois, chaque
  chunk n'est plus qu'un `draw_indexed`. En translucide, **le tri du plus loin au plus proche est conservé** (l'ordre
  prime sur le regroupement) ; la page n'est relue que lorsqu'elle change entre deux voisins consécutifs.
- **Sûreté GPU** : une plage ou une entrée libérée n'est réutilisée qu'après 3 images (`REUSE_DELAY`). Les pages vides
  sont rendues au GPU, sauf une de réserve : la mémoire suit l'exploration. wgpu garde de toute façon un tampon vivant
  tant qu'une commande en vol le lit.
- **Transferts** : une `write_buffer` par maillage pour les sommets et une pour les indices, qui passent par le
  mécanisme de staging interne de wgpu (tampons réemployés par wgpu). Je n'ai pas écrit de courroie de staging maison :
  elle ferait doublon avec celle de wgpu sans gain mesurable (voir les limites).
- **Statistiques** : `RenderStats` expose `chunk_draws`, `page_binds`, pages, octets réservés et occupés, entrées
  d'origines ; le journal de jeu les affiche (« arène : … »).

### Mesures (banc, 6 trajets, deux passes « après » contre une passe « avant »)

| Trajet | CPU avant | CPU après (2 passes) |
| --- | --- | --- |
| survol | 1,64 ms | 1,08 / 1,08 |
| immobile | 1,46 | 0,93 / 0,92 |
| forêt | 1,42 | 0,93 / 1,18 |
| montagnes | 1,35 | 0,85 / 0,91 |
| océan | 1,52 | 0,90 / 0,96 |
| sous-sol | 1,48 | 1,68 / 1,02 (bruité) |

Le temps CPU médian par image baisse d'environ **30 à 40 %** sur les trajets de surface (environ 650 à 900 chunks
dessinés, 160 à 220 liaisons de page au lieu de près de 3 par chunk). Le 99e centile CPU baisse aussi (4,2 → 3,1 ms au
survol, 2,1 → 1,4 ms à l'arrêt). Le temps GPU de la passe du décor reste autour de 8 ms avant comme après : l'affichage plafonne à 120 Hz
et cette mesure sature à la période de l'image, elle **ne distingue pas** les deux versions. Le FPS moyen est plafonné
(120) dans les deux cas.

**Fuites** (journal du banc, huit relevés répartis sur les six trajets, avec les retours loin entre trajets) : le nombre d'entrées de la
table d'origines égale le nombre de maillages chargés (à 1 ou 2 près, entrées en attente) ; les pages montent de 24 à 86
puis redescendent à 40 quand le monde se vide ; 88 à 93 % des octets réservés sont occupés (fragmentation faible).
La mémoire de géométrie suit donc le nombre de chunks ; le total (jusqu'à 690 Mio pour 3 700 maillages, soit près de
185 Kio par maillage) est le même volume qu'avant, seul le découpage change.

**Image équivalente.** Les 43 captures du tour `VOXEL_SCREENSHOTS` ne sont pas reproductibles bit à bit, même sans mes changements
(deux passes « avant » diffèrent de 40 000 à 120 000 pixels : nuages, vent, créatures, eau animée). Je n'ai donc pas pu
montrer une égalité exacte. Ce que j'ai vérifié : la capture sous terre (sans ciel) est **identique à l'octet près** ; sur les scènes de surface, les écarts forts
dans la moitié basse de l'image (terrain) sont de l'ordre du bruit entre deux passes « avant » (quelques centaines à
mille pixels de plus de 24 niveaux) ; l'inspection visuelle d'une capture de rivière (terrain, eau translucide, feuillage, plantes)
est correcte. Une comparaison exacte demande de figer les nuages et les animations : non fait.

### Décisions et limites

- **Multi-draw et indices regroupés non faits.** La mesure ne les justifie pas : le CPU du rendu est déjà
  d'environ 1 ms par image avec 800 à 900 dessins directs, et le GPU ne peut pas départager les variantes sur cette machine
  (mesure saturée à 120 Hz). Le multi-draw indirect est de plus émulé par wgpu sur Metal (une boucle de dessins). À
  revoir si une scène dépasse nettement les quelques milliers de chunks dessinés ou sur une cible mesurée plus lente
  (les GPU Intel n'ont pas été testés ; `first_instance` direct est garanti par wgpu 30, donc pas de chemin de repli
  nécessaire pour ce point).
- **Pas de compaction** des pages : la fragmentation reste basse en pratique (88 à 93 % d'occupation) et les pages vides
  sont rendues. Une page peu occupée mais jamais vide reste réservée.
- **Parcours de visibilité** : pas mesuré séparément ; le CPU total de l'image est d'environ 0,9 ms, ce qui borne son coût.
  Pas de structure dense introduite. Frustum et culling des grottes inchangés.
- Une écriture par maillage (`write_buffer`) : le coût GPU/CPU des envois n'a été mesuré que par le simulateur du lot 6.
- Non couverts par un test automatique (pas de GPU en test) : le dessin lui-même, les structures mobiles (leur origine passe
  par `set_mesh_transform`, vérifié seulement par compilation et par le tour de captures), l'animation d'eau et le feuillage.
