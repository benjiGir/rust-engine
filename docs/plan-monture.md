# Plan : monter à cheval (lot N9 du plan nature)

Le dernier lot du [plan nature](plan-nature.md). Les chevaux existent depuis N8 (35 robes, élevage) ;
ici on peut enfin les **apprivoiser**, leur poser une **selle**, les **monter** et les **diriger**.
Tout est original : seules les idées générales (une selle, un cheval qui accepte au bout de quelques
essais) sont communes à beaucoup de jeux ; aucun nom, texte ni image n'est repris.

## Ce que le joueur fait

1. **Apprivoiser** : clic droit à mains nues sur un cheval sauvage. Il se cabre et jette le joueur par
   terre. Chaque cheval exige entre 2 et 5 essais (tiré à sa naissance) : au dernier, il « t'accepte ».
2. **Seller** : une **selle** (`core:saddle`, fabriquée avec 3 cuirs et 2 lingots de fer) en main, clic
   droit sur le cheval apprivoisé. Sans selle, on ne le monte pas.
3. **Monter** : clic droit sur le cheval sellé. La caméra suit le cheval.
4. **Diriger** : les touches de marche disent où aller. Le cheval **tourne doucement** vers cette
   direction (il ne vire pas à angle droit sur place) et avance dans le sens où il regarde. Marcher =
   **trot** (60 % de sa vitesse), courir (la touche de course) = **galop**. L'espace le fait **sauter** ;
   une marche d'un bloc devant lui, il la monte seul.
5. **Descendre** : Maj, comme d'un wagonnet. On se retrouve à côté du cheval.

Chaque cheval a **sa vitesse de galop** (6 à 10,5 blocs par seconde, le joueur court à 5,6) et **la
hauteur de son saut** (1,3 à 2,6 blocs, contre 1,25 pour le joueur) : on cherche un bon cheval. Un
poulain tient la **moyenne de ses parents**, un peu déviée au hasard (au plus un quart de l'écart
entre le pire et le meilleur) : on élève de meilleurs chevaux en croisant les meilleurs.

Un poulain ne se monte pas. Un cheval sellé tué laisse sa selle. Un cheval apprivoisé ne disparaît plus
quand on s'éloigne (les autres si, comme tous les animaux).

## Le serveur fait foi

On réutilise le mécanisme des wagonnets (`vehicles/carts.rs`) plutôt que d'en inventer un :

- le cavalier reçoit le composant `Riding(cheval)` : la fenêtre sait qu'il est **porté** (comme dans un
  wagon : `PredictedBody::carried`), ne simule plus son corps et suit ce que le serveur lui envoie
  (`ServerMsg::Correct`) ; le serveur le replace sur la selle à chaque pas (`carry_riders`) ;
- le cheval reçoit `Ridden(cavalier)`. Le système `mobs::mounts::steer` (pas fixe, avant la physique)
  lit l'intention du cavalier (`MoveIntent`, déjà envoyée par `ClientMsg::Move`) et la traduit en
  intention du cheval, que la physique ordinaire des créatures (`mobs::physics`) applique ;
- apprivoiser, seller, monter et descendre sont décidés par le serveur (`VoxelServer::use_mount`,
  `dismount`) : un client ne peut pas se déclarer en selle. Il n'y a **aucun message réseau de plus**
  (le clic droit et Maj existent déjà) ;
- les autres joueurs voient le cheval bouger (comme tout animal) et **sa selle** : un nouveau champ
  `Look::saddled` (**protocole 6**). Le cavalier distant, lui, suit la selle (sa position est celle du
  serveur) ; on ne dessine pas encore de pose assise.

## Les données

Dans `assets/core/entities.ron`, le cheval gagne :

```ron
mount: (speed: (6.0, 10.5), jump: (1.3, 2.6), tries: (2, 5), seat: (-0.1, 1.15)),
```

et sept boîtes de matière `core:mob_horse_saddle` (siège, pommeau, troussequin, quartiers, étriers),
que le client ne dessine que si le cheval est sellé (comme les marques des robes, `tint_part`). Un mod
peut donc rendre n'importe quelle créature montable avec `mount: (...)`.

## Le code

| Fichier | Rôle |
|---|---|
| `games/voxel_sim/src/mobs/mounts.rs` | tout : `MountDef`, `Gait`, `Temper`, `Tamed`, `Saddled`, `Ridden`, `try_mount`, `steer`, `dismount`, tests |
| `games/voxel_sim/src/server/interact.rs` | `use_mount` : le clic droit (messages, selle consommée, joueur jeté) |
| `games/voxel_sim/src/vehicles/carts.rs` | `carry_riders` : le siège d'un cheval suit son cap |
| `crates/engine_voxel/src/physics.rs` | `PlayerBody::jump_speed` : un saut plus ou moins haut |
| `games/voxel_sim/src/mobs.rs` | lecture de `mount`, allure à la naissance, saut du cheval monté |
| `games/voxel_sim/src/mobs/breeding.rs` | l'allure d'un poulain |
| `tools/assetgen` | texture `mob_horse_saddle` et icône `saddle` (3 packs) |

## Limites connues

- Les créatures ne sont pas sauvegardées : un cheval apprivoisé et sellé le reste tant que la partie
  dure, pas d'une partie à l'autre. (Sauvegarder les animaux est un chantier à part.)
- La boîte du cavalier (1,8 de haut) dépasse celle du cheval : sous un plafond bas il peut traverser
  les blocs de la tête. Les wagons ont le même défaut.
- Pas de pose assise ni de rênes dessinées, pas de monte à deux, pas de nage guidée.

## Vérification

- tests de `mobs/mounts.rs` : apprivoisement par essais, selle, monte, refus (poulain, déjà pris, pas
  une monture), allure dans les bornes, poulain entre ses parents, hauteur de saut juste, direction
  et trot/galop/saut, siège qui suit le cap, descente à côté, cavalier disparu ;
- en jeu (`VOXEL_CONTROL=1`, `cargo xtask ctl`) : essai en local (voir le résumé du lot).
