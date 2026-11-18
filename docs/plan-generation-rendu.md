# Plan : géographie cohérente et affichage rapide des chunks

Établi le 17 novembre 2026. Statut au 18 novembre 2026 : **lots 0 et 1 exécutés** (livraison A,
voir « Statut de la livraison A » en bas) ; **lots 2 et 3 exécutés comme prototype macro** (livraison B,
voir « Statut de la livraison B ») ; lots 4 à 9 non commencés.

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

### Lot 4 — relief 3D, surface et sous-sol cohérents

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
`generator/hydrology.rs` (lot 3), `generator/context.rs` (`MacroWorld` : requête de point avec vallées et
lacs), `worldmap_macro.rs` et l'exemple `worldmap.rs` (cartes), `tests/macro_world.rs` (12 tests),
`benches/hydrology.rs`.

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

Deux échelles : **cellules fines de 64 blocs** (nœud jitteré, altitude macro) et **bassins de 8 × 8
cellules (512 blocs)**.

1. *Niveau supérieur.* Le nœud d'un bassin est sa cellule fine la plus basse (parmi les 6 × 6 centrales).
   Un bassin s'écoule vers celui de ses 4 voisins dont `(altitude du nœud, hachage)` est strictement plus
   bas, par la **passe la plus basse** de la frontière (paire de cellules de bord). Ordre total strict ⇒
   **acyclique**. Sans voisin plus bas : **puits déclaré**, océan (nœud sous la mer) ou **lac terminal**
   (bassin fermé intentionnel).
2. *Profondeur bornée par définition.* Une marche d'au plus `MAX_BASIN_HOPS` + 1 = 49 bassins mesure la
   profondeur ; un bassin plus profond devient lui-même un puits (« lac plafonné », ses amonts aussi,
   ses avals gardent leur arête). Aucun chemin n'a donc plus de 48 arêtes inter-bassins et résoudre un
   bassin ne regarde jamais plus de 49 bassins : pas d'exploration d'un continent.
3. *Niveau fin.* Chaque cellule s'écoule vers la cellule de sortie de son bassin (ou son nœud si c'est un
   puits) en prenant, parmi ses voisins **du même bassin strictement plus proches de la cible**, le moins
   coûteux (altitude + petit tirage de méandre). La distance entière décroît : ni cycle, ni minimum local
   (au plus 98 pas par bassin, 101 mesurés pour tout un chemin). La sortie passe à l'entrée du bassin aval,
   de l'autre côté de la frontière. La direction d'une cellule est une **fonction pure** de
   `(graine, cellule)` qui lit son bassin, ses 4 voisins et leurs bords : **pas de remplissage de
   dépressions par tuile, pas de couture** ; deux régions séparées ou réunies lisent les mêmes valeurs.
4. *Contrat de bord* (`EdgeContract`) : fonction pure de l'arête ordonnée (A, B) : cellules de sortie et
   d'entrée, identité du bassin versant, accumulation, largeur, niveaux d'eau de part et d'autre
   (`level_out >= level_in`). Les deux voisins appellent la même fonction.
5. *Profil d'eau.* Niveau d'une cellule = maximum, sur son chemin vers l'exutoire, de
   `altitude du nœud - 1,5`, plancher à la mer ou au niveau du lac terminal : il **ne remonte jamais vers
   l'aval** (flottant et après `floor`). Si le terrain monte vers l'aval, le terrain est rehaussé
   (remblai, au lot 4), pas l'eau.
6. *Accumulation* : nombre de cellules en amont (elle comprise), exact jusqu'à `ACCUMULATION_CAP` = 384
   puis saturé ; donc croissant vers l'aval (confluences : l'accumulation de l'aval couvre la somme des
   affluents, vérifié). Largeur = `1,4 + 0,9 × acc^0,62` (au plus 40 blocs).
7. *Lacs.* **Terminaux** : tout puits non océanique (rayon 100 à 180, niveau du nœud). **De passage** :
   cellule de rivière tirée (5 %), isolée dans un voisinage de 7 × 7, rayon 26 à 61 blocs, au niveau de la
   rivière ; son déversoir est l'arête aval (niveau inférieur ou égal : palier). Rivage déformé par un bruit.
8. *Vallées* (`MacroWorld::sample`) : bords de lit et berges selon la distance à la rivière (pente douce
   en plaine alluviale, raide en vallée encaissée), cuvette de lac ; l'altitude se fond dans le terrain
   sur 56 blocs (moins d'une cellule : les 5 × 5 cellules voisines suffisent).

### Tests du lot 3 (`tests/macro_world.rs`, 5 graines : 7, 42, 1337, 20260, 987654321)

| Exigence | Test | Résultat |
| --- | --- | --- |
| Déterminisme (séquentiel, inverse, 4 threads, caches vides, vidés en route, pleins, évincés à 64 entrées) | `samples_are_identical_in_sequence_reverse_parallel_and_any_cache_state`, `hydrology_cells_do_not_depend_on_evaluation_order` | passent, égalité exacte |
| Raccords (régions séparées/réunies aux bords et coins de quatre régions, négatifs compris ; contrat lu par les deux voisins) | `separate_regions_agree_with_a_united_region_at_edges_and_four_region_corners`, `edge_contracts_are_read_identically_by_both_neighbours` | passent (5 coins dont 3 en négatif ; plus de 200 arêtes par graine) |
| Acyclicité, terminaison déclarée et bornée | `every_path_is_acyclic_and_ends_at_a_declared_sink_within_the_bound` | 1 500 cellules terrestres par graine : aucun cycle ni cours interrompu ; profondeur max 16 bassins (borne 48) ; plus long chemin 101 cellules ; aucun puits plafonné |
| Aucune remontée, confluences, lacs | `water_never_climbs_downstream_even_after_rounding_to_blocks`, `lakes_have_a_level_and_an_outlet_or_are_declared_closed` | passent |
| Coût borné | `a_cold_query_evaluates_a_bounded_number_of_cells`, `resolving_a_basin_looks_at_a_bounded_number_of_basins` | voir les mesures |
| Géographie (continuité, terres 20-60 %, sommets non écrêtés, massifs) | `geography_is_continuous_across_region_borders_and_in_negative_coordinates`, `land_sea_chains_and_summits_look_like_a_world` | passent |

### Mesures (Apple M3 Pro, release, `cargo bench -p engine_voxel --bench hydrology`, contexte neuf)

| Mesure | Temps |
| --- | --- |
| Requête de point à froid, massif / plaine côtière / intérieur | 1,55 ms / 2,29 ms / 2,15 ms |
| Géographie seule (un point) | environ 0,8 µs |
| Résolution d'un bassin à froid | 0,44 ms |
| Zone de 1 024 × 1 024 blocs à froid (4 096 points, pas de 16) | 12,2 ms |

Altitudes évaluées par une requête de point à froid (cellules, avec un contexte neuf, 600 points) :
2 443 en moyenne, 5 375 au pire ; résoudre un bassin : au plus 1 764. La borne analytique (constantes :
25 cellules × chemin maximal et accumulation maximale) est de 1,9 million : très large ; le test fige
une borne mesurée de 12 000. Une carte de 16 384 blocs de côté (1 024 × 1 024 pixels) se calcule en 2,5 à
3,7 s sur 12 cœurs.

### Lisibilité des cartes (jugement)

Forces : continents et golfes variés, plateau continental, plaines côtières, massifs dont la ligne de
crête se lit (cols, arcs, flancs), piémonts, réseaux de drainage dendritiques qui se jettent dans la mer
ou dans des lacs fermés, rivières qui s'élargissent vers l'aval, vallées visibles sur l'altitude. Trois
graines examinées (42, 7, 1337, en vue large de 16 384 blocs, plus deux zooms), pas une vue choisie.

Défauts connus : (1) les **bassins versants sont des carrés de 512 blocs** sur la carte des bassins (toutes les
cellules d'un bassin suivent sa sortie) : limite du modèle à deux échelles ; (2) les cours d'eau montrent
des **segments alignés sur la grille** (surtout en plaine, le long des bords de bassins) et un
léger dentelage de 64 blocs ; (3) environ **21 % des cellules terrestres échantillonnées** drainent vers un
lac terminal (un puits intérieur par vallée fermée) : plausible (bassins endoréiques) mais fréquent ;
(4) les chaînes sont plutôt des **massifs en arcs** que de longues cordillères continues, et les fonds
marins montrent des tourbillons hérités du « warp » ; (5) un cours qui traverse une crête creuse un
canyon (ou est précédé d'un remblai) : le profil descend toujours, pas toujours le terrain ; (6) pas de
méandres de plaine ni de deltas au-delà du jitter des nœuds.

### Limites et ce qui n'est pas fait

- Aucun bloc (lot 4), aucun biome ni climat (lot 5), aucun lien avec les structures, les fluides stables
  ni les modifications du joueur : l'hydrologie statique n'est qu'un champ interrogeable.
- Pas de variante `GenerationVersion::V2` ni de choix de générateur à la création d'un monde.
- Les passes entre bassins sont des arêtes de 4 voisins ; il n'y a pas d'arête diagonale ni de
  tributaire qui traverse le bord hors du contrat. Le passage de l'eau d'un bassin à l'autre n'a lieu
  qu'à la passe la plus basse.
- La borne de profondeur (48 arêtes, 24 576 blocs) est une définition, pas un phénomène observé : elle
  n'est jamais atteinte sur l'échantillon (profondeur maximale 16). Un bassin plafonné deviendrait un
  lac terminal ; aucun test n'en a vu.
- La largeur est limitée à 40 blocs (accumulation saturée) : pas de fleuve plus large. Les lacs de
  passage font moins d'une cellule de rayon.
- Pas de mesure de mémoire en jeu (le prototype n'est pas branché au streaming) ; les tables de cache sont vidées au-delà de 262 144 entrées chacune.
