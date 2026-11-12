# Passe visuelle des blocs

Direction : terre, ardoise et cuivre. Références consultées le 12 novembre 2026 :
[coffre](https://www.minecraft.net/en-us/article/block-week-chest),
[four](https://www.minecraft.net/en-us/article/block-week-furnace),
[alambic](https://www.minecraft.net/en-us/article/taking-inventory--brewing-stand).
Ces références servent à lire les silhouettes et leurs pièces caractéristiques.
Les coordonnées, motifs et couleurs de cette passe sont dessinés dans le projet.
Aucune ressource externe n’est intégrée.

## Lot 1 — conteneurs et postes de travail

[Avant](01-conteneurs-avant.png) · [Après](01-conteneurs-apres.png)

Dix modèles : établi, four, lit, coffre, table d’enchantement, alambic, coffre-fort,
four à alliages, enclume et table des runes. Leurs PNG sont produits par assetgen.
Le four conserve la bouche `x=4..11, y=8..13` et son animation en maillage à boîtes.
Le test du feu vérifie toujours les quatre normales latérales, l’absence de feu
sur le dessus/dessous, l’isolation entre fours et le rechargement de l’état sauvegardé.
Il ne compte plus seize sommets : les pieds et la corniche ajoutent des faces.

Les tests CPU vérifient sélection, retrait et appui du joueur sur les dix postes,
ainsi que le passage du rayon sous l’établi. Le changement de hauteur est voulu :
lit à 9/16 (10/16 au chevet), coffre à 14/16, table d’enchantement à 14/16 au livre,
établi et four à 16/16. Les couleurs de repli sont recalculées en espace linéaire.

## Limites et validation en partie

Les planches sont des contrôles CPU, pas des captures de partie. La colonne « nuit »
est un éclairage réduit, sans simulation du shader ou de la propagation de lumière.
Le lancement natif avec `VOXEL_WORLD=/tmp/visuel-codex-20261112` est resté bloqué
sur les services macOS (`com.apple.hiservices-xpcservice`, connexion invalide).
Il a été arrêté. Aucun monde du propriétaire n’a été ouvert ou modifié.

Restent à vérifier en partie : ouverture des interfaces au clic droit, pose orientée,
déplacement autour des meubles, contacts contre un mur, juxtaposition, éclairage
nocturne réel et flammes animées. Les contrôles géométriques CPU ne remplacent pas
cette validation interactive. Aucun dossier `shots/` n’a été créé.

Les identifiants et toutes les propriétés non visuelles du registre sont comparés
à un export antérieur à la passe (couleurs, textures, boîtes et transparence dérivée
exclues de la comparaison). Aucun état n’est ajouté : les variantes décaleraient les
identifiants suivants et les échanges marche/arrêt ne transfèrent pas l’orientation.
Les formes fixes des circuits et des pistons ne prétendent donc pas suivre leur axe logique.
Le lot 4 ajoute des formes contextuelles aux câbles et tuyaux sans recourir aux états.
Les états et animations existants de Mécanique sont conservés.
Les exports [avant](registre-avant.tsv) et [après](registre-apres.tsv) sont identiques :
164 identifiants, chacun avec une empreinte des propriétés non visuelles.

## Lot 2 — circuits, réseaux et machines

[Modèles avant](02-machines-avant.png) · [Modèles après](02-machines-apres.png)
· [PNG des mods avant](02-mods-avant.png) · [PNG des mods après](02-mods-apres.png)

Les leviers, boutons, inverseurs et répéteurs ont un socle et des contacts en volume.
Les paires conservent leurs deux identifiants et leur logique de substitution.
Les pistons sont des modèles fixes verticaux, sans nouvel état ; leur axe visuel ne
représente donc pas toutes les directions logiques possibles. Les arbres du jeu de base
gardent leurs six branches fixes ; les câbles et tuyaux suivent leurs voisins depuis le lot 4.

Les réservoirs, pompes, batteries, lampes, moteurs, ventilateurs, remplisseurs,
assembleurs, panneaux, hélices, ballons et barres ont des pièces distinctes.
Mécanique conserve ses états Axis et ses rotations ; l’engrenage a des dents,
la presse un espace libre et la manivelle une poignée. Entrepôt reçoit un trieur
à trémie, une moissonneuse à dents et une foreuse sur portique. Donjons reçoit
des pointes étagées et des cages ouvertes. Le casseur d’Example et l’horloge
ont aussi des volumes propres.

Les 50 PNG des six mods sont désormais tous gérés par assetgen, y compris les deux
portails de Mécanique qui n’avaient pas de source reproductible. Le test de couverture
compare les PNG versionnés à une régénération dans un dossier temporaire.
Les objets de Forge réutilisent les silhouettes d’outils et armures du jeu ; les pièces
non montées, runes, baguette et sceau ont leurs dessins dédiés.

Les nouvelles colonnes « voisins » et « mur » montrent la juxtaposition et le contact
géométrique en rendu CPU. Elles ne valident pas l’occlusion du maillage GPU.

## Lot 3 — matières naturelles et petits volumes

[Blocs avant](03-nature-avant.png) · [Blocs après](03-nature-apres.png)
· [Matières avant](03-matieres-avant.png) · [Matières après](03-matieres-apres.png)

La torche est une tige avec une tête étagée ; le cactus est rentré de 1/16 sur ses
quatre côtés et garde sa hauteur pour les piles. Les deux terres labourées ont une
surface à 15/16, avec des sillons secs ou humides. Les cultures gardent leur croix
et leur origine de rendu : ce retrait du sol laisse donc un écart visuel de 1/16
sous leur cellule, à observer en partie. Aucune règle agricole n’est modifiée.

La pierre bleutée, le sable, la neige et la glace ont un grain périodique plus calme.
Les minerais dessinent des veines fragmentées ; les cinq bois partagent des cernes
arrondis et une construction d’écorce cohérente, avec un bouleau clair à lenticelles.
Les feuillages ont des bouquets moins saturés et des découpes périodiques. L’herbe et
la terre conservent leurs raccords antérieurs, vérifiés après réduction de palette.
Les plantes Cross, les modes de transparence, les liquides et le verre sont conservés.
Toutes les couleurs des PNG changés sont resynchronisées en espace linéaire.

Le test de raccord porte aussi sur les nouvelles matières. Les contrôles de sélection,
retrait, appui du joueur, juxtaposition et maintien de la face du mur parcourent tous
les modèles et leurs variantes (appui uniquement pour les solides).
La géométrie du cactus et la hauteur des deux terres labourées ont un contrôle dédié.
Au total, 60 définitions de blocs ont une forme modifiée ; aucun identifiant ni
paramètre de jeu n’a changé.

## Lot 4 — câbles et tuyaux raccordés selon leurs voisins

[Avant](04-connexions-avant.png) · [Après](04-connexions-apres.png)

Références Mekanism consultées : [Universal Cable](https://wiki.aidancbrady.com/wiki/Universal_Cable)
et [Mechanical Pipe](https://wiki.aidancbrady.com/wiki/Mechanical_Pipe).
Le principe retenu est le raccord automatique aux voisins compatibles et aux faces
ouvertes des machines, avec des dessins propres au projet.

`core:cable` et `core:pipe` utilisent désormais `connections` : noyau isolé, ligne,
coude, T, montée et jonction sur six faces. Le câble fait 4/16 de section ; le tuyau
6/16 avec un noyau de 8/16. Une branche existe seulement vers un conduit de même
sorte et canal, ou vers une machine avec la capacité correspondante ouverte sur
cette face. Une réserve vide accepte aussi le raccord. Les règles de transfert
existantes sont conservées.

La pose ou la casse d’un voisin modifie immédiatement la géométrie, y compris aux
frontières de chunks. Le même calcul alimente le maillage, les collisions et le
rayon. Le contour souligne le tronçon visé. Les machines dont le corps est en retrait
reçoivent un raccord dans leur propre cellule jusqu’à la première pièce que coupe
la section du conduit. Leurs interfaces restent associées au même bloc.

**Utilisation** : clic droit sur la face visée pour prolonger ou bifurquer ;
**Maj + clic droit avec le conduit en main** pour poser contre une machine sans
ouvrir son interface. Les contrôles existants de pose (case libre, support et
entités) restent actifs. Les objets en main, au sol et leurs icônes utilisent un
segment droit, indépendant des voisins du monde.

Les 64 formes sont calculées au chargement et choisies par les voisins, sans
ajouter de variantes ni de données sauvegardées. Les 164 identifiants restent les
mêmes ; l’export des propriétés non visuelles reste identique avant/après ce lot.
Le test d’audit exclut `connections`, comme les boîtes et les textures, car c’est
une propriété visuelle. Le format des sauvegardes n’a pas changé.

Les tests couvrent les 64 masques sur les six faces, les frontières de chunks,
les sortes et canaux incompatibles, les capacités et faces de machines, les
réservoirs multiples, la disparition réelle des branches à la visée/collision/rendu,
le contour du segment et la pose d’un coude, le déchargement/rechargement et les
identifiants. Un test de streaming vérifie que changer les ports remaillera aussi
le chunk voisin, tandis que changer une quantité ne déclenche aucun remaillage.
Le contrôle antérieur qui attendait exactement deux fois les faces d’un modèle
isolé est maintenu pour les modèles fixes ; les conduits sont maintenant vérifiés
par ces tests spécifiques, puisqu’ils ajoutent effectivement des branches.

Les écritures de capacités via les grilles ne changent que l’invalidation visuelle.
Aucun script, WASM, recette, butin, son ou monde du propriétaire n’est modifié.
La validation interactive du clic et du rendu GPU reste bloquée par l’environnement
macOS décrit plus haut. Le rendu CPU ci-dessus vérifie les géométries réelles.

```sh
cargo run -p voxel --example connection_review -- . /tmp/connexions.png
```

Les six contrôles demandés ont été exécutés pour ce lot : formatage, Clippy sans
avertissement, architecture, hooks et documentation passent. Le seul échec de la
suite complète est toujours le test de socket interdit par l’environnement.
La suite passe avec ce seul test exclu explicitement.

## Lot 5 — menus, infobulles et identification

Les menus, inventaires, options, fenêtres de machines et console utilisent des
fenêtres bornées à l'écran. Le contenu trop haut défile ; les cases et les colonnes
s'adaptent à la largeur disponible. Les listes de mondes, recettes, échanges et
écrans de mods peuvent revenir à la ligne. Le HUD du jeu se masque pendant les
menus et l'inventaire. Le journal de console défile séparément de sa saisie.

Les options sont regroupées en « Affichage et interface », « Audio » et « Souris
et touches ». Deux réglages sont ajoutés : `ui_scale` (75 à 150 %, défaut 100 %)
et `target_info` (activé par défaut). Les anciens fichiers de réglages continuent
à se charger avec ces valeurs par défaut.

Le survol d'une case affiche une petite fiche près du pointeur : nom, origine,
catégorie, quantité, durabilité restante, enchantement, dégâts, protection ou
nourriture selon l'objet. Les données proviennent des définitions du jeu ; les
bits d'enchantement sont séparés de l'usure avant de calculer la durabilité.
Les infobulles sont contraintes à l'écran et peuvent défiler sur un écran court.

En jeu, une fiche centrée en haut identifie le bloc ou la créature visée, y compris
dans une structure mobile. Elle montre l'outil conseillé/requis selon les règles
de récolte existantes, la vie des créatures, et les réserves d'énergie, fluides,
rotation ou stockage des machines. Elle réduit ses détails sur les écrans courts.
Le principe reprend l'identification de [Jade](https://modrinth.com/mod/jade),
avec un dessin propre au jeu. La mise à l'échelle et le placement des infobulles
ont aussi été étudiés dans les références officielles de
[Minecraft](https://www.minecraft.net/en-us/accessibility) et de la
[version 1.19.4](https://feedback.minecraft.net/hc/en-us/articles/13987663727757-Minecraft-Java-Edition-1-19-4).
Aucune ressource de ces projets n'est intégrée.

Planches inspectées :

- [Menu à 640 × 360](05-menu-640.png).
- [Options à 640 × 360](05-options-640.png), [320 × 240](05-options-320.png)
  et [1280 × 720](05-options-1280.png).
- [Cases et infobulle à 640 × 480](05-inventaire-640.png)
  et [320 × 480](05-inventaire-320.png).
- [Fiche de visée à 640 × 360](05-visee-640.png).
- [Comparaison : ancien conteneur sans contrainte de hauteur](05-options-conteneur-libre.png).

Ces images sont des rendus CPU d'egui. Le menu et les options emploient les widgets
de production ; les planches d'inventaire et de visée sont des contrôles des cases,
icônes et fiches communes, sur fond uni. La comparaison « ancien conteneur » est
une reconstitution de son débordement, pas une capture de l'ancien jeu. Le lancement
natif reste bloqué par les services macOS décrits plus haut : les interactions dans
une vraie partie, le plein écran et le changement d'échelle sur écran Retina restent
à vérifier. Aucun monde ni réglage du propriétaire n'a été écrit par ces contrôles.

```sh
cargo run -p voxel --example ui_review -- menu 640 360 /tmp/menu.png
cargo run -p voxel --example ui_review -- options 640 360 /tmp/options.png
cargo run -p voxel --example ui_review -- inventory 640 480 /tmp/inventaire.png
cargo run -p voxel --example ui_review -- hud 640 360 /tmp/visee.png
```

Sept tests supplémentaires couvrent les fenêtres pendant le redimensionnement de
320 × 240 à 1920 × 1080, les neuf cases à 150 % sur petit écran, l'accès au bouton
Retour par défilement, les infobulles au bord inférieur droit, les données d'usure,
les règles de récolte et la compatibilité des réglages antérieurs. Les six contrôles
demandés ont été exécutés : formatage, Clippy sans avertissement, architecture,
hooks et documentation passent. La suite complète ne rencontre que le test de
socket déjà interdit par l'environnement ; elle passe en excluant explicitement
ce seul test, sans changement de sa source.

## Lot 6 — composition de menus et inventaires plus proche de Minecraft

Cette seconde passe remplace le grand cadre des menus par un titre détaché et des
actions centrées au-dessus du monde. Le logo « VOXEL » est dessiné avec cinq glyphes
originaux en géométrie ; les deux actions principales sont larges, puis Options et
Quitter partagent une ligne. Les boutons gris ont un relief carré, une ombre sous le
texte et des états de survol, focus et pression. La pause utilise la même composition.
Aucune texture, police, marque ou image de Minecraft n'est importée.

Les inventaires et machines sont centrés dans un panneau clair avec texte sombre,
bords biseautés, cases en creux et sélection claire. Les cases mesurent au plus
36 points, les espacements sont réduits, le livre de recettes est près de
l'artisanat et l'aide détaillée se replie. La barre rapide partage ce dessin ;
les barres d'usure et l'objet tenu par le curseur suivent la taille des cases.
Les infobulles gardent leurs propriétés réelles, sur fond sombre à liseré violet.

Les options utilisent les onglets Affichage, Audio et Commandes. Les réglages
défilent dans leur propre zone, tandis que les onglets et Retour restent visibles.
Les tests simulent notamment un clic sur Audio, le défilement, le redimensionnement
des inventaires jusque 213 × 160 points logiques et la visibilité des quatre actions
principales sans défilement à 320 × 240. Les contrôles antérieurs sur les neuf cases,
les infobulles et la compatibilité des réglages restent actifs.

Comparaisons : [menu précédent](05-menu-640.png) / [nouveau menu](06-menu-640.png),
[cases précédentes](05-inventaire-640.png) / [nouvel inventaire](06-inventory-640.png).
Autres vues : [menu 1280 × 720](06-menu-1280.png), [menu 320 × 240](06-menu-320.png),
[pause](06-pause-640.png), [options](06-options-640.png),
[options 320 × 240](06-options-320.png), [inventaire 320 × 480](06-inventory-320.png)
et [fiche de visée et barre rapide](06-hud-640.png).

Ces aperçus CPU utilisent les widgets du jeu. La composition armure/artisanat est
commune à l'inventaire de production et à sa planche ; les objets affichés sont un
jeu de contrôle. Le fond uni remplace le monde 3D dans ces planches uniquement.
Le lancement natif reste bloqué par les services macOS décrits plus haut ; le rendu
sur le monde et les interactions complètes en partie restent à confirmer.
Les captures du lot 5 sont conservées comme comparaison historique.

Les six contrôles demandés ont été exécutés : formatage, Clippy, architecture,
hooks et documentation passent. La suite complète rencontre uniquement l'interdiction
de socket déjà décrite ; les autres tests passent avec ce seul test exclu explicitement.
Les mondes, recettes, règles du jeu, scripts et WASM sont inchangés par cette passe.

## Lot 7 — contenu visible dans les conduits

![Eau, gaz et objets en circulation](07-transports.gif)

[Planche fixe](07-transports.png). Les scènes de contrôle contiennent un producteur,
un coude et un récepteur. La vapeur est un fluide gazeux déclaré uniquement dans la
fixture de contrôle ; aucun nouveau bloc ou fluide du jeu n'est ajouté.

Les transferts de fluides et d'objets émettent maintenant des traces de dessin.
Les tuyaux de fluides et les transporteurs d'objets fermés ont un cadre métallique
et des fenêtres translucides. Les collisions et la sélection conservent les boîtes
antérieures. Les courroies montrent les objets au-dessus du tapis, avec leurs vrais
modèles : pixels extrudés des objets et textures des blocs.

Les liquides utilisent un volume coloré et des reflets mobiles. Un fluide dont la
densité est négative utilise une brume colorée et des particules. La couleur et la
lumière propre proviennent du registre des fluides. Les instances reçoivent la
lumière du voisinage des fenêtres ; la lumière propre reste un effet sur le contenu,
sans modifier la propagation lumineuse du monde.

Les traces reprennent les paires et les quantités de la répartition existante. Le
trajet visuel suit les faces raccordées, les coudes, les sens uniques et le canal,
sans traverser de machine intermédiaire. Les fluides conservent les faces exactes
des réservoirs utilisés. Les chemins sont mis en cache et invalidés après un
changement de réseau. Le contenu est masqué si son trajet a été cassé ou retourné.
Il suit la position et la rotation des structures mobiles. F5 vide aussi le cache
des modèles transportés.

Les objets se déplacent à environ quatre blocs par seconde, avec une animation
plafonnée à huit secondes pour les longues lignes. Les flux de fluides s'effacent
en 0,4 seconde sans nouveau transfert. Ce sont des animations de la livraison
instantanée existante : les inventaires, réserves, débits et délais logiques ne
changent pas. Les limites de dessin sont détaillées dans
[la documentation des conduits](../modding.md#le-contenu-visible-des-conduits).

La référence étudiée est la présentation des transports de Mekanism :
[Mechanical Pipe](https://wiki.aidancbrady.com/wiki/Mechanical_Pipe),
[Logistical Transporter](https://wiki.aidancbrady.com/wiki/Logistical_Transporter) et
[Pressurized Tube](https://wiki.aidancbrady.com/wiki/Pressurized_Tube).
Le dessin et les ressources restent ceux du projet.

```sh
cargo run -p voxel --example transit_review -- /tmp/transports
ffmpeg -framerate 10 -i /tmp/transports-%02d.png -filter_complex \
  '[0:v]split[s0][s1];[s0]palettegen=max_colors=128[p];[s1][p]paletteuse=dither=none' \
  /tmp/transports.gif
```

Le rendu CPU compose les mêmes boîtes, fenêtres, modèles et effets que le jeu,
avec les transferts calculés dans trois sous-grilles en mémoire. Il ne simule pas
le shader ou l'éclairage nocturne. Le lancement natif reste bloqué par les services
macOS décrits plus haut : rendu GPU des transparences, éclairage en partie et clics
sur les fenêtres restent à confirmer. Aucun monde du propriétaire n'a été utilisé.

Neuf tests supplémentaires vérifient les traces d'objets, d'eau et de gaz, les
quantités reçues, les coudes, la casse, les récepteurs pleins, les changements de sens,
les faces des réservoirs multiples, les fenêtres et les limites de mémoire. Ils vérifient aussi les modèles en mouvement,
l'extinction des flux, les transformations mobiles, la distance et le budget de dessin.
Les tests existants des réseaux et des collisions restent inchangés.

Les six contrôles demandés passent pour ce lot : `cargo fmt --check`,
`cargo clippy --workspace --all-targets` sans avertissement, `cargo test --workspace`,
`cargo xtask check-arch`, `cargo xtask hooks --check` et `cargo xtask docs --check`.
Le test de socket précédemment bloqué passe dans l'environnement actuel, sans
exclusion ni modification du test. Le nouveau lancement natif a en revanche encore
échoué sur `com.apple.hiservices-xpcservice`, avant l'ouverture du serveur de contrôle.

## Commits et patchs

L’écriture dans `.git` est interdite par l’environnement : `git add` échoue sur
`index.lock` avec `Operation not permitted`. Aucun commit n’a donc été créé.
Les six premiers lots sont fournis sous forme de patchs successifs dans `patches/`, avec leurs
messages de commit français dans les fichiers `*-message.txt`.
Ces patchs décrivent les changements déjà présents dans l’arbre de travail ; ne pas
les réappliquer à cet arbre. Ils permettent de reproduire les lots sur la révision
de départ, dans leur ordre numérique, puis de créer les commits correspondants.

Pour reproduire les planches :

```sh
cargo run -p voxel --example block_review -- . conteneurs /tmp/conteneurs.png /tmp/registre.tsv
cargo run -p voxel --example block_review -- . machines /tmp/machines.png
cargo run -p voxel --example block_review -- . nature /tmp/nature.png
cargo run -p voxel --example asset_review -- . mods /tmp/mods.png
cargo run -p voxel --example asset_review -- . terrain /tmp/matieres.png
```

La première racine peut être une copie antérieure de `assets/` et `mods/`.

## Contrôles des lots 1 à 3

`fmt --check`, `clippy --workspace --all-targets`, `check-arch`, `hooks --check` et
`docs --check` passent. `cargo test --workspace` échoue uniquement sur
`control::tests::a_client_gets_one_reply_per_line` : la création de la socket locale
renvoie `Operation not permitted`. Ce test existant n’a pas été modifié ni ignoré
dans le code. La suite passe avec ce seul test exclu explicitement. Les six commandes
demandées ont été exécutées pour chaque lot ; leur état final est le même. Clippy
ne signale aucun avertissement ajouté. La régénération compare les 281 PNG versionnés.

Les propriétés non visuelles des objets ont également été comparées aux sources
antérieures : seules leurs couleurs de repli ont changé. Les scripts, WASM, recettes,
butins et sons sont inchangés ; `saves/` et `.idea/` n’ont pas été modifiés par la passe.
