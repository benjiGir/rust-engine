# Plan : un dossier de jeu installable

Aujourd'hui, le jeu ne tourne que depuis le dépôt. Tout se range par rapport à une « racine du projet »,
le parent du dossier `assets/`. Cette racine est trouvée par `VOXEL_ASSETS`, sinon par un `./assets` du
dossier courant, sinon par le chemin du dépôt **figé à la compilation** (`CARGO_MANIFEST_DIR`). Un
exécutable copié ailleurs chercherait donc ses assets dans le dépôt de la machine qui l'a compilé.

Ce que la racine contient, et qui le lit :

| Chemin | Contenu | Lu par | Écrit par |
|---|---|---|---|
| `assets/core/` | blocs, objets, textures, sons, quêtes du jeu de base | jeu, serveur | — |
| `shader_packs/` | packs de shaders (Crépuscule…), `assets/shader_packs/` avant I2 | jeu | — |
| `mods/<id>/` | mods (données, `.wasm`, et le source `script/`) | jeu, serveur | `xtask build-mods` |
| `texture_packs/<id>/` | packs de textures 32 et 64 px | jeu | assetgen |
| `saves/<monde>/` | mondes (`VOXEL_SAVES` pour ailleurs) | jeu, serveur | jeu, serveur |
| `settings.ron`, `mod_settings.ron` | réglages du joueur et des mods | jeu | jeu |
| `server.ron` | réglages du serveur dédié (`VOXEL_SERVER_CONFIG`) | serveur | serveur (au premier lancement) |

## Le but

Un dossier qu'on dézippe et qu'on lance, avec à côté des deux exécutables tout ce qu'un joueur ou un
hébergeur personnalise :

```text
Voxel/
├── voxel                    le jeu (voxel.exe sous Windows)
├── voxel-server             le serveur dédié, sans affichage
├── game/                    le jeu de base, livré, jamais modifié (l'actuel assets/core)
├── mods/                    les mods : ceux livrés et ceux qu'on ajoute (un dossier par mod)
├── texture_packs/           les packs de textures (livrés et ajoutés)
├── shader_packs/            les packs de shaders (livrés et ajoutés)
├── saves/                   les mondes
├── screenshots/             les captures (F2)
├── logs/                    le journal du dernier lancement (latest.log) et les précédents
├── settings.ron             les réglages du joueur
├── mod_settings.ron         les réglages des mods
└── server.ron               les réglages du serveur dédié (créé au premier lancement du serveur)
```

Trois règles :

1. **Le jeu de base est à part** (`game/`). Une mise à jour le remplace en entier sans toucher au reste.
2. **Ce qu'on ajoute et ce qui est livré vit au même endroit.** On dépose un mod dans `mods/`, un pack
   dans `texture_packs/` ou `shader_packs/`. Une mise à jour ne remplace que les dossiers livrés qui
   portent le même nom.
3. **Le serveur dédié lit le même dossier.** Il trouve ses mods là où le jeu les trouve, et la poignée de
   main vérifie déjà que client et serveur ont les mêmes.

## La racine, trouvée au lancement

Un seul module (`voxel_sim::paths`) remplace `project_root`, `assets_dir` et les chemins écrits en dur.
Il rend une structure `GamePaths` : `game`, `mods`, `texture_packs`, `shader_packs`, `saves`,
`screenshots`, `logs`, `settings`, `mod_settings`, `server_config`. Il cherche la racine dans cet ordre :

1. `--data-dir <dossier>` ou `VOXEL_DATA` : tout est là. Plusieurs serveurs sur une machine, des essais.
2. **Le dossier de l'exécutable**, s'il contient `game/` : c'est l'installation. Sur macOS, dans une
   application `Voxel.app`, c'est le dossier qui contient l'application.
3. **Le dépôt** (en développement, `cargo run`) : la racine actuelle, avec `assets/` à la place de
   `game/`. Rien ne change pour le développement.

`VOXEL_ASSETS`, `VOXEL_SAVES` et `VOXEL_SERVER_CONFIG` restent des surcharges ponctuelles.

**Un dossier en lecture seule.** Une installation dans un dossier protégé (Program Files, /Applications)
ne peut pas écrire `saves/` ni `settings.ron`. Dans ce cas, le jeu range ce qu'il écrit dans le dossier
utilisateur du système (`%APPDATA%\Voxel`, `~/Library/Application Support/Voxel`,
`~/.local/share/voxel`). Il y lit aussi `mods/` et les packs, en plus de ceux de l'installation, et le
journal dit où il écrit.

## Les mods, les packs et leurs doublons

- **Un mod** est un dossier qui contient `mod.ron` (comme aujourd'hui). Le `.wasm` compilé est livré ; le
  source `script/` ne l'est pas. Deux mods de même identifiant : celui du dossier utilisateur passe
  devant celui de l'installation, et le journal le signale.
- **Un pack de shaders** quitte `assets/shader_packs/` pour `shader_packs/`, à côté des packs de
  textures. Le pack « Standard » reste le shader intégré au moteur.
- **Un pack de textures** garde sa forme actuelle (`pack.ron`, `assets/core/…`).
- Une archive `.zip` déposée dans ces dossiers n'est pas lue au début : on dézippe. La lecture des
  archives pourra venir plus tard.

## Le serveur dédié

`voxel-server` lit le même dossier : `server.ron`, `saves/`, `mods/`. `--data-dir` permet plusieurs
serveurs sur une machine, chacun avec ses mondes et ses mods. Il ne lit jamais `texture_packs/` ni
`shader_packs/`, qui ne concernent que l'affichage.

## L'empaquetage

`cargo xtask package [--target <triple>]` fabrique `dist/Voxel-<version>-<système>/` :

1. il compile en release `voxel` et `voxel-server` ;
2. il lance `build-mods` (les `.wasm` à jour) ;
3. il copie `assets/core` dans `game/core`, chaque mod de `mods/` **sans** `script/` ni `target/`, les
   packs de textures et de shaders livrés ;
4. il écrit un `settings.ron` par défaut et un `LISEZMOI.txt` (lancer, ajouter un mod, héberger) ;
5. il zippe le dossier.

Le `.app` macOS (icône, signature) et un installateur Windows viendront après : le dossier zippé suffit
pour jouer et pour héberger.

## Découpage

| Lot | Contenu | Vérification |
|---|---|---|
| **I1, les chemins (réalisé)** | `voxel_sim::paths`, `GamePaths` ; tous les chemins passent par lui ; le dépôt reste la racine en développement | les 1 166 tests ; le jeu se lance comme avant ; un test de la recherche de racine (exécutable, `VOXEL_DATA`, dépôt) |
| **I2, les dossiers du joueur (réalisé)** | `shader_packs/` à la racine ; `screenshots/` et `logs/latest.log` ; doublons de mods signalés | un mod et un pack déposés à la main apparaissent |
| **I3, lecture seule (réalisé)** | repli sur le dossier utilisateur du système quand l'installation n'est pas inscriptible | un test avec un dossier en lecture seule |
| **I4, l'empaquetage** | `cargo xtask package`, puis lancer le jeu et le serveur **depuis `dist/`, hors du dépôt** | le jeu démarre, un monde se crée, le serveur dédié tourne, un invité le rejoint, tout depuis `dist/` |

## Réalisé

- **I1** : `voxel_sim::paths` cherche la racine (`--data-dir`/`VOXEL_DATA`, puis un `game/` à côté de l'exécutable
  ou du `.app`, puis le dépôt) et rend `GamePaths` (`game`, `mods`, `texture_packs`, `shader_packs`, `saves`,
  `settings`, `mod_settings`, `server_config`). `assets_dir()` et `project_root()` en sont de simples lectures ;
  les mondes, les mods, `server.ron` et les dossiers du rechargement à chaud passent par lui ; le jeu et
  `voxel-server` lisent `--data-dir` au démarrage. Six tests rejouent la recherche (installation, `.app`,
  `--data-dir`, dépôt, surcharges, argument). Vérifié : `voxel-server` copié dans un dossier avec `game/` et
  `mods/`, lancé d'ailleurs, trouve tout à côté de lui et y écrit ses mondes ; depuis le dépôt, rien ne change.
  Le jeu de base s'appelle encore `assets/` dans le dépôt et `game/` dans une installation ; les packs de
  shaders restent sous le jeu de base jusqu'à I2.
- **I2** : les packs de shaders quittent `assets/shader_packs/` pour `shader_packs/` à la racine (surveillés
  par le rechargement à chaud). F2 enregistre une capture, interface comprise, dans
  `screenshots/<date>.png` ; le frustum culling, outil de débogage, passe sur F7. Le journal s'écrit aussi
  dans `logs/latest.log` (`server-latest.log` pour `voxel-server`) ; le précédent est renommé d'après sa
  date (`voxel-<date>.log`), dix sont gardés. Les dates sont en temps universel (pas de dépendance pour
  le fuseau). Les mods sont lus par `engine_mods::scan` : un mod illisible est écarté avec sa raison au lieu
  de désactiver tous les mods, et deux mods de même identifiant gardent le premier dossier (prêt pour le
  dossier du joueur d'I3). `voxel-server` lit enfin `--data-dir` (oublié en I1).
- **I3** : quand l'installation n'est pas inscriptible (on essaie d'y écrire un fichier), la racine devient le
  dossier utilisateur du système (`Layout::UserDir`) : mondes, réglages, journal, captures et `server.ron`
  y vont. Les mods et les packs (textures et shaders) sont lus dans les deux dossiers, celui du joueur
  d'abord (`GamePaths::mods_dirs`, `texture_pack_dirs`, `shader_pack_dirs`) ; un doublon de l'installation
  est écarté et signalé. `--data-dir` ne déclenche pas le repli. Vérifié avec une installation copiée en
  lecture seule (`chmod 555`) : le jeu et `voxel-server` écrivent dans `~/Library/Application Support/Voxel`,
  les packs livrés et ceux du joueur apparaissent dans les menus, le mod du joueur passe devant le livré.

## Choix à valider

| Question | Proposition | Pourquoi |
|---|---|---|
| Où vivent les sauvegardes et les réglages | dans le dossier du jeu (portable), repli sur le dossier utilisateur s'il est en lecture seule | c'est la demande : un dossier qui contient tout ; facile à copier, à sauvegarder, à moder |
| Nom du dossier du jeu de base | `game/` | distinct des dossiers du joueur ; `assets/` reste le nom dans le dépôt |
| Packs de shaders | sortir d'`assets/` vers `shader_packs/` | même logique que les packs de textures : le joueur en ajoute |
| Archives `.zip` | pas au début | un dossier se lit déjà ; les archives demandent un système de fichiers virtuel |
