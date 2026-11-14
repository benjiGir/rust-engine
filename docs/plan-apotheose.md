# Plan : butin légendaire, attributs et enchantement avancé

Inspiré d'**Apotheosis** (Shadows_of_Fire, et ses bibliothèques Apothic Attributes, Apothic
Enchanting, Apothic Spawners), sans le copier : mêmes idées de progression, refaites à notre façon
et dans nos données.

## Ce que fait Apotheosis (résumé de l'étude)

- **Aventure** :
  - objets à **rareté**, avec des **affixes** tirés au sort (bonus d'attributs), trouvés sur les
    créatures et dans les coffres ;
  - **paliers du monde** (Haven → Pinnacle) qu'on débloque et qu'on choisit : ennemis plus forts,
    meilleur butin ;
  - **élites** et **envahisseurs** (annoncés par un son et un faisceau de lumière, butin garanti) ;
    augmentations aléatoires des créatures ;
  - **gemmes** à poser dans les **emplacements** des objets (l'effet dépend du type d'objet) ;
    sceaux pour retirer une gemme ou ajouter un emplacement ;
  - **table de récupération** (objets → matériaux de rareté), **table de réforge** (matériaux →
    objet neuf à affixes), table d'augmentation (relancer un affixe, très cher).
- **Attributs** : de nouveaux attributs (critique, vol de vie, perforation d'armure...), une fiche
  des attributs du joueur dans l'inventaire, l'armure comme seule réduction des dégâts.
- **Enchantement** :
  - la table lit trois stats sur les blocs posés autour d'elle (32 positions, deux anneaux) :
    **Eterna** (puissance, niveau max), **Quanta** (part de hasard), **Arcana** (nombre et
    rareté des enchantements) ;
  - chaque étagère a un plafond, et les stats secondaires « indices », « stabilité » et
    « trésor » ;
  - niveaux jusqu'à 100, plafonds d'enchantement relevés ;
  - **tomes** par type d'objet, tomes d'extraction ;
  - **bibliothèque** qui stocke les enchantements en points (niv. 1 = 1, niv. 2 = 2, niv. 3 = 4) ;
  - coût réel en points d'expérience.
- **Générateurs de créatures** modifiables (délai, nombre, ignorer la lumière...), **charmes de
  potion**, quelques retouches (jardin, villageois).

Sources : [dépôt](https://github.com/shadows-of-fire/apotheosis),
[Apotheosis](https://www.curseforge.com/minecraft/mc-mods/apotheosis),
[Apothic Enchanting](https://www.curseforge.com/minecraft/mc-mods/apothic-enchanting),
[Apothic Attributes](https://www.curseforge.com/minecraft/mc-mods/apothic-attributes),
[Apothic Spawners](https://www.curseforge.com/minecraft/mc-mods/apothic-spawners). Les chiffres
exacts (taux, coûts) ne sont pas publiés : on choisit les nôtres, en données.

## Ce qui existe dans le jeu

- **Enchantement** (`games/voxel/src/enchanting.rs`) :
  - **un seul** enchantement par objet (Efficacité, Tranchant, Solidité, Protection, niveau 1 à 3) ;
  - il est rangé dans les bits hauts de `ItemStack::damage` : usure sur les bits 0 à 15, niveau sur
    16 à 23, sorte sur 24 à 31 ;
  - table d'enchantement payée en niveaux d'expérience et en saphirs.
- **Piles** :
  - `inventory::ItemStack { item, count, damage }` (Copy) et `capabilities::ItemStack` (sauvegardé
    en postcard, format **non** auto-descriptif) ;
  - les sacs à dos rangent déjà un numéro dans `damage` et leur contenu dans `backpacks.bin`
    (`games/voxel/src/backpacks.rs`) : c'est le modèle à suivre.
- **Combat et survie** :
  - `mobs.rs` (`attack`, `damage_mob`) ;
  - `survival.rs` (`Vitals::hurt`, armure en points) ;
  - outils (`ToolDef` : dégâts, vitesse de minage, palier) et armures (`ArmorDef` : protection) ;
  - expérience en niveaux (`survival.rs`) ;
  - potions et effets (`potions.rs`).
- **Butin** :
  - `loot.ron` pour les blocs, les `drops` des créatures dans `entities.ron`
    (`survival.rs::drop_mob_loot`) ;
  - coffres des structures à paliers (`ChestLoot` dans `generator/structures.rs`, donjons du mod
    `donjons` avec boss et cages à gardiens).
- Pas d'arc pour le joueur. Pas de générateur de créatures dans le jeu de base, seulement dans le
  mod `donjons`.

## Phase 1 : fondations

1. **Données étendues par pile** :
   - un magasin d'objets du monde, `items.bin` à la racine du monde, sur le modèle de
     `backpacks.bin` ;
   - une pile « étendue » a le **bit 31** de `damage` à 1, et ses bits 0 à 30 donnent son numéro
     dans le magasin. L'enchantement actuel ne met jamais ce bit (sortes 1 à 4), et les sacs à dos
     ne sont pas des objets durables : le documenter et le vérifier par un test ;
   - une fiche : usure, enchantements (plusieurs), rareté, affixes, emplacements et gemmes, nom.
     Elle suit l'objet partout : inventaire, coffres, sol, sacs, wagons, sauvegarde ;
   - une pile simple qui reçoit une donnée étendue (un deuxième enchantement, un affixe) migre vers
     une fiche : l'usure et l'enchantement existants y sont recopiés ;
   - ne jamais changer le format des chunks. Les anciens mondes se chargent tels quels ;
   - une fiche qui ne sert plus (objet détruit) peut rester : noter la limite, pas de ramasse-miettes
     à cette phase.
2. **Attributs** (module pur et testé, côté jeu) :
   - les attributs : dégâts, chance critique, dégâts critiques, vol de vie, perforation d'armure
     (fixe et en %), points d'armure, vie max, régénération, vitesse de déplacement, vitesse de
     minage, chance de butin, portée, gain d'expérience ;
   - ils sont calculés depuis l'objet en main et l'armure portée (objet de base, enchantements,
     affixes, gemmes), plus les effets de potion, et recalculés seulement quand l'équipement change ;
   - ils sont branchés dans le combat (`mobs::attack`, `damage_mob`), les dégâts reçus
     (`Vitals::hurt` : l'armure reste la seule réduction, la perforation la contourne), le minage
     (`break_time`), le mouvement du joueur et le butin ;
   - une **fiche des attributs** est accessible depuis l'inventaire (un bouton). Elle liste les
     valeurs, avec le détail au survol et un filtre « modifiés seulement ».
3. **Plusieurs enchantements par objet** :
   - la table d'enchantement peut en donner plusieurs ;
   - les bulles d'aide (`ui_info.rs`) les listent tous ;
   - les enchantements existants gardent leur effet.

## Phase 2 : aventure (raretés, affixes, gemmes, élites)

1. **Raretés** : Commune, Peu commune, Rare, Épique, Mythique. Chacune a sa couleur dans les bulles
   et un liseré dans la case.
2. **Affixes**, en données (`assets/core/affixes.ron`, et `affixes.ron` dans les mods) :
   - chaque affixe a un nom (préfixe ou suffixe), les catégories d'objets auxquelles il
     s'applique (arme, outil, armure par emplacement) et un attribut avec un intervalle de valeurs
     par rareté ;
   - le nom de l'objet est composé (« Épée de fer acérée de la tempête ») ;
   - tirage : le nombre d'affixes et les valeurs dépendent de la rareté.
3. **Paliers du monde** :
   - cinq paliers, une sélection dans un écran (nouvelle touche dans les options) ;
   - le déblocage se fait par des critères en données : hostiles tués, boss de crypte vaincu,
     objet épique obtenu, profondeur atteinte ;
   - chaque palier renforce les créatures (vie, dégâts) et relève les chances de rareté. Le palier
     est sauvegardé dans `world.ron`.
4. **Sources de butin** :
   - une créature hostile a une chance de lâcher un objet à affixes ;
   - les coffres des structures en contiennent selon leur palier (s'accrocher à `ChestLoot`) ;
   - les **élites** (une part des hostiles, selon le palier) ont des augmentations tirées au sort
     (vie, dégâts, vitesse, effet de potion), un nom, une teinte ou un liseré de couleur, et un
     butin garanti ;
   - les **envahisseurs** (à partir du palier 2) sont annoncés par un son et un faisceau lumineux.
     Ils ont l'équipement et les stats d'une élite renforcée, et un objet à affixes garanti.
5. **Gemmes** :
   - six sortes (une par famille d'attributs) en cinq qualités. Leur bonus dépend de la catégorie
     de l'objet, et tout est en données (`gems.ron`) ;
   - les **emplacements** (0 à 3) sont tirés avec la rareté ;
   - poser une gemme : prendre la gemme en main (curseur) et cliquer sur l'objet dans l'inventaire ;
   - le **sceau de retrait** rend les gemmes, et le **sceau d'emplacement** ajoute un emplacement ;
   - les gemmes se trouvent sur les élites et les envahisseurs, et dans les coffres de donjon.
6. **Tables** (blocs avec écran) :
   - **table de récupération** : un objet à affixes y est détruit et rend des matériaux de rareté
     (un objet par rareté) ;
   - **table de réforge** : un objet de base, des matériaux de rareté et de la poussière de gemme
     donnent un objet à affixes de cette rareté. On voit trois tirages et on en choisit un, contre
     des niveaux d'expérience.

## Phase 3 : enchantement avancé et charmes

1. **Stats de la table** :
   - Eterna, Quanta et Arcana, lues sur les blocs posés dans les 32 positions autour de la table
     (deux anneaux, de l'air entre la table et le bloc) ;
   - chaque bloc de stats a une contribution et un plafond d'Eterna, en données dans `blocks.ron`
     (un champ `enchanting`), donc ajoutable par les mods ;
   - stats secondaires : indices (nombre d'enchantements montrés à l'avance), stabilité (le
     hasard ne fait que du bien), trésor (enchantements rares) ;
   - l'écran de la table montre les trois stats et les enchantements proposés ;
   - de nouvelles étagères originales (par exemple de saphir, de jade, d'obsidienne volcanique),
     avec textures générées.
2. **Niveaux** : niveau de table jusqu'à 100, plafonds des enchantements relevés selon l'Eterna.
   Le coût est retiré en **points** d'expérience réels, pas en niveaux.
3. **Nouveaux enchantements** : Butin (plus de butin), Vol de vie, Fortune (minerais), Toucher de
   soie, Aspiration (butin directement dans l'inventaire), Épines (renvoie des dégâts), Légèreté
   (moins de dégâts de chute), Vivacité (vitesse). Chacun passe par les attributs de la phase 1.
4. **Livres et tomes** :
   - un **livre enchanté**, posé sur un objet avec l'enchantement, à une table (ou une enclume
     simple, nouveau bloc) ;
   - des **tomes** par type (armes, outils, armures) qui n'acceptent que leurs enchantements ;
   - un **tome d'extraction** qui retire les enchantements d'un objet sur un livre, sans le
     détruire.
5. **Bibliothèque d'enchantements** (bloc) :
   - stocke les enchantements en points (niv. n = 2^(n−1) points), avec un plafond par
     enchantement ;
   - recherche, filtre par objet, extraction d'un livre au niveau choisi (jusqu'au plus haut niveau
     déjà déposé) ;
   - elle accepte les livres par les tuyaux d'objets.
6. **Charmes de potion** : un objet fait d'une potion, qui garde son effet actif tant qu'il est
   dans l'inventaire et qu'on l'active d'un clic droit. Il s'use avec le temps et se recharge avec
   la même potion.

### Phase 3 : ce qui est fait, et ce qui diffère du plan

- **Stats de table** (`enchant_table.rs`) : 32 positions (deux anneaux de 16 à deux blocs de la table ; l'air est exigé
  entre la table et l'étagère, **à la même hauteur**). Les contributions viennent du champ `enchanting` de
  `blocks.ron` (`EnchantingStats`, dans le moteur : un mod l'utilise sans toucher au jeu) ; la table a ses propres
  valeurs (base de Quanta et un indice). Eterna = somme plafonnée par le plus haut `max_eterna` présent (échelle 15, 40, 50,
  65, 100). Cinq étagères : chêne, saphir, améthyste, jade, obsidienne volcanique.
- **Niveaux et coût** : trois lots de niveau de table 1 à 100 (un tiers, deux tiers, tout l'Eterna, puis Quanta et
  Stabilité), prix de `4 × niveau` **points d'expérience** (pas des niveaux) et 1 à 3 saphirs (comme avant). Les
  enchantements sont en données (`enchantments.ron`) : niveau maximal sans Eterna (`max_level`, 3) et avec 100
  (`hard_max`, jusqu'à 8). Les anciens objets gardent leur numéro de sorte et leur niveau.
- **Douze enchantements** : les quatre anciens et Butin, Vol de vie, Fortune, Toucher de soie, Aspiration, Épines,
  Légèreté, Vivacité. Leurs effets sont des bonus d'attributs par niveau (cinq attributs ajoutés à la fin : `fortune`,
  `thorns`, `fall_resist`, `silk_touch`, `magnet`). Ils s'appliquent dans `gear.rs` et `enchant_effects.rs`.
- **Livres, tomes** (`enchant_books.rs`, `book_play.rs`) : un livre vierge, trois tomes par type et le tome
  d'extraction. Le livre ou le tome **posé en main** devant la table devient un livre enchanté (un tome ne reçoit que
  les enchantements de son type). Un livre enchanté (ou le tome d'extraction) tenu à la souris, un clic sur un objet,
  dans l'écran d'une table ou de l'**enclume simple** (nouveau bloc, sans stats ni coût en expérience).
- **Bibliothèque** (`enchant_library.rs`) : points `2^(n-1)`, plafond de niveau et de points par sorte en données
  (`library_level`), recherche, filtre « va sur l'objet en main », niveau d'extraction choisi jusqu'au plus haut niveau
  déposé, entrée par les tuyaux d'objets (case 0, refus rendus par la case 1). Cassée, elle rend ses points en livres.
- **Charmes** (`charms.rs`, `charm_play.rs`) : un objet par potion (`charms.ron`), clic droit pour l'activer, effet
  renouvelé chaque seconde tant qu'il est dans l'inventaire, usure d'un point par seconde, recharge par la potion au curseur.
- **Écarts et limites** : un livre ne coûte rien à appliquer ; une bibliothèque ne range que des livres enchantés (pas
  les objets) ; les tomes n'ont pas d'autre usage que de filtrer ce que la table y met ; le charme de soin rend de la vie
  toutes les dix secondes (un soin instantané ne dure pas) ; un charme vide reste dans l'inventaire (il ne se brise jamais).

## Hors périmètre (à décider plus tard)

- Générateurs de créatures modifiables : seul le mod `donjons` en a. Ce serait une extension de
  ce mod, ou un générateur de base à créer.
- Table d'augmentation (relancer un seul affixe), sceau de malice, retouches du jardin et des
  villageois.

## Contraintes pour l'agent

- **Une phase à la fois**, dans l'ordre. Chacune se termine verte et vérifiée en jeu avant la
  suivante, avec un compte rendu par phase.
- `cargo xtask check-arch` :
  - le moteur ne dépend pas du jeu, et la simulation ne dépend pas de l'affichage. Dans
    `engine_voxel` : ce qui doit passer par les blocs et les mods (stats de table en données,
    capacités) ;
  - aucun fichier `.rs` au-delà de 2 500 lignes. `games/voxel/src/main.rs` est déjà proche de la
    limite : du code dans de nouveaux modules, et seulement des appels d'une ligne dans `main.rs` ;
  - pas de fichier de réexport.
- Nouveaux blocs **à la fin** de `assets/core/blocks.ron`. Ne pas changer le format des chunks.
  Les anciens mondes, les anciens `settings.ron` et les objets déjà enchantés restent valides
  (tests à l'appui).
- Tout ce qui est réglable passe par les données (raretés, affixes, gemmes, paliers, stats des
  étagères, enchantements) : un mod peut en ajouter.
- Commentaires et textes en français, dans le style du projet. Textures générées par
  `tools/assetgen`, sans ressource externe.
- Tests unitaires :
  - aller-retour des données étendues, et migration d'une pile simple ;
  - calcul des attributs, et formule de dégâts avec armure et perforation ;
  - tirage des raretés et des affixes, déterministe avec une graine ;
  - pose et retrait des gemmes ;
  - récupération et réforge ;
  - stats de la table d'enchantement (positions, plafonds en échelle) ;
  - points de la bibliothèque.
- Vérifications : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --release`, `cargo xtask check-arch`, `cargo xtask hooks --check`.
- Essai en jeu dans un monde à part (`VOXEL_MUTE=1 VOXEL_CONTROL=1 VOXEL_SEED=3
  VOXEL_WORLD=apotest`), piloté par `cargo xtask ctl`. Ajouter les commandes d'essai utiles
  (donner un objet à affixes d'une rareté, faire apparaître une élite, régler le palier).
  - Ne jamais toucher à `saves/world`. Copier le `settings.ron` de l'utilisateur avant les essais
    et le remettre à l'identique après. Ne rien supprimer dans `saves/`.
- Ne rien commiter, ne rien pousser.
