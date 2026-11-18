# Terre, ardoise et cuivre

Le jeu conserve une lecture voxel familière et développe ses propres matières : verts
sauge, terre chaude, pierre bleutée, bois veinés, métal patiné et accents cuivrés.
Les dessins sont originaux et générés par le projet, sans textures, modèles ou polices
provenant de Minecraft.

![Planche de contrôle du thème et des objets](images/skin-preview.png)

Cette planche utilise le thème et les images du jeu, avec un rendu CPU sans fenêtre.

## Matières et objets

- PNG RGBA en **16 × 16**, échantillonnés sans flou. Les aplats se composent de groupes
  de pixels ; les détails isolés servent aux arêtes et aux cristaux.
- Palette ramenée à 32 niveaux par canal, ombres légèrement froides, reflets chauds.
  La transparence des plantes, feuilles et objets est conservée.
- Outils dessinés en diagonale, manche en bas à gauche. Les repères des manches sont
  vérifiés contre les icônes dans `games/voxel/src/hand_model.rs` ; la pose en main
  utilise l'origine centrée du modèle.
  La pioche porte une tête transversale à deux pointes ; la hache une large lame
  à talon ; la pelle un fer de bêche et une poignée en D.
- Les aliments ont des silhouettes dédiées. Le four conserve sa bouche aux pixels
  `x = 4..11`, `y = 8..13`, utilisés par les flammes animées.
- Les icônes des blocs sont des miniatures isométriques **32 × 32**, calculées à partir
  des textures de dessus/côté et des boîtes du modèle, puis mises en cache.

L'herbe et la terre utilisent un grain périodique : leurs pixels de bord se raccordent
après réduction de palette. Le dessus de l'herbe reçoit en plus une faible variation
continue dans le shader, sur plusieurs blocs et sans rupture entre quads greedy.
Le bord végétal latéral se raccorde horizontalement, sans contour sombre systématique.
La pierre, le sable, la neige, la glace, les écorces striées et les feuillages
emploient aussi un grain périodique dans `textures/nature.rs`. Les cinq essences
partagent la construction des cernes et des fibres, avec leurs couleurs propres.
Les minerais forment de petites veines fragmentées dans la pierre bleutée.

## Objets lâchés

Les piles dans le monde utilisent les faces et boîtes du bloc d'origine. Les outils,
armures et provisions sont extrudés à partir des pixels opaques de leur propre PNG ;
les bandes de même couleur sont fusionnées et mises en cache par objet. Les modèles
tournent, reçoivent la lumière du lieu et restent masqués par les murs. Les transparents
sont dessinés après les opaques, du plus loin au plus proche ; les feuilles conservent
leurs découpes. F5 invalide aussi ce cache.

L'action configurable **Jeter un objet** (`KeyQ` physique par défaut) lance un exemplaire
de la case sélectionnée devant le joueur. Son usure et son enchantement sont conservés.

Les couleurs `color` des blocs et des objets sont des moyennes **linéaires** de leurs
PNG sRGB, en excluant les pixels transparents. Elles servent aux objets au sol, au
terrain lointain et au repli sans texture. Le terrain lointain applique les mêmes
multiplicateurs de biome que les chunks détaillés.

## Son

Les matières doivent s'entendre : pas discrets sur sol meuble, bois mat, pierre
courte, métal résonnant et verre atténué dans les aigus. Les banques enregistrées
ont plusieurs prises sans répétition immédiate, une faible variation de hauteur et
des niveaux qui laissent de la place aux ambiances. Les boucles naturelles ont des
raccords fondus ; les musiques sont des morceaux calmes composés, lus en streaming.
Les sources externes et leurs licences sont créditées dans
`assets/core/sounds/CREDITS.md`, également livré avec le jeu. Les masters et recettes
sont dans `tools/assetgen/audio/` ; `cargo run -p assetgen -- --sounds-only` les
reproduit sans réseau. Voir la [passe sonore](audio/README.md) et ses extraits.

## Interface et personnages

`games/voxel/src/skin.rs` centralise la palette, les boutons, les cases, les cadres et les
jauges. Les menus sont superposés au monde, sans grande fenêtre d'encadrement : logo
original « VOXEL » dessiné en pixels, actions principales centrées et boutons gris
en relief. La typographie courante est à chasse fixe. Les inventaires et machines
utilisent un panneau minéral clair à bords biseautés, du texte sombre et des cases en
creux serrées. La sélection claire reste visible sur les petits objets ; les
infobulles sont sombres à liseré violet. Le cuivre reste un accent des matières du
jeu. Les losanges rouges indiquent la vitalité, les grains dorés la nourriture et
la barre sauge l'expérience. Les écrans des mods partagent le même thème.

Les fenêtres restent dans le viewport et défilent quand leur contenu est trop haut.
Les cases mesurent jusqu'à **48 points**, avec un pas commun de **2 points** entre
elles et une bordure d'icône de **2 points** au plus : une icône occupe 44 points
dans une case de 48, au lieu d'être réduite par une marge proportionnelle importante.
Les grilles du joueur et des conteneurs partagent leurs neuf colonnes. La largeur
des panneaux inclut explicitement le cadre et la barre de défilement ; l'en-tête,
les séparateurs et les grilles utilisent les mêmes bords. Les entrées et sorties
des machines restent alignées, les jauges sont centrées sur les cases et leurs
statuts longs utilisent une ligne séparée. Les libellés et contrôles des options
ont deux colonnes fixes sur les fenêtres larges.
Les cases, colonnes et lignes s'adaptent à la largeur disponible ; les options sont
regroupées par onglets (Affichage, Audio, Commandes) et proposent une échelle
d'interface de 75 à 150 %. Le bouton Retour reste hors du défilement des réglages.
Les infobulles utilisent le nom, l'origine et les propriétés réelles des objets.
Une fiche compacte en haut de l'écran identifie le bloc ou la créature visée,
avec l'outil requis et les réserves des machines quand la place le permet.
Cette fiche s'inspire du principe de Jade, avec nos cadres, couleurs et dessins.

Le bras porte une manche vert grisé et un poignet couleur cuir. Les habitants portent
des tabliers et accessoires associés à leur métier ; la faune utilise des tons naturels.
Le ciel passe du bleu profond au zénith à une brume claire à l'horizon. La
diffusion de la lumière réchauffe le ciel du côté du soleil. Les disques
solaire et lunaire sont agrandis pour rester lisibles. La lune montre ses
mers et ses cratères. Les étoiles de plusieurs magnitudes tournent avec le
cycle et se regroupent autour d'une faible bande galactique. Les nuages ont
une densité en trois dimensions et s'assombrissent dans leur propre ombre.
Ils sont calculés à demi-résolution sur chaque axe, puis composés derrière
le terrain. Le ciel reste indépendant du pack de textures et le brouillard
le relie au relief lointain. Les dimensions à ciel fixe gardent la couleur
choisie dans leur `dimensions.ron`.

Les outils et armes en main utilisent le modèle extrudé commun au sol et aux
transports : silhouette originale de l'icône, épaisseur d'un pixel et couleurs
linéaires. Le profil d'affichage reprend les paramètres `firstperson_righthand`
du modèle Minecraft `item/handheld` : rotation (0°, −90°, 25°), translation
(1,13 ; 3,2 ; 1,13) en seizièmes et échelle uniforme 0,68. Il s'applique au modèle
centré, après l'ancrage de vue (0,56 ; −0,52 ; −0,72) et la trajectoire de frappe.
La rotation latérale et la perspective rendent la grande face oblique ; le manche
peut sortir du cadre en bas ou à droite. Les familles n'ont plus de taille ou de
prise artificielle propre. Le format de la fenêtre change la projection, pas ce
profil. La main vide garde le bras. La projection de main reste à 70°, indépendante
du champ de vision du monde. Les réglages et le cache sont dans `hand_model.rs` ;
`hand.rs` relie les animations au joueur. Aucun dessin ni fichier graphique de
Minecraft n'est intégré au jeu.

Les blocs en main reprennent les volumes du modèle au sol et la découpe des
textures par face. Les fleurs et pousses utilisent leur silhouette extrudée.
La baguette garde sa prise sur le manche avec une échelle de 0,66. Le lit combine
une tête avec oreiller et un pied avec couverture de sauge. Les mêmes deux
moitiés composent sa miniature d'inventaire et son modèle au sol.

## Régénérer et contrôler

```sh
cargo run -p assetgen -- --textures-only
cargo run -p assetgen -- --pack32-only
cargo run -p voxel --example skin_preview -- docs/images/skin-preview.png
```

Les générateurs de surfaces sont dans `tools/assetgen/src/textures.rs`, ceux des objets
dans `tools/assetgen/src/textures/items.rs`. Les dessins du mobilier sont dans
`textures/workstations.rs`. La génération inclut les assets des mods de référence
Example, Mécanique, Entrepôt, Forge, Magie et Donjons, et reste déterministe.
`textures/machines.rs` dessine circuits et machines ; `textures/mod_assets.rs`
couvre notamment les runes, les pièces forgées et les blocs de crypte. Un test
régénère et compare chaque PNG versionné du jeu et des six mods.

Un [pack optionnel 32 × 32](../texture_packs/README.md) se choisit depuis le menu
principal avant de lancer un monde. Il possède une direction plus travaillée que
le classique : touffes d'herbe ombrées, terre avec racines et cailloux, éclats de
roche, minerais facettés, fibres et nœuds dans les planches, écorces et cernes,
bouquets de feuilles, maçonnerie du four et ferrures rivetées. Les outils, armures,
fioles, livres et gemmes ont des silhouettes et des détails dessinés directement
sur la grille 32 ; les autres petits accessoires conservent leurs repères avec
une finition propre à leur matière. Les cultures gardent leurs types et stades.

`texture_pack32.rs` orchestre les modules `texture_pack32/{terrain,wood,fixtures,
items,detail,draw}.rs`. Les motifs principaux ne dépendent plus de l'arrangement
des pixels du 16 × 16. Un contrôle à résolution commune 8 × 8 vérifie que leur
composition diffère encore après suppression du grain fin. Les sols et feuillages
se raccordent sur les deux axes, le côté enherbé horizontalement, sans cadre
sombre autour des blocs. La palette conserve 32 niveaux par canal et la bouche
du four conserve ses coordonnées UV (8..23, 16..27 en pixels 32).

La moyenne linéaire de chaque dessin, pixels transparents exclus, est accordée
à celle de sa source. Les 462 images sont contrôlées (écart inférieur à 0,02 par
canal) afin de garder les couleurs de repli et du terrain lointain cohérentes,
sans modifier les définitions de base. Les masques de végétation et d'équipement
sont adaptés au nouveau dessin ; le rôle de l'alpha reste inchangé. Le profil
d'affichage en main et l'épaisseur physique d'un seizième restent les mêmes.
Les haches placent leur grande lame du côté avant du manche dans ce profil,
comme les dessins classiques : le tranchant s'éloigne du joueur.

## Animaux et ennemis

Le cochon et la vache ont des volumes de poitrail, de croupe et d'encolure,
un museau en relief, des oreilles et des sabots distincts. Le rôdeur porte une
capuche et des vêtements plissés ; le cracheur de spores a un chapeau étagé, des
lamelles et un orifice de tir. Le slime a une enveloppe à paliers et des reflets.
Les gardiens portent des plaques séparées, épaulières, gantelets et bottes ;
celui de la crypte garde sa silhouette massive de pierre et sa visière chaude.
Ces dessins restent originaux et conservent les collisions et règles de combat.

Les vingt et une matières de peau, pelage, tissu, cuir, gel, champignon et armure viennent
de `tools/assetgen/src/creature_skins.rs`. Elles sont dessinées directement sur
les grilles 16 et 32 : fibres, coutures, rayures de forge et pores utilisent les
pixels supplémentaires. Les couleurs de repli des pièces sont les moyennes
linéaires des dessins 16 ; les dessins 32 sont accordés à ces moyennes.
La géométrie reste identique entre les deux packs. Le modèle animé commun est
dans `mob_model.rs` : tête et détails pivotent ensemble, patte et sabot partagent
leur hanche. Les petits utilisent les mêmes formes à l'échelle prévue par le jeu.
Les [planches du bestiaire](visual/README.md#lot-15--animaux-et-ennemis-en-16-et-32)
montrent les modèles réels sous quatre vues, dont la marche avec le regard tourné.
Les moutons ajoutent des volumes de toison et une face découverte ; les poules,
un bec, une petite crête, des ailes et des pattes distinctes ; les poissons, un
corps effilé, des branchies et des nageoires à rayons. La queue des poissons
possède un pivot commun et une oscillation latérale. Les huit butins associés
(laine, plume, viandes et poisson crus ou cuits) sont dessinés par `fauna_items.rs`
aux deux résolutions. Les nouvelles [planches de faune](visual/16-faune-32.png)
et [de butins](visual/16-butins.png) les présentent.
Les moutons ont seize teintes de toison, dans une palette sRGB commune aux fibres
du modèle et aux icônes de laine (`assets/core/wool_colors.ron`). La teinte est
appliquée en linéaire uniquement aux surfaces de laine : peau, yeux et sabots
gardent leur matière. Les teintes naturelles sont fréquentes et les couleurs vives
plus rares. Les petits reprennent la couleur d'un parent. La laine blanche garde
l'identifiant `core:wool`, avec quinze nouveaux objets et leurs dessins natifs
16 et 32. Les [planches de couleurs](visual/17-moutons-32.png) rapprochent chaque
mouton de sa laine.

## Mobilier et machines

Les boîtes décrivent les pièces réelles : pieds, plateau, cuve, couvercle, livre,
fermoir. Les détails suivent les coordonnées globales des boîtes ; un matelas bas
utilise donc les lignes centrales du PNG latéral. Le lit reste un couchage d’un bloc.
Les changements de collision liés aux boîtes sont intentionnels.

Les câbles et tuyaux suivent leurs connexions : noyau central, branches vers les
voisins compatibles et raccords jusqu’au corps des machines en retrait. La sélection
souligne le tronçon visé ; le segment droit représente le conduit en inventaire,
en main et au sol. Le cuivre reste visible sur la petite section du câble.
Le principe de raccordement est inspiré de Mekanism ; les matières restent celles
du projet. Les formes dépendent des voisins, sans variantes numériques supplémentaires.

En monde, les conduits de fluides et les transporteurs d'objets fermés ont des
arêtes métalliques et des fenêtres transparentes. Cette enveloppe de dessin garde
les boîtes physiques et le contour de sélection antérieurs. Les objets transportés
utilisent leurs vrais modèles texturés ; sur une courroie, ils passent au-dessus du
tapis. Les liquides montrent un volume coloré et des reflets mobiles, les gaz une
brume colorée. Les couleurs et la lumière propre viennent du registre des fluides.
Ces effets suivent les transferts du réseau et disparaissent à l'arrêt du flux.
Ils ne changent ni les réserves, ni la vitesse logique du transport.

Les planches de la [passe visuelle des blocs](visual/README.md) sont des rendus CPU
des formes et PNG du jeu. Leur colonne de nuit réduit la lumière de contrôle ; elle
ne simule ni la propagation lumineuse du monde ni les effets du shader.

Après un changement de dessin, **F5** recharge les textures, les miniatures et les
icônes extrudées en main. Les changements du thème et de la lumière, écrits en Rust,
nécessitent de relancer le binaire recompilé. Le mode `VOXEL_SCREENSHOTS` fournit aussi
les vues en jeu, dont les poses de repos, de minage et d'épée.

## L'eau

L'eau libre (liquide translucide sans lumière propre : `FLAG_WATER` dans le maillage) est animée
par le shader des chunks (`crates/engine_render/src/chunk_shader.wgsl`), sans remailler :
- la texture glisse et ondule, une houle de trois ondes incline la surface (les grandes faces
  fusionnées n'ont que quatre sommets : on ne déplace pas la géométrie) ;
- le ciel se reflète de biais (Fresnel) et le soleil laisse une tache qui scintille ; les vagues
  s'effacent avec la distance (18 à 70 blocs) pour ne pas faire de moiré ;
- **sous l'eau** (`Renderer::underwater`, de 0 à 1, lissé) : le rouge s'éteint avant le vert, le
  lointain devient un bleu sombre, des reflets de lumière dansent sur ce qui est près de la
  surface, et le fond de l'écran prend la couleur de l'eau. La surface se voit par en dessous.
`games/voxel/src/water_fx.rs` ajoute les effets décoratifs : gerbe de gouttes et rides au plongeon,
sillage quand on nage à la surface, bulles sous l'eau. La physique (`PlayerBody`) freine un plongeon
(`splash` donne la vitesse d'entrée), et un **courant** emporte les corps vers les cellules d'eau
moins remplies (`PlayerBody::current`). La lave n'est pas animée (elle émet de la lumière).

**Sortir de l'eau** : un nageur qui avance contre une berge monte seul une marche de 0,6 bloc au plus
(comme au sol) ; contre une berge plus haute (jusqu'à un bloc au-dessus de la surface), tenir Espace
donne un élan (`EXIT_SPEED`) qui le fait passer par-dessus. Une falaise plus haute reste hors de portée.

**L'écoulement** : un réservoir (source) alimente un ruisseau de 7 cellules au plus, de plus en plus
mince, au lieu d'inonder tout ce qui est plat (`fluid_sim.rs`, « portée »). En tombant, l'eau
retrouve toute sa portée en bas. À l'écran, la normale du dessus de l'eau suit la pente de la
surface (calculée au maillage) : le shader fait glisser la texture et les vagues vers le bas de la
pente, d'autant plus vite qu'elle est raide. Une chute (le côté d'une cascade, une pente très raide)
reste **translucide** : on y voit couler des filets plus clairs, des rubans verticaux de largeurs et
de vitesses différentes dont des bandes descendent (`fall_streaks` dans le shader), comme dans les
shaders d'eau de Minecraft, sans écume opaque.
