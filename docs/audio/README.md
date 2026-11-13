# Passe sonore — lot 11

La banque précédente était composée de bruit filtré et d'oscillateurs, avec un seul
échantillon par action et quatre morceaux aléatoires mono à 11 025 Hz. Elle est
remplacée par **82 sons** : prises de matières, ambiances naturelles et quatre
morceaux composés par The Cynic Project. Les crédits et licences sont livrés avec
le jeu dans [`assets/core/sounds/CREDITS.md`](../../assets/core/sounds/CREDITS.md).
La sélection provient de Kenney et des pages individuelles OpenGameArt/Freesound
répertoriées dans les crédits. Les prises d'Antoine Goumain sont sous CC BY 4.0,
les autres sous CC0 1.0. Aucun son n'est extrait de Minecraft.

## Écoute de contrôle

- [Bruitages avant](11-bruitages-avant.wav) / [bruitages après](11-bruitages-apres.wav).
- [Ambiances](11-ambiances.wav).
- [Musiques](11-musiques.wav).

Le montage des bruitages dure 21 secondes, avec un gain fixe identique avant/après,
sans normalisation des extraits. Les pas s'enchaînent toutes les 0,43 seconde :
herbe à 0 s, pierre à 2 s, bois à 4 s, neige à 6 s, pose de bois/pierre à 8/9 s,
casse de pierre à 10 s, verre à 12 s, éclaboussure à 14 s, gorgée à 16 s et
enchantement à 18 s. Les nouvelles prises sont alternées ; l'ancien son de pierre
servait aussi au bois. Les ambiances se succèdent toutes les 6 secondes
(jour, nuit, pluie, vague, chouette). La planche musicale prend 7,5 secondes à partir
de la vingtième seconde de chaque morceau, dans l'ordre jour 1, jour 2, nuit, grotte.

Ces WAV sont des montages hors ligne avec **le décodeur Kira du jeu** : tous les
paquets des quatre OGG ont été décodés pour la planche musicale. Ils ne sont pas des
enregistrements d'une partie et n'incluent pas le compresseur du mélangeur.

```sh
cargo run -p voxel --example audio_review -- assets/core/sounds sfx /tmp/bruitages.wav
cargo run -p voxel --example audio_review -- assets/core/sounds ambience /tmp/ambiances.wav
cargo run -p voxel --example audio_review -- assets/core/sounds music /tmp/musiques.wav
```

## Production et lecture

`tools/assetgen/audio/bank.ron` contient les recettes. Les sources PCM/Vorbis sont
versionnées ; leurs auteurs, pages amont, licences, conversions et SHA-256 sont
consignés dans `sources.json`. La régénération ne fait aucun téléchargement et ne
demande pas FFmpeg :

```sh
cargo run -p assetgen -- --sounds-only
```

Les effets sont en PCM 16 bits à 44,1 kHz, avec suppression du continu, filtrage
des aigus les plus agressifs, retrait des silences et courts fondus. Leur niveau
vise −23 dBFS RMS, avec un plafond de −6 dBFS et une remontée limitée à 12 dB.
Les ambiances ponctuelles visent −28 dBFS RMS, les boucles −31 dBFS RMS.
Les boucles utilisent un raccord à puissance constante de 0,75 seconde, réduit
pour les petits fichiers ; les deux canaux partagent le même fondu. L'enchantement
est un motif monté à partir d'une cloche enregistrée et filtrée.

Les pas, poses et casses ont quatre prises par famille. Le tirage exclut la prise
précédente ; la variation supplémentaire de hauteur est réduite de ±8 à ±2 %.
Les blocs de bois et les machines métalliques du jeu et des mods ont leurs propres
références sonores. Les identifiants et l'ordre des blocs sont conservés. La
comparaison des RON avant/après vérifie que seuls les champs `sounds` ont changé.

Les morceaux complets sont en Vorbis stéréo 44,1 kHz, ajustés à −23 LUFS ; ils sont
**décodés progressivement**, sans charger toute la musique en mémoire. Le WAV reste
prioritaire pour les mods, avec OGG en repli. Le bus principal garde 3 dB de marge
et applique une compression douce des gros cumuls. Les boucles de nature s'effacent
sous terre et dans les biomes sans végétation. F5 recharge les prises et réarme la
musique ; l'intervalle entre morceaux ne compte plus deux fois leur durée.

Pour réimporter une source externe, `tools/assetgen/import_audio.py` décrit les
conversions de préparation avec FFmpeg. Il attend un cache local explicite,
comprenant les archives et leurs fichiers extraits indiqués par les pages amont.
Cet import initial ne fait pas partie de la régénération ordinaire ; les masters
versionnés sont la référence reproductible.

## Vérification

[`11-controle.json`](11-controle.json) donne la durée, le format, le pic et le RMS des
82 sons. Les tests couvrent les raccords, le format stéréo PCM, les niveaux et bords
des effets, le décodage, la sélection des prises, les morceaux en streaming et la
temporisation après rechargement. Les fichiers sources sont contrôlés par SHA-256.

Les archives des anciennes prises synthétiques utilisées pour l'aperçu avant ne
sont pas réintroduites dans la banque du jeu.

La tentative de lancement natif de ce lot reste bloquée avant l'ouverture de la
fenêtre par `com.apple.hiservices-xpcservice` sur macOS. Le serveur de contrôle ne
répond pas et le processus a été arrêté. L'écoute depuis la partie reste donc à
confirmer ; les quatre fichiers d'écoute ci-dessus permettent de juger la banque
sans carte son ni fenêtre de jeu.
