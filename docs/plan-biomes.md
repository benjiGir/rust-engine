# Plan : des biomes plus grands et plus logiques (génération v3, révision du climat)

**Statut : implémenté (génération v4).** Réponses de benji : v4, période de 24 000 blocs, une teinte par
arbre, nord = z négatif. Les mondes v1 et v3 existants ne changent pas ; les nouveaux mondes sont en v4.

Ce qui existe : le nœud `ZTriangleWave` (`density_graph`), `terrain_v4.ron`, `biomes_v4.ron` et `surface_v4.ron`,
les biomes `snowy_plains`, `fjord`, `rainforest`, `arid_desert`, `great_prairie` (blé, orge et avoine sauvages en
grands champs) et `autumn_maples`, les arbres `MapleYellow/Gold/Red/Crimson` avec leurs feuilles et leurs textures
(base, 32 et 64). Tests : `tests/generation_v4.rs` (température croissante du nord au sud, réflexion, biomes par
bande de latitude). Carte : `cargo run --release -p engine_voxel --example worldmap -- --generator v4 ...`.
Reste à régler à l'œil : proportions des biomes, relief des fjords, densité des champs de blé.

## Pourquoi

Aujourd'hui (v3), la température et l'humidité sont deux bruits 2D indépendants
(`assets/core/worldgen/terrain_v3.ron`, `temperature` à `first_octave: -11`). Résultat : un désert peut toucher
une toundra, et les biomes n'ont aucune « logique géographique ». On veut :

- un **gradient de température nord-sud** : le nord est froid, le milieu tempéré, le sud chaud ;
- des biomes **plus grands** ;
- de nouveaux paysages : grandes prairies et champs de blé sauvage, forêts d'érables automnales multicolores.

## Idée centrale : la latitude

Dans le monde, l'axe **z** devient la latitude (z négatif = nord). On ajoute à la température un terme qui
dépend de z, et on garde le bruit actuel comme **variation locale** (plus petite) :

```
température = latitude(z) * 0.8 + bruit * 0.35
```

Le monde est infini, mais le gradient ne peut pas monter indéfiniment. On utilise donc une **onde triangulaire**
très longue (période d'environ 24 000 blocs) : de z = 0 (tempéré, là où on apparaît) vers le sud la chaleur
monte jusqu'au désert, vers le nord le froid monte jusqu'au glacial. Au-delà, la bande se **réfléchit** (elle
redescend) : on ne tombe jamais dans un désert « sans fin » ni dans un monde uniforme.

Pour le joueur, ce que ça donne en marchant vers le **nord** : forêt tempérée, puis taïga, plaines enneigées,
lacs gelés, fjords et côtes glacées. Vers le **sud** : forêt humide dense, jungle, puis savane et déserts de plus
en plus arides.

## Les changements, lot par lot

### B1. Le moteur : un nœud de latitude (petit)

- Nouveau nœud de densité `TriangleWaveZ { period, offset }` dans `density_graph/def.rs` et `graph.rs`
  (comme `YClampedGradient`, mais sur z, et périodique).
- `terrain_v3.ron` : `temperature = Add(Mul(latitude, 0.8), Mul(bruit, 0.35))`.
- Humidité : on la rend aussi **dépendante de la latitude** (sec aux deux extrêmes chauds, humide près de
  l'équateur), sinon le sud n'aurait jamais de jungle à côté du désert.
- **Test de passage :** tests unitaires du nœud (valeurs connues, périodicité, déterminisme) ; un test qui
  échantillonne l'axe z et vérifie que la température monte de façon monotone sur une demi-période.

### B2. Biomes plus grands

- Baisser la fréquence des bruits `temperature`, `humidity`, `variety` d'une octave (`first_octave` -11 → -12,
  etc.) : des plaques d'environ 2 fois plus larges.
- Les écotones (zones de transition) restent grâce à la proximité dans la table de `biomes_v3.ron`.
- **Test de passage :** carte de biomes (l'outil de captures existe déjà) avant/après, taille moyenne d'une plaque
  mesurée sur un échantillon.

### B3. Le nord : taïga, neige, lacs gelés, fjords

- Réorganiser les cases de `biomes_v3.ron` : bandes de température nettes (glacial / froid / frais / tempéré /
  chaud / brûlant), avec l'humidité pour départager à l'intérieur de chaque bande.
- **Fjords** : près des côtes froides, une érosion basse + continentalité moyenne creusent déjà des vallées
  encaissées. On ajoute un biome `core:fjord` (côte froide, falaises, eau sombre) et on règle le relief pour des
  entailles profondes et étroites.
- **Lacs gelés** : les lacs existants gèlent déjà sous 0,15 de température (voir `FREEZING` dans `v3.rs`).
  Vérifier qu'ils apparaissent bien dans la bande glaciale, avec de la glace épaisse et de la neige dessus.
- **Plaines enneigées** : un biome `core:snowy_plains` entre toundra et taïga (peu d'arbres, neige au sol).

### B4. Le sud : forêt humide, jungle, déserts croissants

- Bande chaude et humide : **forêt humide très dense** (nouveau biome `core:rainforest`, arbres hauts, sous-bois
  fourni, lianes, fougères), puis la jungle actuelle.
- Bande chaude et sèche : savane → garrigue → désert → **désert aride** (plus de sable pur, plus d'os de dune,
  presque aucune plante) puis plateaux rouges. L'aridité augmente avec z : les plantes disparaissent
  progressivement (nouveau biome `core:arid_desert` au fond de la bande).
- **Test de passage :** captures au sol à plusieurs latitudes, compte de plantes par colonne par bande.

### B5. Grandes prairies sauvages et champs de blé sauvage

- Le biome `core:prairie` devient **vaste** (la bande tempérée sèche), avec de très grandes plaques de blé, d'orge
  et d'avoine sauvages (aujourd'hui des touffes éparses à 1 % de chance).
- Une **taille de champ** par bruit à grande échelle : à l'intérieur d'un champ, la densité de blé monte à 40 %
  ou plus, hors champ elle retombe. Les cultures existantes (`wild_wheat`, `wild_oats`, `wild_barley`) sont
  déjà là, et le vent aussi.
- **Test de passage :** capture de prairie en vue large, densité mesurée dans / hors champ.

### B6. Forêts d'érables automnales à 4 ou 5 teintes

- Aujourd'hui `core:maple_leaves` n'a **qu'une** couleur (orange-brun). On ajoute **5 variantes de feuillage** :
  jaune, or, orange, rouge, rouge sombre.
- **Choix à valider :** chaque **arbre** prend une teinte (avec un bruit à moyenne échelle pour que des voisins se
  ressemblent un peu), plutôt que chaque bloc. C'est plus naturel et moins coûteux que du mélange par feuille.
  Les feuilles tombantes (`leaves_fx.rs`) reprennent la couleur de l'arbre le plus proche.
- Textures originales générées par `assetgen` (jamais copiées) pour les packs 32 et 64.
- Place dans la bande **fraîche et humide** (entre taïga et forêt tempérée), en grandes plaques.

### B7. Compatibilité et sauvegardes

- Les mondes existants (v1, v2, v3 déjà créés) **ne changent pas**, sinon leurs chunks déjà générés ne
  correspondraient plus aux nouveaux. On introduit donc une **version v4** de génération, ou un drapeau
  `climate_v2` dans le monde : à trancher (voir questions).

## Contraintes du dépôt

- Le nouveau code reste sous 2 500 lignes par fichier et 20 fichiers `.rs` par dossier (`check-arch`). Le dossier
  `worldgen/generator` en compte 7 : on a de la marge. Les biomes ajoutés sont des **données** (`biomes.ron`,
  `biomes_v3.ron`), pas du code.
- Aucun `index.ts` n'est impliqué (projet Rust).
- Tout le contenu (textures, noms, formes) est original, produit par `assetgen`.

## Ordre proposé et coût

| Lot | Contenu | Taille |
| --- | --- | --- |
| B1 | nœud de latitude + température/humidité | petit |
| B2 | biomes plus grands | petit (réglage) |
| B3 | nord (taïga, neige, lacs gelés, fjords) | moyen |
| B4 | sud (forêt humide, déserts) | moyen |
| B5 | prairies et champs de blé | moyen |
| B6 | érables 5 teintes | moyen (textures) |

On joue B1 + B2 d'abord et on regarde la carte, parce que tout le reste en dépend.

## Questions pour benji

1. **Versioning :** une nouvelle génération **v4** (propre, sans risque pour les mondes existants), ou on modifie
   la v3 telle quelle (les anciens mondes v3 auraient des cassures aux frontières des chunks déjà générés) ?
   *Recommandation : v4.*
2. **Taille du monde climatique :** période de 24 000 blocs (de l'équateur au pôle : environ 6 000 blocs), ça te
   va, ou plus court / plus long ?
3. **Érables :** une teinte par arbre (recommandé) ou par grandes plaques de forêt ?
4. **Nord = z négatif :** d'accord pour cette convention, et pour apparaître au milieu de la bande tempérée ?
