# Plan : un système de wagonnets complet (une phase)

Inspiré de Minecraft et de ses mods de rails, sans les copier :

- **Minecraft, l'expérience « Minecart Improvements »** (1.21.2) : vitesse maximale réglable
  (8 blocs/s par défaut, jusqu'à bien plus), plus de recentrage brutal sur le rail, collision
  propre dans les pentes, mouvement interpolé.
- **Railcraft** : rails spécialisés (accélérateur, détecteur, butoir, aiguillage), wagons
  attelés en trains, chargeurs et déchargeurs pour automatiser le transport.
- **Steve's Carts** : wagons de travail (stockage, collecte), locomotive.
- **Era of Rails** : trains attelés, aiguillages commandés par signal pour des réseaux automatiques.
- **Create: Steam 'n' Rails** : trains et signalisation (hors périmètre de cette phase : on garde
  des wagons sur rails de blocs, pas des trains en sous-grilles).

## Périmètre de la phase

1. **Rails** (blocs, côté moteur `crates/engine_voxel`)
   - Six sortes, décrites en données dans `blocks.ron` par un champ `rail: Normal | Booster |
     Detector | Activator | Stop` (un mod peut donc ajouter les siens) : rail, rail
     accélérateur, rail détecteur, rail activateur, butoir. L'aiguillage n'est pas un bloc à
     part : un virage en T bascule quand il reçoit du signal (comme dans Minecraft).
   - Géométrie : dix formes (deux droites, quatre virages, quatre montées), calculées depuis
     les rails voisins à la pose et au changement d'un voisin (s'inspirer de
     `connections.rs`), rangées dans `BlockEntity::data` (pas de variantes de blocs à
     sauvegarder). Un module pur et testé : forme → chemin (segment ou arc) dans le bloc,
     position le long du chemin, tangente, sortie vers le bloc suivant.
   - Rendu : une nouvelle forme `Shape::Rail` dans le mailleur (`mesh.rs`) : un quad texturé
     au ras du sol, tourné selon la forme, incliné pour une montée ; collision et sélection
     comme une dalle très fine.
   - Circuits (`circuits.rs`, `machines.rs`) : l'accélérateur et l'activateur lisent
     `received_power` ; le détecteur émet 15 tant qu'un wagon est dessus (comme le bouton), et
     prévient ses voisins à l'arrivée et au départ.

2. **Wagons** (entités ECS, côté jeu `games/voxel`, nouveau module `carts.rs` + physique pure
   dans le moteur)
   - Quatre sortes : **wagonnet** (on y monte), **wagon coffre** (27 cases), **wagon trémie**
     (ramasse les objets au sol sur son passage et vide le conteneur au-dessus du rail),
     **locomotive** (brûle du charbon ou du bois et pousse le train).
   - Physique sur rail : une vitesse le long du rail ; gravité dans les pentes, frottement,
     accélérateur alimenté (pousse, ou freine s'il n'est pas alimenté), butoir (arrêt), vitesse
     maximale réglable dans les options (8 blocs/s par défaut, 30 au plus). Pas de saut de
     rail à grande vitesse : sous-pas de 0,5 bloc au plus. Hors rail : la physique normale
     d'une boîte (`PlayerBody`), avec un fort frottement.
   - Wagons qui se touchent : ils se repoussent. **Attelage** (nouvel objet, clic sur un
     wagon puis un autre) : les wagons attelés gardent leur écart et forment un train ;
     la locomotive tire tout le train.
   - Monter et descendre : clic droit sur un wagonnet pour s'asseoir (composant `Riding`,
     la caméra suit le wagon), Maj pour descendre ; avancer donne une petite poussée. Une
     créature qui entre dans un wagonnet vide y monte. Un rail activateur alimenté fait
     descendre le passager et coupe la trémie.
   - Poser : clic droit avec un wagon sur un rail. Casser : coups (clic gauche) ; le wagon
     lâche son objet et son contenu.
   - Interface : clic droit sur un wagon coffre ou trémie ouvre son inventaire, comme un coffre.
   - Dessin : des boîtes, comme les créatures (`mobs::instances`) ; le wagon s'incline dans les
     pentes et tourne dans les virages ; position interpolée entre deux pas.

3. **Automatisation**
   - **Chargeur** et **déchargeur** (blocs avec la capacité `Inventory`, branchables aux
     tuyaux d'objets) : un wagon coffre ou trémie arrêté sur le rail voisin est rempli ou vidé ;
     le bloc émet un signal quand il a fini (pour relancer un accélérateur).

4. **Contenu, sauvegarde, outils**
   - Recettes (`recipes.ron`), objets (`items.ron`), textures et icônes générées par
     `tools/assetgen` (aucune ressource externe). L'explorateur de recettes les montre tout
     seul.
   - Sauvegarde des wagons par dimension (sorte, position, vitesse, inventaire, combustible,
     attelages), à côté des structures mobiles (`persistence.rs`, `save_grids`). Un wagon dans
     un chunk pas encore chargé attend, figé (comme les créatures).
   - Commandes `ctl` : `carts` (liste), `cart <sorte> x y z` (poser), `ride`, `dismount`, pour
     les essais sans clavier.
   - Documentation : `README.md` (commandes) et `docs/architecture.md` (où vit quoi).

## Contraintes

- Le moteur ne connaît pas le jeu, et la simulation ne dépend pas de l'affichage
  (`cargo xtask check-arch`) : rails, géométrie et physique sur rail dans `engine_voxel` ;
  ECS, interface et dessin dans `games/voxel`. Aucun fichier `.rs` au-delà de 2 500 lignes.
- Les numéros des blocs de base ne bougent jamais : les nouveaux blocs vont **à la fin** de
  `assets/core/blocks.ron`.
- Commentaires et textes en français, dans le style du projet.
- Tests unitaires pour la géométrie des rails, la physique (droite, pente, virage,
  accélérateur, butoir, vitesse max), l'attelage, la sauvegarde aller-retour.
- Avant de rendre : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace --release`, `cargo xtask check-arch`.
- Essai en jeu dans un monde de test à part : `VOXEL_MUTE=1 VOXEL_CONTROL=1 VOXEL_SEED=3
  VOXEL_WORLD=carttest`, piloté par `cargo xtask ctl`, captures dans `shots/` puis supprimées.
  Ne jamais toucher à `saves/world` ni au `settings.ron` de l'utilisateur.
- Ne rien commiter.
