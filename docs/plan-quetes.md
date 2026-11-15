# Plan : le livre de quêtes

Inspiré de **FTB Quests** (FTBTeam) tel que l'utilisent les packs **All the Mods** (ATM 9, ATM 10) :
un fil conducteur qui donne les bases et montre où aller ensuite, sans prendre le joueur par la main.
On reprend les idées, pas le contenu : les quêtes d'ATM sont sous licence « tous droits réservés »
(`config/ftbquests/quests/README.md` du dépôt ATM-10). **Aucun texte, aucune image ni aucun
identifiant d'ATM ne doit être repris** : nos quêtes sont écrites pour notre jeu.

## Ce que fait FTB Quests (résumé de l'étude)

Sources :
- [FTB-Quests](https://github.com/FTBTeam/FTB-Quests) : le code, notamment `quest/task/*`,
  `quest/reward/*`, `quest/DependencyRequirement.java`, `quest/ProgressionMode.java` et
  `quest/reward/RewardAutoClaim.java` ;
- les fichiers de quêtes d'[ATM-10](https://github.com/AllTheMods/ATM-10/tree/main/config/ftbquests/quests) :
  `data.snbt`, `chapter_groups.snbt`, `chapters/*.snbt` et `lang/fr_fr`.

La documentation en ligne de FTB (docs.feed-the-beast.com) est rendue en JavaScript et n'a pas pu
être lue : ce qui suit vient du code et des fichiers d'ATM.

- **Hiérarchie** : un fichier global (`data.snbt` : mode de progression par défaut, forme par
  défaut, délai de détection...), des **groupes de chapitres** (« Quêtes principales », « Magie »,
  « Stockage », « Technologie »...), des **chapitres** (un fichier chacun), et dans un chapitre des
  **quêtes**. Une quête a des **tâches** et des **récompenses**. Tout a un identifiant, et les textes
  sont à part, par langue (`lang/fr_fr/chapters/<chapitre>.snbt` : `title`, `quest_desc`).
- **Une quête sur le plan** : position `x, y` sur une grille (des demi-cases), une **forme**
  (cercle, carré, carré arrondi, hexagone, pentagone, losange...), une **taille** (1 par défaut, 2 ou
  3 pour les étapes marquantes) et une **icône** (un objet, par défaut celui de la première tâche).
- **Dépendances** : une liste de quêtes, avec un mode `all_completed` (défaut), `one_completed`,
  `all_started` ou `one_started`. Options : `optional` (ne compte pas dans le pourcentage du
  chapitre), `invisible`, `hide_until_deps_visible`, masquer les traits de dépendance. Les traits
  sont dessinés de la dépendance vers la quête : c'est l'arbre qui montre la suite.
- **Mode de progression** : `linear` (une tâche n'avance pas tant que les dépendances ne sont pas
  faites) ou `flexible` (les tâches avancent toujours, la quête se termine quand les dépendances le
  sont). ATM est en `flexible` partout.
- **Tâches** : objet (de loin la plus utilisée chez ATM : 329 tâches d'objet sur les 5 fichiers lus),
  case à cocher (lire et valider), tuer, dimension, biome, structure, lieu, observation (regarder un
  bloc ou une créature), statistique, expérience, énergie, fluide, avancement, étape, et une tâche
  personnalisée qu'un autre mod remplit. Une tâche d'objet compte ce que le joueur a sur lui (scan de
  l'inventaire) ; options : consommer les objets (`consume_items`, désactivé par défaut chez ATM),
  seulement par fabrication (`only_from_crafting`), filtre par groupe d'objets (les « tags »).
- **Récompenses** : objet, points d'expérience, niveaux, table de butin ou tirage au sort
  (`random`, `loot`), choix entre plusieurs objets, commande, notification. Chaque quête finie
  attend qu'on **récupère** ses récompenses (bouton, ou « tout récupérer ») ; l'auto-récupération
  existe (`default`, `disabled`, `enabled`, `no_toast`, `invisible`). ATM la désactive.
- **Écran** : il prend tout l'écran. À gauche, la liste des chapitres rangés par groupe, avec une
  icône et le pourcentage fait ; au centre, le plan du chapitre, qu'on fait glisser et qu'on zoome ;
  un clic sur une quête ouvre sa fiche (texte, tâches, récompenses, bouton « Récupérer »). Les
  quêtes ont une couleur ou un contour selon leur état : verrouillée, disponible, en cours, finie,
  récompense en attente. On peut **épingler** une quête pour suivre ses tâches à l'écran.
- **Notifications** : une bannière quand une quête est finie, et un compteur des récompenses à
  récupérer.
- **Équipes** (FTB Teams) : la progression est partagée par équipe. Nous n'avons pas de
  multijoueur : on garde un seul joueur.

**Ce qu'ATM en fait** (67 fichiers de chapitres dans ATM-10) :
- un chapitre « Bienvenue » (premières notions, case à cocher) ;
- une **ligne principale** en plusieurs parties (bois → établi → outils → four → métaux → Nether →
  ...) qui mène à l'objectif final du pack ;
- **un chapitre par mod** (Create, Mekanism, AE2, Ars Nouveau...) rangé dans un groupe thématique ;
- quelques chapitres de conseils (« trucs et astuces », « construction »).

Le ton est léger, les descriptions courtes (deux à quatre paragraphes), et chaque quête demande
**d'avoir** quelque chose plutôt que de suivre des étapes : on sait quoi viser, pas comment le
faire. C'est ce qui guide sans prendre par la main, et c'est l'esprit à garder.

## Ce qu'on retient pour notre jeu

| FTB Quests | Chez nous |
|---|---|
| Fichiers SNBT, identifiants hexadécimaux, textes à part par langue | Un `quests.ron` dans `assets/core/` et dans chaque mod. Identifiants lisibles (`core:wood/first_log`). Textes en français directement dans le fichier, comme `guide.ron`. |
| Groupes de chapitres | Oui (`groups`) : « Les bases », « Aventure », « Mods »... Un mod peut ajouter un groupe ou se ranger dans un groupe existant. |
| Plan avec x/y, formes, tailles | Oui. Formes : `Circle`, `Square`, `RoundSquare`, `Hexagon`, `Diamond`, `Pentagon`. |
| Dépendances all / one, optionnelle, invisible, cachée | Oui : `requires: All` ou `One` (« commencée » : hors périmètre), `optional`, `hidden` (cachée tant que ses dépendances ne sont pas finies), `secret` (jamais montrée avant d'être finie). |
| linear / flexible | Oui, au niveau du fichier, du chapitre ou de la quête. `Flexible` par défaut. |
| Tâches | Voir « Tâches » plus bas. Pas d'énergie, de fluide ni d'avancement dans une première version. |
| Récompenses | Objets, points d'expérience, tirage dans une table de `loot.ron`, choix d'un objet parmi plusieurs. Pas de commande. |
| Récupérer à la main, ou automatiquement | Oui (`autoclaim` par quête, désactivé par défaut). |
| Épingler | Oui : les quêtes épinglées s'affichent à l'écran (phase 2). |
| Équipes | Non : un joueur, une progression par monde. |
| Éditeur en jeu | Non : on écrit les fichiers RON. Le rechargement à chaud des données (déjà là pour les assets) suffit pour mettre en place un plan. |

## Ce qui existe déjà dans le jeu

- **Événements** (`games/voxel/src/events.rs`) : `BlockBroken`, `BlockPlaced`, `EntityDied` et
  `PlayerDamaged` sont publiés dans l'ECS. Le système de quêtes peut les lire avec son propre
  lecteur, **sans toucher au code qui les émet**.
- **Progression d'aventure** (`tiers.rs::Progress`) : créatures tuées, boss, meilleure rareté,
  profondeur, palier débloqué. Une tâche peut lire ces compteurs.
- **Dimensions** (`dimensions.rs::travel`), **biome courant** (affiché dans l'état `ctl`),
  **structures** (`ctl structures` les localise : voir comment, pour une tâche « trouver une
  structure »), **expérience** (`survival.rs::grant_xp`, niveaux).
- **Inventaire** (`inventory.rs`) et sacs à dos (`backpacks.rs`) : le scan d'une tâche d'objet compte
  l'inventaire du joueur (pas le contenu des sacs, comme FTB).
- **Artisanat** : `Inventory::take_craft_result` (pour l'option « seulement par fabrication »).
- **Explorateur de recettes** (`recipe_browser.rs`) : un clic sur l'objet d'une tâche ouvre ses
  recettes (`self.ui.browser.open(item, Lookup::Recipes)`), comme dans le Guide du voyageur.
- **Guide du voyageur** (`guide.rs`, `guide_ui.rs`, `assets/core/guide.ron`) : le livre de
  référence. Une quête peut renvoyer vers un chapitre du guide (bouton « Lire dans le guide »).
  `guide::expand_keys` (`{touche:inventory}`) sert aussi aux textes des quêtes.
- **Écrans** : `ui.rs::Screen`, fenêtre d'inventaire `ui_layout::inventory_window`. Le livre de
  quêtes, lui, prend **tout l'écran** : il lui faut sa propre fenêtre (comme le menu pause ou les
  options), pas la fenêtre d'inventaire.
- **Touches** : `settings.rs::Keys` et `Action`. La touche **L** est libre. Un ancien `settings.ron`
  sans la nouvelle touche doit prendre la touche par défaut (`#[serde(default)]`, comme `tiers`).
- **Sauvegarde du monde** : `world.ron` (`persistence.rs`), et des fichiers à côté (`backpacks.bin`,
  `items.bin`). La progression des quêtes va dans **`quests.ron`** à la racine du monde (partagé par
  les dimensions, comme `backpacks.bin`).
- **Notifications** : `self.toast(...)` et `self.bell()`.
- **Mods** : chaque mod a son dossier (`mods/<nom>/`) et ses fichiers RON lus par
  `content.rs::sources`. Les mods existants : `forge`, `magie`, `mecanique`, `entrepot`, `donjons`
  (`example` est un mod de démonstration : pas de chapitre).

## Le format des données

`assets/core/quests.ron`, puis le `quests.ron` de chaque mod, lus dans l'ordre de chargement.
Exemple (le format exact est à fixer par l'agent, dans cet esprit) :

```ron
(
    // Seulement dans assets/core : les réglages par défaut.
    settings: (progression: Flexible, shape: Circle),
    groups: [
        (id: "core:basics", title: "Les bases", icon: "workbench"),
        (id: "core:adventure", title: "Aventure", icon: "iron_sword"),
        (id: "core:mods", title: "Mods", icon: "guide"),
    ],
    chapters: [
        (
            id: "core:wood",
            group: "core:basics",
            title: "Le bois et l'établi",
            icon: "oak_log",
            // Facultatif : Linear ou Flexible pour tout le chapitre.
            progression: Flexible,
            quests: [
                (
                    id: "core:wood/first_log",
                    title: "Un arbre, enfin",
                    // Paragraphes séparés par une ligne vide. `{touche:inventory}` est remplacé.
                    text: "Tout commence par une bûche. Frappe un tronc en gardant le clic gauche enfoncé.",
                    x: 0.0, y: 0.0, shape: Hexagon, size: 2.0,
                    tasks: [Item(item: "#logs", count: 1)],
                    rewards: [Item(item: "torch", count: 4), Xp(10)],
                ),
                (
                    id: "core:wood/workbench",
                    title: "Un plan de travail",
                    text: "...",
                    x: 2.0, y: 0.0,
                    after: ["core:wood/first_log"],
                    tasks: [Item(item: "workbench")],
                    rewards: [Xp(10)],
                    guide: "core:wood_and_crafting",
                ),
            ],
        ),
    ],
)
```

- **Identifiants** : `espace:nom` comme ailleurs. Un mod n'ajoute que des chapitres et des groupes
  de son espace de noms, mais il peut ranger un chapitre dans un groupe d'un autre espace
  (`group: "core:mods"`), et une quête peut dépendre d'une quête d'un autre espace
  (`after: ["core:iron/ingot"]`).
- **Objets** : sans espace de noms, celui du fichier (`stick` → `core:stick`). `#groupe` désigne un
  groupe d'objets des `items.ron` (`#logs`, `#planks`).
- **Quête** : `id`, `title`, `subtitle` (facultatif), `text`, `x`, `y`, `shape`, `size`, `icon`
  (facultatif : par défaut l'objet de la première tâche), `after` (dépendances), `requires`
  (`All` par défaut ou `One`), `optional`, `hidden`, `secret`, `progression`, `tasks`, `rewards`,
  `autoclaim`, `guide` (un chapitre du guide).
- **Vérifications au chargement** (une erreur claire, comme pour `guide.ron`) :
  - objets, groupes, créatures, dimensions, biomes, structures, tables de butin et chapitres du
    guide inconnus ;
  - identifiants en double, mauvais espace de noms ;
  - dépendance inconnue, **cycle de dépendances** ;
  - chapitre vide, quête sans tâche.
  Un mod désactivé : ses chapitres disparaissent, et une quête d'un autre mod qui en dépendait voit
  la dépendance ignorée (avec un avertissement dans le journal), pas une erreur.

### Tâches

| Tâche | Fini quand | Détection |
|---|---|---|
| `Item(item, count, consume: false, crafted: false)` | le joueur **a** `count` exemplaires sur lui (`#groupe` accepté) | scan de l'inventaire, une fois par seconde et à chaque ouverture du livre. `consume: true` : un bouton « Remettre » prend les objets. `crafted: true` : ne compte que ce qui sort de la grille d'artisanat. |
| `Check` | le joueur coche la case dans la fiche | clic |
| `Break(block, count)` | `count` blocs cassés | événement `BlockBroken` |
| `Place(block, count)` | `count` blocs posés | événement `BlockPlaced` |
| `Kill(creature, count)` | `count` créatures tuées (`creature` facultatif : toutes les hostiles) | événement `EntityDied` (vérifier que la mort vient du joueur) |
| `Dimension(id)` | le joueur entre dans cette dimension | après `travel` |
| `Biome(id)` | le joueur se trouve dans ce biome | une fois par seconde |
| `Structure(id)` | le joueur entre dans une structure de ce type | une fois par seconde (s'appuyer sur ce qu'utilise `ctl structures`) |
| `Depth(y)` | le joueur descend sous cette hauteur | une fois par seconde |
| `Level(n)` | le joueur atteint ce niveau d'expérience | une fois par seconde |
| `Tier(n)` | ce palier du monde est débloqué | `tiers.rs::Progress` |
| `Signal(id)` | un mod l'envoie (phase 4) | nouvel appel de l'API des mods |

Le compteur d'une tâche (blocs, créatures) n'avance que si la quête est **disponible** en mode
`Linear`, et toujours en mode `Flexible`.

### Récompenses

| Récompense | Effet |
|---|---|
| `Item(item, count)` | donné au joueur ; ce qui ne tient pas tombe à ses pieds |
| `Xp(points)` | `grant_xp` |
| `Loot(table)` | un tirage dans une table de `loot.ron` |
| `Choice([...objets])` | le joueur choisit **un** objet dans la fiche |

Une récompense n'est donnée **qu'une fois** : la sauvegarde retient les récompenses récupérées.

## La progression et sa sauvegarde

- **États d'une quête** : `Locked` (dépendances non finies), `Available` (rien de fait), `Started`
  (au moins une tâche avance), `Done` (tâches finies, récompenses à récupérer), `Claimed` (tout est
  récupéré). Une quête sans récompense passe directement à `Claimed`.
- **Le cœur de la logique est pur** (sans ECS ni affichage) : `quests.rs` (données, vérifications),
  `quest_progress.rs` (états, compteurs, mise à jour à partir d'un « instantané » du joueur et d'une
  liste d'événements) : tout est testable en unitaire.
- **`quests.ron` dans le monde** : quêtes finies, compteurs des tâches, cases cochées, récompenses
  récupérées, quêtes épinglées, dernier chapitre ouvert. Les identifiants inconnus (mod retiré) sont
  **conservés** tels quels, pour retrouver la progression si le mod revient.
- Un ancien monde n'a pas de `quests.ron` : tout part de zéro, et les tâches d'objet se valident au
  premier scan (le joueur qui a déjà un établi voit la quête finie tout de suite, c'est voulu).

## L'écran

Une fenêtre **plein écran** (au-dessus du jeu, qui continue de tourner comme avec l'inventaire),
ouverte par la touche **L** (réglable dans les options) et par un bouton « Quêtes » dans
l'écran d'inventaire. Échap la ferme.

- **Colonne de gauche** : les groupes, et sous chacun ses chapitres (icône, titre, `n / total` ou
  pourcentage, une pastille si des récompenses attendent). Un groupe se replie.
- **Centre : le plan du chapitre** :
  - les quêtes à leur `x, y` (une case de grille = environ 40 px au zoom 1), dessinées dans leur
    forme avec leur icône d'objet (`self.ui.icon`, comme les cases) ;
  - des **traits** de chaque dépendance vers la quête, avec une flèche ou un dégradé ; couleur selon
    l'état (gris : verrouillé, clair : disponible, vert : fini) ;
  - les états : verrouillée (assombrie, cadenas si `show_lock`), disponible (contour clair), en cours
    (contour doré), finie avec récompense en attente (pastille qui pulse), récupérée (vert) ;
  - les quêtes `hidden` ne sont ni dessinées ni reliées tant qu'elles sont verrouillées ; les
    `secret`, jamais avant d'être finies ;
  - glisser avec le clic gauche (ou le clic du milieu) pour se déplacer, molette pour zoomer, un
    bouton pour recentrer ; le plan se cadre tout seul à l'ouverture d'un chapitre ;
  - une infobulle au survol : titre, sous-titre, état.
- **Droite : la fiche de la quête choisie** (panneau qui s'ouvre au clic) :
  - titre, sous-titre, texte (avec `{touche:...}` remplacé) ;
  - les **tâches**, chacune avec son icône, sa progression (`3 / 16`) et une coche ; un clic sur
    l'objet d'une tâche ouvre ses recettes dans l'explorateur ;
  - les **récompenses**, et les boutons « Récupérer » (et « Remettre » pour une tâche qui consomme) ;
  - « Épingler », et « Lire dans le guide » si la quête a un `guide` ;
  - les dépendances manquantes, cliquables (le plan saute à la quête, même dans un autre chapitre).
- **En haut** : le titre du chapitre et sa progression, et « Tout récupérer ».
- **Le jeu, hors du livre** :
  - une bannière (toast) « Quête terminée : ... » avec le son de la cloche ;
  - les **quêtes épinglées** (au plus 3) en haut à droite de l'écran : titre et tâches restantes ;
  - un petit indicateur dans l'écran d'inventaire quand des récompenses attendent.

L'interface suit le style du jeu (`skin.rs`, `ui_layout.rs`), et doit rester lisible et
utilisable en 1280 × 720.

## Le contenu

Un fil conducteur **qui dit quoi viser, pas comment** : chaque quête demande d'avoir ou de faire
quelque chose, avec un court texte (deux à quatre phrases) qui donne une piste, et renvoie au guide
pour le détail. Les récompenses sont modestes (quelques torches, de la nourriture, de l'expérience,
parfois un tirage) : elles aident sans remplacer le jeu.

**Toutes les recettes, valeurs et noms cités doivent être vérifiés dans les fichiers du jeu**
(`recipes.ron`, `smelting.ron`, `brewing.ron`, `ores.ron`, `items.ron`, `blocks.ron`, `entities.ron`,
`structures.ron`, `dimensions.ron` des mods...). Un test vérifie déjà que chaque objet existe ; le
texte, lui, doit être relu contre les données.

Groupe **« Les bases »** (ligne principale) :
1. **Premiers pas** : une bûche, des planches, l'établi, une case à cocher « lire le guide ».
2. **Les outils** : outils en bois, puis en pierre (moellons), la première pioche en pierre.
3. **Le feu et le four** : charbon, torches, four, verre, premier lingot.
4. **L'âge du métal** : cuivre, fer, armure de fer, seau, cisailles.
5. **Se nourrir et s'installer** : houe, champ irrigué, pain, lit, coffre, sac à dos.
6. **Les profondeurs** : descendre sous la hauteur 20, or, cinabre, saphir, pioche en saphir.

Groupe **« Aventure »** :
7. **Le combat** : tuer des hostiles, armure complète, premier objet rare, palier 2 du monde.
8. **L'enchantement** : table, étagères, livre, bibliothèque, charme.
9. **L'alchimie** : fioles, alambic, chaque potion.
10. **L'exploration** : trouver un village, une ruine, une mine, une crypte ; jade dans les montagnes.

Groupe **« Machines »** : circuits (fil, levier, répéteur, piston), réseaux (câbles, tuyaux,
panneau solaire, batterie), rails et wagons, stockage (coffres à paliers, contrôleur).

Groupe **« Mods »** : **un chapitre par mod**, dans le `quests.ron` du mod : `forge`, `magie`,
`mecanique`, `entrepot`, `donjons`. Lire le mod (son `mod.ron`, ses `items.ron`, `recipes.ron`,
`machine_recipes.ron`, son script) pour écrire un chemin court qui mène à ce qui fait son intérêt.
Le premier pas d'un chapitre de mod dépend d'une quête de la ligne principale (par exemple : la forge
après le premier lingot de fer), pour que le joueur le découvre au bon moment.

Le Guide du voyageur reste le livre de référence. Le livre de quêtes est le fil à suivre.

## Les phases

Une phase à la fois, chacune verte et vérifiée avant la suivante, avec un compte rendu.

1. **Données et progression** :
   - `quests.rs` (format, chargement core et mods, vérifications), `quest_progress.rs` (états,
     compteurs, récompenses), sauvegarde `quests.ron` ;
   - détection : scan d'inventaire une fois par seconde, lecture des événements, dimension, biome,
     profondeur, niveau, palier ;
   - toasts, récupération des récompenses ;
   - commandes `ctl` (voir plus bas) ;
   - un premier chapitre de test (« Premiers pas ») pour essayer.
2. **L'écran** : la fenêtre plein écran (chapitres, plan, fiche), la touche L, le bouton dans
   l'inventaire, les quêtes épinglées à l'écran.
3. **Le contenu** : tous les chapitres de base et un chapitre par mod.
4. **API des mods** : une tâche `Signal` qu'un script remplit par un nouvel appel
   (`quest_signal(id)`), et un événement `QuestCompleted` pour les scripts. Respecter le processus
   de l'API (`cargo xtask hooks`, `docs/modding.md`). Un mod d'exemple s'en sert.

## Ce qui est fait

### Phase 1 : données et progression (faite)

- **Données** (`games/voxel/src/quests.rs`) : le format de `quests.ron` (core puis mods), ses
  vérifications (objets, groupes d'objets, blocs, créatures, biomes, dimensions, structures, paliers,
  tables de tirage, chapitres du guide ; doublons, espaces de noms ; dépendance inconnue ou en boucle ;
  chapitre vide, quête sans tâche), l'héritage `Flexible`/`Linear` (fichier, chapitre, quête). Le format
  est décrit dans `docs/modding.md`.
- **Progression** (`quest_progress.rs`) : états `Locked`/`Available`/`Started`/`Done`/`Claimed`,
  compteurs de chaque tâche (qui ne redescendent jamais), `All`/`One`, `optional`, `hidden`, `secret`,
  tâches d'objet (groupe, `consume`, `crafted`), événements, récompenses données une seule fois, choix,
  épingles (trois au plus), sauvegarde `quests.ron` à la racine du monde (identifiants inconnus conservés,
  fichier illisible mis de côté en `quests.ron.illisible`). Tout est testé sans ECS ni egui.
- **Branchement** (`quest_play.rs`) : chargement au premier pas du monde ; événements lus à chaque pas
  (`BlockBroken`, `BlockPlaced`, `EntityDied`) plus les objets fabriqués (grille et livre de recettes) ;
  instantané du joueur une fois par seconde (objets portés, dimension, biome, hauteur, niveau, palier,
  structure) ; bannière « Quête terminée : ... » avec la cloche ; récompenses récupérées à la main (ou
  tout de suite pour `autoclaim`). Passer une dimension recharge le monde : la progression est relue.
- **Touche L** (`Action::Quests`, ancien `settings.ron` : valeur par défaut) : en attendant l'écran de la
  phase 2, elle affiche un résumé et récupère les récompenses en attente (sauf les choix).
- **Commandes `ctl`** (`quest_ctl.rs`) : `quests [chapitre|quête]`, `quest done|claim|check|handin|choose|
  scan|reset|pin`, `questpin id`.
- **Contenu** : un chapitre « Premiers pas » (8 quêtes) dans le groupe « Les bases ». Chaque recette citée
  est celle de `recipes.ron` (établi : 4 planches en carré ; bûche : 4 planches ; pioche en bois :
  `MMM`/` S `/` S ` ; torche : charbon ou charbon de bois + bâton, 4 torches ; pierre : pioche de niveau 1
  pour les moellons).

**Écarts au plan, et limites** :
- Les tables du `loot.ron` du jeu sont par **bloc** (ce qu'un bloc cassé lâche) : elles ne conviennent pas
  à une récompense. `Loot("table")` désigne donc une table du `loot` du fichier `quests.ron` lui-même
  (`rolls`, `entries` avec `item`, `count`, `weight`).
- `Break` et `Place` comptent tout bloc cassé ou posé (le jeu n'indique pas qui l'a fait : un
  piston ou une machine compte aussi). `Kill` compte les créatures mortes à moins de 16 blocs du joueur
  (comme l'expérience et le palier).
- `Structure` : le joueur doit être à moins du rayon de la structure (horizontalement, sans regarder la
  hauteur : une mine ou un donjon enterré compte dès qu'on est au-dessus).
- `Depth(y)` est une hauteur absolue (`y` du monde), pas une profondeur sous la mer.
- `Signal` n'existe pas encore (phase 4).
- Non essayé en jeu : `crafted`, `consume` (testés en unitaire), `Choice` (idem, et `quest choose`),
  l'ouverture des recettes d'un clic et « Lire dans le guide » (phase 2).

## Hors périmètre (à décider plus tard)

- Éditeur de quêtes en jeu.
- Équipes et multijoueur.
- Tâches d'énergie, de fluide, d'observation ; récompenses par commande.
- Images décoratives sur le plan (ATM en met beaucoup : logos, captures).
- Boutique de quêtes, monnaie de quêtes.
- Traductions.

## Contraintes pour l'agent

- **Une phase à la fois**, dans l'ordre, chacune verte et vérifiée en jeu avant la suivante, avec un
  compte rendu par phase.
- **Ne rien reprendre d'ATM** (textes, images, identifiants) : licence « tous droits réservés ».
- `cargo xtask check-arch` :
  - le moteur ne dépend pas du jeu, et la simulation ne dépend pas de l'affichage ;
  - aucun fichier `.rs` au-delà de 2 500 lignes. `games/voxel/src/main.rs` en a déjà environ 2 445 :
    du code dans de nouveaux modules, et dans `main.rs` **seulement des appels d'une ligne** (dix
    lignes au plus en tout, déclarations de modules comprises) ;
  - pas de fichier de réexport.
- La logique (chargement, vérifications, états, compteurs, récompenses, sauvegarde) ne dépend ni de
  l'ECS ni d'egui : elle se teste seule.
- Les anciens mondes (sans `quests.ron`) et les anciens `settings.ron` (sans la touche des quêtes)
  restent valides, tests à l'appui.
- Commentaires et textes en français, dans le style du projet. Pas de ressource externe : une icône
  ou une texture nouvelle se génère avec `tools/assetgen`.
- Tests unitaires, au moins :
  - chargement et aller-retour des données, erreurs (objet inconnu, doublon, espace de noms, cycle) ;
  - dépendances `All` et `One`, `optional`, `hidden`, `secret` ;
  - modes `Linear` et `Flexible` ;
  - tâche d'objet avec groupe, `consume`, `crafted` ;
  - compteurs d'événements ;
  - récompenses données une seule fois, choix d'une récompense ;
  - sauvegarde : aller-retour, identifiants inconnus conservés, monde sans `quests.ron` ;
  - chaque objet, bloc, créature cité dans les `quests.ron` livrés existe (core et mods).
- Commandes `ctl` d'essai, par exemple : `quests` (états et compteurs), `quest done id`,
  `quest claim id`, `quest reset [id]`, `screen quests [chapitre]`, `questpin id`.
- Vérifications : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --release`, `cargo xtask check-arch`, `cargo xtask hooks --check`.
- Essai en jeu dans un monde à part (`VOXEL_MUTE=1 VOXEL_CONTROL=1 VOXEL_SEED=3
  VOXEL_WORLD=questtest`), piloté par `cargo xtask ctl`, en **mode vol** (les créatures tuent
  vite en mode marche), avec des captures du livre ouvert, d'un plan avec des traits de dépendance
  et d'une fiche de quête, en 1280 × 720.
  - Ne jamais toucher à `saves/world`. Copier le `settings.ron` de l'utilisateur avant les essais et
    le remettre à l'identique après. Ne rien supprimer dans `saves/`.
- Ne rien commiter, ne rien pousser.
