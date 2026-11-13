# Plan : coffres, stockage et sacs à dos (une phase)

Inspiré de deux mods de Minecraft, sans les copier :

- **Sophisticated Storage** : coffres et tonneaux à paliers (bois → cuivre → fer → or → diamant →
  netherite) qu'on améliore **sur place** avec un kit, sans perdre le contenu ; des **emplacements
  d'amélioration** qui ajoutent des fonctions (piles plus grandes, compactage, néant...) ; un
  **contrôleur** qui réunit tous les coffres voisins en un seul inventaire ; un tri en un clic.
- **Sophisticated Backpacks** : un sac à dos qu'on ouvre d'une touche sans le poser, à paliers lui
  aussi, avec les mêmes améliorations, plus celles d'un sac porté : ramassage filtré, aimant,
  réapprovisionnement de la barre d'objets, nourriture.

Et deux demandes directes :

- **Aperçu rapide** : voir le contenu d'un coffre en le regardant, sans l'ouvrir.
- **Déposer** : un bouton qui vide l'inventaire dans le coffre en complétant d'abord les piles déjà
  présentes.

## Ce qui existe

- Coffre de base `core:chest` : 27 cases dans la capacité `Inventory` de son entité de bloc
  (`crates/engine_voxel/src/containers.rs`), écran `Screen::Chest` (`games/voxel/src/ui.rs`,
  `chest_panel`, `container_slot`, `click_container`).
- Wagon coffre et wagon trémie (`games/voxel/src/carts.rs`, `Screen::Cart`).
- Grand coffre du mod Entrepôt (`mods/entrepot`, menu de mod `Screen::Mod`, déjà un bouton « Ranger »).
- Fiche de visée (`games/voxel/src/ui_target.rs`, réglage `target_info`) : nom, origine, capacités du
  bloc regardé.
- Piles côté jeu : `inventory::ItemStack { item, count, damage }` (Copy) ; côté moteur :
  `capabilities::ItemStack { item: String, count, damage }`, sauvegardé en postcard (format **non**
  auto-descriptif : ajouter un champ casse les sauvegardes). `damage` porte déjà l'usure (bits 0-15)
  et l'enchantement (bits 16-31, voir `enchanting.rs`).

## Périmètre de la phase

### 1. Coffres : aperçu et transferts rapides (les demandes directes)

- **Aperçu rapide** : quand on regarde un bloc qui a un inventaire (coffre de base, coffres à
  paliers, grand coffre de mod, tout bloc avec la capacité `Inventory`) ou un wagon coffre, la fiche
  de visée montre une petite grille d'icônes : les objets regroupés par sorte, avec leur total, triés
  par quantité. Au plus 12 icônes, puis « +N sortes ». « Vide » si rien. On respecte le réglage
  `target_info`, et une nouvelle option « Aperçu des coffres » (activée par défaut, `#[serde(default)]`).
- **Boutons sur tout écran de conteneur** (coffre, coffres à paliers, wagon coffre, sac à dos ; un
  module partagé, pas trois copies) :
  - **Déposer** : chaque pile de l'inventaire principal (les 27 cases, **pas** la barre d'objets)
    va dans le conteneur. D'abord en complétant les piles du même objet (même usure) déjà présentes,
    puis dans les cases vides. Ce qui ne rentre pas reste dans l'inventaire.
  - **Compléter** : comme Déposer, mais seulement pour les objets déjà présents dans le conteneur.
  - **Tout prendre** : l'inverse, vers l'inventaire du joueur.
  - **Trier** : regroupe les piles du conteneur et les range par catégorie puis par nom.
- **Maj + clic** sur une case : envoie toute la pile de l'autre côté (inventaire ↔ conteneur), avec
  la même règle d'empilement. Maj + clic droit garde sa fonction actuelle (diviser la pile).
- Toute la logique (déposer, compléter, prendre, trier, Maj + clic) dans un module pur et testé,
  sur des listes de cases, sans interface.

### 2. Coffres à paliers et améliorations (Sophisticated Storage)

- **Paliers** : coffre (27 cases, 1 emplacement d'amélioration) → coffre de cuivre (45, 2) → coffre
  de fer (54, 3) → coffre d'or (81, 4) → coffre de saphir (108, 5). Nouveaux blocs **à la fin** de
  `blocks.ron`, même modèle de boîtes que le coffre, textures générées par `tools/assetgen`.
- **Kits de palier** (objets) : clic droit sur un coffre posé du palier juste en dessous. Le bloc
  change, l'entité de bloc garde ses objets, et ses cases s'agrandissent. Les kits se fabriquent
  aussi pour aller directement du coffre de base au palier visé (recettes).
- **Emplacements d'amélioration** : rangés à part dans l'inventaire du bloc (par exemple les
  dernières cases, à documenter), affichés dans une colonne à droite de l'écran du coffre. Les tuyaux
  d'objets ne doivent ni y déposer ni y prendre.
- **Améliorations** (objets, recettes, icônes générées) qui marchent dans un coffre à paliers :
  - **Pile ×2** et **Pile ×4** : la taille maximale des piles dans ce conteneur est multipliée
    (elles ne se cumulent pas au-delà de ×8). Une pile trop grande pour l'inventaire du joueur se
    prend en plusieurs fois.
  - **Compactage** : 9 objets identiques sont remplacés par le bloc de la recette 3 × 3 qui les
    transforme (fer → bloc de fer, etc.), lu dans les recettes d'artisanat (`Recipes`) : aucune liste
    écrite à la main.
  - **Néant** : avec un filtre (quelques cases fantômes : on y dépose un exemplaire, rien n'est
    consommé), détruit les objets filtrés qui entrent ; sans filtre, seulement quand le conteneur est
    plein.
  - **Entonnoir** : prend chaque seconde une pile dans le conteneur au-dessus et pousse dans celui
    du dessous (en respectant les ports).
- **Contrôleur de stockage** (bloc) : il réunit tous les coffres à paliers et coffres de base reliés
  à lui par contact, jusqu'à 64 blocs dans un rayon de 16. Son écran présente un inventaire unique :
  les objets regroupés par sorte, avec une recherche (réutiliser `recipe_browser::matches`). Un clic
  prend une pile, Maj + clic tout, et Déposer remplit les conteneurs en complétant d'abord les piles.
  Un tuyau branché au contrôleur dépose dans le réseau.

### 3. Sacs à dos (Sophisticated Backpacks)

- **Sacs** (objets, `max_stack` 1) : sac en cuir (27 cases, 1 amélioration) → sac de cuivre
  (54, 2) → sac de fer (81, 3) → sac de saphir (108, 4). Montée de palier par recette (le sac et un
  kit), en gardant le contenu.
- **Contenu** : un magasin des sacs du monde, `backpacks.bin` à la racine du monde, à côté de
  `world.ron` (pas par dimension). Chaque sac a un numéro unique, rangé dans le champ `damage` de sa
  pile. Les sacs ne sont ni des outils ni des armures, et ne s'enchantent pas : il faut vérifier que
  `enchanting.rs` et l'usure ne les touchent jamais. Le magasin garde, par numéro : le palier, les
  cases, les améliorations et leurs réglages. Un sac neuf (fabriqué, `/give`, triche de
  l'explorateur) reçoit son numéro au premier usage. Un sac jeté au sol, mis dans un coffre ou
  ramassé garde son numéro. Un sac **ne peut pas** contenir de sac.
- **Ouvrir** : touche **B** (nouvelle action `Backpack` dans les options, `KeyB` par défaut, sans
  casser les anciens `settings.ron`) pour ouvrir le premier sac de l'inventaire, ou clic droit avec
  le sac en main. L'écran est le même que celui des conteneurs (boutons Déposer, Trier, etc.), plus
  la colonne des améliorations.
- **Améliorations de sac** : les mêmes objets que pour les coffres (pile, compactage, néant), plus :
  - **Ramassage** (avec filtre) : un objet ramassé par le joueur va d'abord dans le sac.
  - **Aimant** : attire les objets au sol dans un rayon de 6 blocs.
  - **Réapprovisionnement** : quand une pile de la barre d'objets passe sous un quart, elle est
    complétée depuis le sac.
  - **Nourriture** : quand la faim baisse assez (sans gaspiller), mange un aliment du sac.
  Une amélioration agit tant que le sac est dans l'inventaire du joueur (barre ou inventaire
  principal), sans qu'il soit ouvert. Chacune peut être coupée d'un clic dans la colonne des
  améliorations.

### 4. Contenu, outils, documentation

- Recettes (`recipes.ron`), objets (`items.ron`), blocs et textures. L'explorateur de recettes les
  montre tout seul.
- Commandes `ctl` pour les essais sans clavier : `backpack` (ouvrir le premier sac), `deposit`,
  `quickmove <case>` si besoin, et un moyen de lire le contenu d'un conteneur ou d'un sac (étendre
  `inventory` ou ajouter `container x y z`). Les boutons se cliquent avec `clicktext`.
- Documentation : `README.md` (commandes) et `docs/architecture.md` (où vit quoi : le magasin des
  sacs, les emplacements d'amélioration).

## Contraintes

- `cargo xtask check-arch` :
  - le moteur ne connaît pas le jeu, et la simulation ne dépend pas de l'affichage. Dans
    `engine_voxel` : la logique pure des transferts et des améliorations des conteneurs (entonnoir,
    compactage, néant), le comportement des coffres à paliers et du contrôleur. Dans `games/voxel` :
    les sacs, l'interface, les touches ;
  - aucun fichier `.rs` au-delà de 2 500 lignes. `games/voxel/src/main.rs` est déjà à environ
    2 470 lignes : n'y ajouter que des appels d'une ligne, et mettre le code dans de nouveaux
    modules ;
  - pas de fichier de réexport.
- Les numéros des blocs de base ne bougent jamais : nouveaux blocs **à la fin** de
  `assets/core/blocks.ron` ; mettre à jour le compte des blocs posables dans `registry.rs` si un test
  le demande.
- Ne pas changer le format de sauvegarde des chunks (postcard) : les sacs passent par leur propre
  fichier. Les anciens mondes et les anciens `settings.ron` doivent se charger.
- Commentaires et textes en français, dans le style du projet (doc de module `//!` qui explique).
- Tests unitaires :
  - Déposer, Compléter, Tout prendre, Trier et Maj + clic : empilement, usure différente, plein,
    barre d'objets épargnée ;
  - montée de palier qui garde le contenu ;
  - piles ×2 et ×4 ;
  - compactage tiré des recettes ;
  - néant avec et sans filtre ;
  - entonnoir ;
  - contrôleur (regroupement, dépôt réparti) ;
  - sacs : numéro gardé à travers sol, coffre et sauvegarde ; refus d'un sac dans un sac ;
    ramassage filtré ; réapprovisionnement ; nourriture ;
  - aller-retour de sauvegarde de `backpacks.bin`.
- Avant de rendre, tout doit être vert : `cargo fmt --all`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace --release`, `cargo xtask check-arch`,
  `cargo xtask hooks --check`.
- Essai en jeu dans un monde de test à part : `VOXEL_MUTE=1 VOXEL_CONTROL=1 VOXEL_SEED=3
  VOXEL_WORLD=storagetest`, piloté par `cargo xtask ctl`, captures dans `shots/` puis supprimées.
  - Vérifier : l'aperçu en regardant un coffre ; Déposer qui complète les piles ; Trier ; un coffre
    monté au fer avec son contenu ; les améliorations pile et compactage ; le contrôleur ; un sac
    ouvert avec B (ou `ctl backpack`) ; le ramassage et l'aimant ; le sac retrouvé après
    redémarrage.
  - Ne jamais toucher à `saves/world` ni au `settings.ron` de l'utilisateur à la racine : s'il est
    modifié par un essai, le remettre exactement comme avant. Ne rien supprimer dans `saves/`.
- Ne rien commiter, ne rien pousser.
- Ordre de travail si tout ne tient pas : 1 (aperçu, boutons, Maj + clic) d'abord, puis 2, puis 3.
  Dire clairement ce qui manque.
