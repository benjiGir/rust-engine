# Biomes v4, révision 2 : des voisins cohérents

Retour de benji après un tour en jeu : « de la jungle collée à des forêts de chêne, c'est pas la folie ».

## Ce que dit la recherche (et ses limites)

Les sources web lues donnent peu de détails sur le fonctionnement interne des mods. Ce qui est établi :

- **Le choix multi-bruit de Minecraft** (la base de Terralith, Biomes O' Plenty, Regions Unexplored via TerraBlender,
  Biolith ou Lithostitched) : chaque biome a des plages idéales sur plusieurs paramètres (température, humidité,
  continentalité, érosion, étrangeté, profondeur), et un point prend le biome dont les plages sont les plus proches
  (distance euclidienne). C'est exactement notre `BiomeTable`.
  ([aperçu du système multi-noise](https://github-wiki-see.page/m/jacobsjo/snowcapped/wiki/Overview-of-the-Minecraft-multi-noise-system))
- **Regions Unexplored 0.6** a corrigé des « placements plus sensés, comme les séquoias qui ne bordent plus les
  jungles », et laisse régler le poids de chaque biome
  ([notes de version](https://www.curseforge.com/minecraft/mc-mods/regions-unexplored/files/8167399)). Même problème que le
  nôtre, résolu en retouchant les cases.
- **Tectonic** ne fait que le relief (continents, montagnes, océans) ; les biomes viennent d'ailleurs. **Terralith** et
  **Biomes O' Plenty** ajoutent des biomes dans l'espace climatique ; ensemble, ils donnent parfois de très petits
  biomes ([comparatif](https://minecrafthub.io/articles/terralith-vs-biomes-o-plenty)).
- **Temperature Bands** remplace les cartes de température et d'humidité par des bandes, et calcule l'humidité selon la
  proximité de la mer et des rivières
  ([page du mod](https://www.curseforge.com/minecraft/mc-mods/temperaturebands)).

Je n'ai pas trouvé de source qui décrive des « règles de voisinage » explicites. La pratique visible est donc : un
espace climatique bien découpé, des cases qui se touchent seulement quand les climats se ressemblent, et des poids pour
régler les proportions. C'est ce qu'on fait ici, avec une mesure en plus.

## Le diagnostic, mesuré

Au lieu de regarder à l'œil, `tests/generation_v4.rs` classe chaque biome de terre sur une **échelle de climats**
(polaire, froid, tempéré, chaud, tropical ou aride) et compte les frontières entre deux biomes qui ne sont pas voisins
sur cette échelle (ou jungle contre désert). Avant correction, **106 ‰ des frontières étaient fautives**, et presque
toutes venaient d'un seul biome : `wooded_hills` (collines boisées de chêne) avait une case de température de -0,45 à
0,55, donc il se posait sur des reliefs en pleine jungle (`jungle | wooded_hills` : 88 des 100 frontières fautives).
La jungle commençait aussi à 0,2, collée aux forêts tempérées.

## Ce qui est fait

- Collines boisées limitées au tempéré et au frais (température -0,3 à 0,15) : le relief des zones chaudes reste de la
  jungle ou de la forêt humide.
- Jungle à partir de 0,32 : entre le tempéré et la jungle, la forêt humide (0,1 à 0,45) et la lisière de jungle font
  la transition.
- Résultat : **1 ‰** de frontières fautives sur les graines 7 et 99. Le test impose au plus 15 ‰.

## Propositions pour la suite (à décider avec benji)

1. **(Fait, choisi par benji) Humidité côtière et fluviale** (idée de Temperature Bands) : plus humide près de la mer et des rivières, plus
   sec à l'intérieur des continents. Donne des déserts d'intérieur et des jungles côtières, plus crédibles.
2. **Garde-fou de taille minimale** : repérer les biomes de moins de N blocs (mesure sur la grille) pour éviter les
   confettis, avec une cible par biome.
3. **Variantes de relief par climat** : des collines et montagnes propres à chaque climat (collines de jungle,
   collines de taïga), pour ne plus emprunter un biome du mauvais climat.
4. **Poids par biome** : une proportion cible (la prairie 20 %, l'érablière 10 %...), vérifiée par test comme les
   frontières.
5. **Mesurer d'autres relations** : par exemple l'altitude des neiges selon la latitude, et les lacs gelés.

## La boussole

Objet `core:compass` (4 lingots de cuivre et 1 de fer en croix). Dans la barre d'objets, une règle de cap s'affiche en
haut de l'écran : N en rouge, NE, E, SE, S, SO, O, NO, et un repère central. Le nord est le sens des z négatifs, c'est
aussi la direction du froid (le grand nord glacial).

## Humidité côtière et fluviale (fait)

`terrain_v4.ron` : l'humidité ajoute une courbe selon la continentalité (+0,22 au rivage, jusqu'à -0,22 au cœur des
continents) et un petit bonus au fond des vallées (jusqu'à +0,1). Mesuré sur la graine 7 entre z = -1500 et 1500 :
humidité moyenne +0,24 sur les côtes, -0,22 à l'intérieur ; +0,18 au fond des vallées contre +0,07 sur les crêtes
(`v4_humidity_falls_with_the_distance_from_the_sea`). Effet attendu en jeu : forêts et jungles près des côtes,
prairies, steppes et déserts à l'intérieur, et des rivières bordées de verdure. Les tests de bandes de latitude et de
frontières cohérentes passent toujours.
