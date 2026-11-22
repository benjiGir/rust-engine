# Packs de shaders

Depuis le menu principal, ouvrir **Packs de shaders…**, sélectionner un pack
puis **Appliquer**. Le décor du menu montre immédiatement l'effet et le jeu
mémorise le choix. **Standard** restaure le rendu intégré. La préférence est
locale au joueur et se combine avec son pack de textures.

**Crépuscule**, livré avec le jeu, combine les ombres portées du relief, une
brume plus dense dans les vallées, les rayons dans les ouvertures du feuillage
et l'éclairage directionnel de nuit.

## Créer un pack

Placer un dossier dans `shader_packs/`, à la racine du jeu (à côté de
`texture_packs/`). Son nom sert d'identifiant et accepte les lettres minuscules
ASCII, les chiffres, `-` et `_`. Créer
`pack.ron` à la racine de ce dossier.

```ron
(
    api: 1,
    name: "Mon pack",
    description: "Couleurs douces et brume légère.",
    sun_shadows: true,
    sun_rays: true,
    moonlight: true,
    terrain: Some("terrain.wgsl"),
    sky: Some("sky.wgsl"),
)
```

La description est facultative. Le pack fournit au moins un effet ou active
un effet de lumière. Chaque
fichier WGSL est placé à côté du manifeste. Une passe omise utilise son effet
standard. Le moteur compose les fichiers avec ses shaders puis valide les
pipelines avant de les activer. Une erreur à l'application apparaît dans le
menu. Le dernier pack valide reste actif pendant la correction. **F5** et la
surveillance des fichiers rechargent les effets du pack sélectionné.

### Terrain et eau

Le fichier de terrain définit cette fonction. Tous les RGB sont linéaires, les
positions et distances sont en blocs et le temps est en secondes.

```wgsl
fn voxel_terrain(input: TerrainEffect) -> vec4<f32> {
    let softened = input.lit * vec3(1.03, 1.0, 0.96);
    return vec4(mix(softened, input.fog_color, input.fog), input.alpha);
}
```

- `lit`, `alpha` sont la couleur éclairée et l'opacité calculées par le moteur.
- `position`, `normal`, `distance` décrivent la surface et sa distance horizontale à la caméra.
- `fog_color`, `fog`, `daylight`, `sun`, `time` donnent le brouillard, la lumière du jour,
  la direction du soleil et le temps.
- `eye` est la position de la caméra dans le monde.
- `water` indique une surface d'eau. `water_depth` est l'épaisseur de l'eau.
  `underwater` indique le degré d'immersion.

### Ombres du soleil

`sun_shadows: true` active une carte de profondeur locale de 2048 × 2048.
Le terrain et les feuilles projettent une ombre directionnelle suivant la
position du soleil, même quand ils se trouvent hors du champ de la caméra.
Le terrain reçoit les ombres avant la teinte du pack. La pénombre s'élargit
avec la distance entre l'obstacle et la surface, tout en gardant un bord
plus net au contact. Le soleil s'éteint progressivement près de l'horizon
et la carte se fond
dans l'éclairage du terrain lointain. Le mode Standard désactive la passe.

### Ciel

Le fichier de ciel définit `fn voxel_sky(input: SkyEffect) -> vec3<f32>`.
`color` contient le ciel après composition des nuages. `ray` et `sun` sont
des directions. `overcast` et `underwater` varient de 0 à 1. `time` est le temps
de la météo en secondes.

### Rayons et clair de lune

`sun_rays: true` intègre la lumière diffusée à demi-résolution. La profondeur
de la scène arrête chaque rayon au premier obstacle et la carte d'ombres
décide des portions éclairées. Un filtre guidé par la profondeur préserve
les silhouettes lors de la composition. La passe se désactive sous l'eau.

`moonlight: true` utilise la même carte d'ombres la nuit, orientée vers la
lune. La lumière devient directionnelle et l'eau reflète davantage l'astre
lorsqu'il est visible. La couverture nuageuse atténue l'effet. Les rayons
lorsque les deux options sont activées sont plus faibles sous la lune.

Voir les deux fichiers du [pack Crépuscule](../shader_packs/crepuscule/)
pour un exemple complet.
