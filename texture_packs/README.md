# Packs de textures

Depuis le menu principal : **Packs de textures… → Terre, ardoise et cuivre · 32 × 32
→ Appliquer**, puis ouvrir ou créer un monde. Le choix est mémorisé dans les options
du joueur et s'applique aussi aux mondes déjà enregistrés. Pour revenir au rendu
d'origine, sélectionner **Classique · 16 × 16** puis appliquer.

Le pack fourni comprend **459 PNG RGBA en 32 × 32**, couvrant les textures de blocs,
les icônes d'objets et les matières des animaux et ennemis du jeu et des mods de référence.
Les principales matières sont
redessinées directement en 32 : touffes d'herbe, terre caillouteuse et racines,
roche éclatée, veines de minerai, nœuds et fibres dans le bois, feuillage en
bouquets, ferrures rivetées et fours maçonnés. Les outils, armures, fioles, livres
et gemmes ont de nouveaux profils et détails. Les petits accessoires gardent
leurs repères avec des finitions de matière. Les raccords périodiques des sols
sont conservés, sans contour de tuile systématique. Les dessins de base
en 16 × 16 restent dans `assets/core/` et `mods/`.

Le dossier `earth-copper-32/` contient `pack.ron` et
`assets/<espace de noms>/{textures,items}/*.png`. Seules les images sont remplacées :
aucune donnée de monde, recette, propriété de bloc, collision ou ressource sonore.
Une image absente revient à celle du jeu ou du mod ; le rendu adapte sa résolution.
Si le pack lui-même est absent ou invalide, le jeu démarre avec le pack classique.

Les PNG et le manifeste sont générés et versionnés. Pour les reproduire :

```sh
cargo run -p assetgen -- --pack32-only
```

La commande `--textures-only` régénère les dessins de base **et** ce pack. Modifier
`tools/assetgen/src/texture_pack32.rs` et ses modules dans `texture_pack32/`
pour faire évoluer les dessins 32 × 32.
F5 et le rechargement automatique prennent aussi en compte ses images.

Lors d'une distribution du jeu, conserver ce dossier à côté de `assets/` et `mods/`.
Le workflow Windows inclut le pack dans l'archive livrée.

Les [planches de contrôle](../docs/visual/13-pack-comparaison.png) comparent les
textures, leurs raccords et les objets en 3D. Une
[seconde planche](../docs/visual/13-details-comparaison.png) montre les équipements,
plantes et détails du mobilier, par paires 16/32. Les moyennes
linéaires de toutes les images sont accordées à l'origine ; le terrain lointain
garde ainsi sa teinte. Aucun motif ou fichier graphique de Minecraft n'est utilisé.

Les [animaux et ennemis en 16](../docs/visual/15-bestiaire-16.png) et
[en 32](../docs/visual/15-bestiaire-32.png) partagent leurs nouvelles formes et
articulations, avec des matières de peau, pelage, tissu, cuir, gel et armure.
Ces dessins sont dans `tools/assetgen/src/creature_skins.rs` et sont inclus dans
`--textures-only`. Les modèles changent au prochain lancement du jeu ; leurs PNG
se rechargent aussi avec F5.

Les [moutons, poules et poissons](../docs/visual/16-faune-32.png) possèdent aussi
leurs textures natives, ainsi que [leurs butins en 16 et 32](../docs/visual/16-butins.png).
Les [seize couleurs de moutons et de laine](../docs/visual/17-moutons-32.png)
partagent la palette `assets/core/wool_colors.ron`, utilisée par le jeu et assetgen.
