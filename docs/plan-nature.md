# Plan : une nature plus vivante

La demande : des races de vaches, des alpagas et des chevaux ; des forêts plus denses, des arbres plus
variés et plus grands ; des champs de blé, d'orge et d'avoine sauvages, des herbes hautes, des buissons
à fruits, des champs de fleurs ; un vent plus vivant, des feuilles qui tombent, et des forêts de
cerisiers (sakura) dont les pétales tombent.

## Ce qui existe

| Sujet | Aujourd'hui |
|---|---|
| Animaux | cochon, vache, mouton (toisons colorées, tonte), poule, poisson ; élevage par nourriture ; une seule vache, dans les plaines |
| Arbres | 19 formes (`TreeKind` : chêne, grand chêne, bouleau, sapin, grand sapin, acacia, jungle, marais, séquoia, érable, fruitier en fleurs, cyprès, olivier...) ; un chêne fait 4 à 8 blocs de haut |
| Densité | `tree_density` par biome, en arbres pour 32 × 32 colonnes : forêt 10, forêt de bouleaux 10, taïga 9, forêt sombre 18, jungle 14 ; soit environ un arbre pour 100 colonnes en forêt |
| Plantes | herbe haute, fougère, 6 fleurs, champignons, buisson mort, lianes, algues ; posées par biome (`plants`), en taches |
| Cultures | blé, carotte, pomme de terre, chou, melon (4 stades chacun), seulement plantées par le joueur |
| Vent | une oscillation régulière de 5 cm au plus des 42 blocs `sway: true` (feuilles, herbes, fleurs, cultures) : pas de rafales, pas de direction, pas plus forte sous l'orage ; les plantes plient déjà depuis leur pied (seuls leurs sommets du haut bougent) |
| Particules | l'eau seulement (`render/water_fx.rs` : éclaboussures, rides, bulles), des cubes dessinés par la fenêtre |
| Biomes | 47 en génération v3, dont prairie fleurie, champ de lavande, verger, bois d'érables |

## Ce que font Minecraft et les mods

- **Les cerisiers** (Minecraft 1.20, [Cherry Grove](https://minecraft.wiki/w/Pink_petals)) : un biome de
  cerisiers aux feuilles roses qui laissent tomber des particules de pétales, et un sol couvert de
  [pétales roses](https://minecraft.wiki/w/Pink_petals), un bloc posable qui s'empile comme les
  cornichons de mer (jusqu'à quatre par case) à la place des fleurs ordinaires.
- **Les vaches** (Minecraft 1.21.5, [les trois variantes](https://www.bisecthosting.com/blog/all-minecraft-1-21-5-cow-variants)) :
  tempérée (la classique, brune tachée de blanc), froide (pelage clair et long, cornes vers l'avant, dans
  la taïga et les collines venteuses), chaude (pelage roux, cornes relevées, dans la savane, la jungle et
  les badlands). La variante dépend du biome d'apparition.
- **Les chevaux** (Minecraft, [la page du cheval](https://wiki.ronlab.site/content/minecraftwiki_en_all_maxi_2025-11/Horse)) :
  7 robes (blanc, crème, alezan, bai, noir, gris, bai foncé) × 5 marques (aucune, balzanes et liste,
  champ blanc, taches blanches, points noirs) = 35 combinaisons, toutes aussi probables ; en
  troupeaux de 2 à 6 dans les plaines et la savane. Des mods comme [SWEM Addendum](https://www.curseforge.com/minecraft/mc-mods/swem-addendum)
  ajoutent des races (frison, arabe, fjord...) et des marques de tête et de pattes.
- **Les alpagas** : pas dans Minecraft (le lama en tient lieu). Des mods en ajoutent :
  [Alpacas!](https://www.curseforge.com/minecraft/mc-mods/alpacas) (des alpagas de couleurs pastel qui
  apparaissent dans le monde), [Llamarama](https://modrinth.com/mod/llamarama) (des lamas laineux en
  montagne, qu'on tond pour une laine à part).
- **Les feuilles qui tombent** ([Falling Leaves](https://www.curseforge.com/minecraft/mc-mods/falling-leaves-forge)) :
  des feuilles se détachent du dessous du feuillage et suivent un champ de vent procédural qui change
  avec la météo : presque rien par beau temps, poussées de côté pendant l'orage.
  [Falling Leaves Plus](https://www.curseforge.com/minecraft/mc-mods/falling-leaves-plus) y ajoute des
  débris propres à chaque essence (brindilles, glands, cônes).
- **Les cultures sauvages** ([Farmer's Delight](https://blog.curseforge.com/farmers-delight-mod-faqs/),
  [Wildy Wheat](https://modrinth.com/mod/wildy-wheat)) : des plants sauvages qui poussent dans l'herbe
  selon le biome et donnent des graines quand on les casse.
- **Les baies** (Minecraft, [baies sucrées](https://minecraft.net/article/taking-inventory--sweet-berries)) :
  des buissons dans les taïgas, qu'on récolte d'un clic droit et qui repoussent.
- **Les arbres** ([Dynamic Trees](https://www.curseforge.com/projects/252818)) : des arbres qui
  grandissent branche par branche ; serrés, ils montent plus haut et plus fins qu'isolés.

## Les lots

| Lot | Contenu | Vérification |
|---|---|---|
| **N1, le vent (réalisé)** | un vent global (direction, force, rafales qui passent en vagues sur le paysage), plus fort sous la pluie et l'orage ; les feuilles ondulent plus et selon ce vent ; les plantes (herbes, fleurs, cultures, fougères) se couchent bien plus que les feuilles. Tout se calcule dans le shader, depuis la météo que le serveur envoie déjà : rien de plus sur le réseau | captures et une courte vidéo au calme et sous l'orage ; le coût en image (`cargo xtask bench`) |
| **N2, les feuilles qui tombent (réalisé)** | des feuilles se détachent du dessous du feuillage proche du joueur et tombent en tournoyant, poussées par le vent de N1 ; leur couleur suit l'essence (et la teinte du biome) ; plus nombreuses par vent fort et en automne des érables ; un plafond de particules (quelques centaines), dessinées comme les éclaboussures de l'eau | captures en forêt, au calme et sous l'orage ; coût mesuré au bench |
| **N3, des forêts plus denses et des arbres plus variés (réalisé)** | densité relevée des forêts (×2 à ×3 selon le biome, réglée sur des captures) ; des arbres plus grands et moins réguliers : chênes et bouleaux de tailles variées, couronnes irrégulières, grands chênes à branches, **chêne noir** à tronc 2 × 2 pour la forêt sombre, bouleaux élancés, sapins géants ; sous-bois (buissons, jeunes arbres) ; le serré monte plus haut que l'isolé | cartes de biomes et captures avant/après ; temps de génération et de maillage mesurés (le bench), pour que la densité ne coûte pas d'images |
| **N4, les cerisiers (réalisé)** | bois, feuilles et planches de cerisier (textures dans les trois packs) ; un **cerisier** : tronc qui se divise, large couronne rose qui retombe ; un biome **cerisaie** (pentes tempérées, entre prairie et montagne) ; un **tapis de pétales** posable (1 à 4 par case) au sol à la place des fleurs ; les cerisiers lâchent des **pétales** (N2) plus que les autres arbres | carte des biomes (où sont les cerisaies), captures de jour et au couchant |
| **N5, les prés et les champs (réalisé)** | **orge** et **avoine** : graines, quatre stades, récolte, recettes (pain d'orge, porridge ; l'avoine nourrit les chevaux) ; des **champs sauvages** de blé, d'orge et d'avoine en taches dans les plaines, les prairies et la steppe (on y trouve les graines) ; **herbe haute sur deux blocs** ; **buissons à baies** (baies sucrées en taïga, myrtilles en forêt froide) qu'on récolte d'un clic droit et qui repoussent ; des **champs de fleurs** : biome de tournesols et prairies de fleurs plus fournies, quelques fleurs nouvelles | captures des prés ; un test de pousse et de récolte |
| **N6, les races de vaches (réalisé)** | un mécanisme de **variantes** pour les créatures (une variante tirée à l'apparition selon le biome ; textures ou teintes et parties propres ; transmise aux petits ; envoyée au client avec l'apparence) ; trois vaches : **tempérée** (l'actuelle), **des Highlands** (froid : longs poils roux, frange, grandes cornes larges), **à bosse** façon zébu (chaud : robe claire, bosse, cornes en lyre) ; elles se croisent entre elles | captures des trois ; tests du tirage par biome et de l'élevage |
| **N7, les alpagas** | un alpaga (cou long, toison épaisse) dans les prairies alpines, les hauts plateaux et la steppe, en petits troupeaux ; robe naturelle tirée à l'apparition (blanc, crème, fauve, brun, gris, noir) ; **on le tond** (laine de sa couleur, qui repousse en mangeant) ; il crache sur qui le frappe | captures ; tests de la tonte et de la repousse |
| **N8, les chevaux** | un cheval aux 35 robes (7 couleurs × 5 marques, avec les variantes de N6), en troupeaux de 2 à 6 dans les plaines, la prairie et la savane ; élevage à l'avoine ou à la pomme | captures de troupeaux et de robes |
| **N9, monter à cheval** | apprivoiser (le monter plusieurs fois jusqu'à ce qu'il accepte), une **selle**, le diriger (avancer, tourner, sauter, galoper), descendre ; le serveur fait foi, comme pour les wagons ; vu des autres joueurs | essai en jeu, en local et en LAN |

L'ordre propose d'abord le décor (N1 à N5), qui se voit partout, puis les animaux, qui reposent tous sur
les variantes de N6. Chaque lot se commite seul, après fmt, clippy, les tests, `check-arch` et un coup
d'œil en jeu.

## Ce qui reste vrai

- **Les mondes existants.** Les biomes, les arbres et les plantes viennent des données de génération ;
  les changer modifie les chunks pas encore générés des mondes existants, pas ceux déjà sauvegardés. À la
  frontière, une forêt peut passer d'un coup de clairsemée à dense (comme après une mise à jour de
  Minecraft). Le jeu le signale déjà dans le journal (« génération du monde : ... »).
- **Le multijoueur.** Le vent et les particules sont calculés par chaque fenêtre depuis la météo que le
  serveur envoie déjà : rien de plus sur le réseau. La force du vent suit les nuages, donc elle est la même
  pour tous ; le détail des rafales et la lente rotation du vent suivent l'horloge de chaque fenêtre. Les variantes des animaux partent avec leur apparence.
- **Les packs de textures.** Chaque bloc ou créature nouveau demande ses textures en 16, 32 et 64 px
  (générées par `tools/assetgen`, comme les autres).
- **Le coût.** Des forêts plus denses, ce sont plus de blocs à mailler et à dessiner : le bench du
  projet mesure avant et après, et la densité se règle sur ce qu'il dit.

## Réalisé

- **N1** : `Weather::wind` (`games/voxel_sim/src/world/weather.rs`) donne la direction (propre au monde,
  elle tourne lentement de part et d'autre de sa direction dominante) et la force (une brise de 0,15 à
  0,3 par beau temps, qui monte avec les nuages jusqu'à 1 sous l'orage, sans à-coup). La fenêtre la passe
  au rendu (`engine_render::Wind`, dans l'uniforme de la caméra). Le shader des chunks (`wind_offset`)
  couche feuilles et plantes dans la direction du vent, avec des rafales en bandes qui avancent avec lui
  et un frémissement plus vif quand il souffle fort ; le haut d'une plante (drapeau `FLAG_PLANT`, posé
  par le maillage) plie environ trois fois plus qu'une feuille et s'abaisse en se couchant. Au calme, le
  feuillage bouge comme avant (quelques centimètres) ; sous l'orage, une herbe haute se couche d'environ
  un tiers de bloc. Le masque des ombres ne bouge pas, comme avant. Vérifié : un test du vent (force,
  continuité, direction propre au monde), les shaders validés par les tests des packs, et des captures
  d'une savane au calme puis sous l'orage à 0,4 s d'écart. Pas mesuré au bench : le shader ne fait que
  quelques opérations de plus par sommet qui se balance. La pluie tombe encore droite.
- **N2** : `games/voxel/src/render/leaves_fx.rs`. La fenêtre examine 200 colonnes par seconde autour de
  la caméra (18 blocs de rayon) et y cherche le dessous du feuillage (une feuille au-dessus du vide) ;
  une feuille s'en détache avec une chance qui monte avec le vent (× 5,7 sous l'orage), et plus souvent
  sous les érables et les arbres en fleurs. Elle tombe à 0,75 bloc par seconde en se balançant et en
  basculant, portée par le vent de N1, se pose à plat sur le sol ou l'eau et s'efface en 4 secondes. Sa
  couleur est celle de son essence, teintée par le feuillage du biome, avec une nuance propre. Au plus
  320 feuilles, de petites boîtes plates dessinées avec les créatures. L'état de `ctl` donne leur nombre
  (`render.falling_leaves`). Mesuré à la lisière d'une jungle clairsemée (graine 3) : 13 feuilles en l'air
  au calme, 36 sous l'orage. Deux tests (chute, dérive, pose, plus de feuilles au vent, plafond).
- **N3** : **en génération v3 seulement** (les mondes neufs) : la génération v1 est figée par ses
  empreintes (`crates/engine_voxel/tests/generation_reference.rs`) et ses mondes ne changent jamais. Les
  biomes ont des valeurs propres à la v3 (`v3_trees`, `v3_tree_density` dans `biomes.ron`,
  `BiomeDef::trees_for`) ; la v3 tire 64 emplacements par colonne de chunks au lieu de 24. Densités v3 :
  forêt 34 (10 en v1), bouleaux 30, taïga 26, forêt sombre 40, forêt mixte 24, vieille taïga 22, jungle
  26, bois d'érables 22... Une partie des tirages tombe (troncs trop proches, eau, sol) : mesuré sur la
  graine 42, une forêt v3 a 18,7 arbres par colonne de chunks contre 9,6 en v1 (× 1,95, vérifié par un
  test). Nouvelles formes (`TreeKind`) : **chêne** v3 aux couronnes irrégulières, une fois sur deux en
  grappes au bout de courtes branches ; **bouleau** v3 irrégulier ; **grand bouleau** (10 à 14 blocs) ;
  **chêne noir** à tronc 2 × 2, racines, couronne plate et large ; **épicéa géant** 2 × 2 (21 à 26 blocs) ;
  **jeune chêne** pour le sous-bois. Au cœur d'un bosquet (facteur de bosquet au-dessus de 1,2), un arbre
  v3 gagne jusqu'à 3 blocs de hauteur. Le terrain lointain connaît les nouvelles formes. Les chênes noirs
  utilisent le bois et les feuilles du chêne (teintées par la forêt sombre) : pas de textures neuves.
  Coût mesuré (`cargo bench -p engine_voxel --bench generation -- "forêts v3"`, avant/après sur la même
  machine) : génération d'un chunk de forêt + 3 %, de forêt sombre + 8 % ; maillage + 24 % (forêt) et
  + 31 % (bouleaux) ; triangles d'un chunk de forêt 9 402 → 12 996, de bouleaux 4 002 → 8 296. Captures
  de la visite (`VOXEL_SHOTS=biome-`, mêmes vues) : la canopée de la forêt se ferme, la forêt de
  bouleaux double, la taïga devient une mer d'épicéas où le sol se voit peu. Pas vus en jeu : la forêt
  sombre et la vieille taïga (absentes de la visite) ; la présence des chênes noirs est vérifiée par le
  test.
- **N4** : bûche, feuilles, planches et pousse de cerisier, et **pétales roses** (un tapis posé à plat, un
  seul niveau au lieu des 1 à 4 de Minecraft), ajoutés à la fin de `blocks.ron` (les numéros des blocs
  existants ne bougent pas) ; recette des planches, butin des feuilles (pousse 5 %), familles `logs`,
  `planks`, `leaves`. Textures dans les trois packs, générées par `assetgen` : 16 px, 32 px (redessinées
  avec leurs détails, pas agrandies) et 64 px (tirées de l'essence « cherry ») ; 18 fichiers neufs, aucun
  existant réécrit. **Cerisier**
  (`TreeKind::Cherry`) : un tronc court qui se divise en deux ou trois branches en biais, chacune sous
  un dôme rose large et aplati dont le bord retombe. **Cerisaie** (`core:cherry_grove`, v3 seulement) :
  sur les collines du tempéré ni sec ni détrempé (érosion moyenne, pas les massifs : une première case
  la mettait sur des pics enneigés), 7 cerisiers par colonne de chunks, des pétales au sol à la place des
  fleurs. Les feuilles de cerisier lâchent des **pétales** (`leaves_fx`) : 4,5 fois plus que des
  feuilles, plus petits, du rose au presque blanc, qui tombent moins vite. Mesuré au cœur d'une
  cerisaie (graine 42, en −532, −452) : 308 pétales en l'air au calme (le plafond est de 320). Vérifié
  : la cerisaie apparaît en v3 et jamais en v1 (test), le cerisier tient dans les bornes des chunks
  (test), la vue « biome-cerisaie » de la visite et une capture sous les arbres.
- **N5** : **orge** et **avoine** (graines, quatre stades, récolte ; trois orges font un pain, deux
  avoines et des baies un **porridge**) ; **blé, orge et avoine sauvages** en taches dans les plaines,
  la prairie, la steppe et les forêts tempérées (on y trouve les graines) ; **herbe haute** et
  **tournesol** sur deux blocs (`blocks/tall_plant.rs` : la tête porte la clé du pied suivie de `_top`,
  les deux tombent ensemble) ; **buissons à baies** (baies sucrées, myrtilles) en trois stades : un clic
  droit cueille un buisson mûr, qui revient au stade du milieu et repousse ; **tulipe** et **allium** ;
  un biome **plaine de tournesols** (`core:sunflower_plains`, v3). Tout en v3 seulement (`v3_plants`).
  Tests : pied et tête liés, cueillette, une plante haute de la génération a toujours sa tête.
- **N6** : `games/voxel_sim/src/mobs/variants.rs`. Une créature peut lister des `variants` dans
  `entities.ron` (identifiant, nom, biomes, corps entier). À l'apparition, un groupe prend la race de son
  biome (corps de base ailleurs) ; les biomes des races s'ajoutent à ceux de la créature. Un petit prend
  la race d'un de ses parents au sort. La race (composant `Variant`) part au client dans l'apparence
  (`Look::variant`, protocole 3) et le rendu dessine son corps (`MobDef::parts_for`). `/summon <créature>
  [race]` (console et `ctl`). Deux races de vaches : **des Highlands** (taïgas, landes, toundra,
  hauts plateaux, prés alpins : robe rousse à longues mèches, poils qui pendent sous le ventre et sur
  les flancs, frange sur les yeux, grandes cornes larges relevées au bout) et **zébu** (savane,
  garrigue, maquis, jungle clairsemée, steppe : robe claire et rase, tête grise, bosse, fanon, oreilles
  tombantes, cornes en lyre). Les vaches apparaissent aussi sur le podzol. Quatre textures dans les trois
  packs. Les créatures ne sont pas sauvegardées : la race non plus. Vérifié : tirage par biome et
  croisement (tests), planche `creature_review -- . 32 sortie.png --breeds`, et les trois vaches en jeu.

## Choix à valider

| Question | Proposition | Pourquoi |
|---|---|---|
| Ordre des lots | N1 → N5 (le décor), puis N6 → N9 (les animaux) | le décor se voit partout tout de suite ; les animaux partagent le mécanisme de variantes |
| Races de vaches | des variantes d'une même vache (elles se croisent), comme Minecraft 1.21.5 | une seule vache à faire vivre (élevage, butin), et des variantes réutilisées par le cheval et l'alpaga |
| Monter à cheval | oui, en lot à part (N9) | c'est le plus gros morceau (apprivoiser, selle, conduite, réseau) ; les chevaux existent avant |
| Densité des forêts | ×2 à ×3 selon le biome, réglée sur captures et bench | assez pour qu'une forêt fasse forêt, sans payer en images |
| Orge et avoine | des cultures complètes (graines, récolte, recettes) | des champs sauvages sans usage n'auraient pas de sens ; l'avoine sert aux chevaux |
| Les mondes existants | accepter les changements dans leurs chunks neufs | figer les données de génération par monde est un chantier à part |
