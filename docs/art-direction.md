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
- Outils dessinés en diagonale, manche en bas à gauche. Les points de prise sont
  définis dans `games/voxel/src/hand.rs` et vérifiés contre les icônes.
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
Les cases, colonnes et lignes s'adaptent à la largeur disponible ; les options sont
regroupées par onglets (Affichage, Audio, Commandes) et proposent une échelle
d'interface de 75 à 150 %. Le bouton Retour reste hors du défilement des réglages.
Les infobulles utilisent le nom, l'origine et les propriétés réelles des objets.
Une fiche compacte en haut de l'écran identifie le bloc ou la créature visée,
avec l'outil requis et les réserves des machines quand la place le permet.
Cette fiche s'inspire du principe de Jade, avec nos cadres, couleurs et dessins.

Le bras porte une manche vert grisé et un poignet couleur cuir. Les habitants portent
des tabliers et accessoires associés à leur métier ; la faune utilise des tons naturels.
Les ciels et la lumière du soir reprennent cette palette.

## Régénérer et contrôler

```sh
cargo run -p assetgen -- --textures-only
cargo run -p voxel --example skin_preview -- docs/images/skin-preview.png
```

Les générateurs de surfaces sont dans `tools/assetgen/src/textures.rs`, ceux des objets
dans `tools/assetgen/src/textures/items.rs`. Les dessins du mobilier sont dans
`textures/workstations.rs`. La génération inclut les assets des mods de référence
Example, Mécanique, Entrepôt, Forge, Magie et Donjons, et reste déterministe.
`textures/machines.rs` dessine circuits et machines ; `textures/mod_assets.rs`
couvre notamment les runes, les pièces forgées et les blocs de crypte. Un test
régénère et compare chaque PNG versionné du jeu et des six mods.

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

**L'écoulement** : un réservoir (source) alimente un ruisseau de 7 cellules au plus, de plus en plus
mince, au lieu d'inonder tout ce qui est plat (`fluid_sim.rs`, « portée »). En tombant, l'eau
retrouve toute sa portée en bas. À l'écran, la normale du dessus de l'eau suit la pente de la
surface (calculée au maillage) : le shader fait glisser la texture et les vagues vers le bas de la
pente, d'autant plus vite qu'elle est raide. Une chute (le côté d'une cascade, une pente très raide)
reste **translucide** : on y voit couler des filets plus clairs, des rubans verticaux de largeurs et
de vitesses différentes dont des bandes descendent (`fall_streaks` dans le shader), comme dans les
shaders d'eau de Minecraft, sans écume opaque.
