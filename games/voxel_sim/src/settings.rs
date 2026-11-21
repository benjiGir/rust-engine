//! Les options du joueur, sauvegardées dans `settings.ron` (à la racine du projet).
//!
//! Distance de rendu, champ de vision, sensibilité de la souris, volumes, synchronisation
//! verticale, bras visible, et les touches. Un fichier absent ou illisible donne les
//! valeurs par défaut : le jeu démarre toujours.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TexturePack {
    Earth32,
    Earth64,
    #[default]
    #[serde(other)]
    Classic,
}

/// Une action du joueur, liée à une touche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Forward,
    Back,
    Left,
    Right,
    Jump,
    Descend,
    Sprint,
    Inventory,
    Drop,
    Fly,
    Time,
    Backpack,
    /// L'écran des paliers du monde.
    Tiers,
    /// Le livre de quêtes.
    Quests,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Forward,
        Action::Back,
        Action::Left,
        Action::Right,
        Action::Jump,
        Action::Descend,
        Action::Sprint,
        Action::Inventory,
        Action::Drop,
        Action::Fly,
        Action::Time,
        Action::Backpack,
        Action::Tiers,
        Action::Quests,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Forward => "Avancer",
            Self::Back => "Reculer",
            Self::Left => "Gauche",
            Self::Right => "Droite",
            Self::Jump => "Sauter / monter",
            Self::Descend => "Descendre (vol)",
            Self::Sprint => "Courir",
            Self::Inventory => "Inventaire",
            Self::Drop => "Jeter un objet",
            Self::Fly => "Voler",
            Self::Time => "Vitesse du temps",
            Self::Backpack => "Ouvrir le sac à dos",
            Self::Tiers => "Paliers du monde",
            Self::Quests => "Livre de quêtes",
        }
    }
}

/// Les touches, par leur nom (`KeyW`, `Space`...) : lisible et stable dans le fichier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub forward: String,
    pub back: String,
    pub left: String,
    pub right: String,
    pub jump: String,
    pub descend: String,
    pub sprint: String,
    pub inventory: String,
    pub drop: String,
    pub fly: String,
    pub time: String,
    pub backpack: String,
    /// L'écran des paliers du monde (absent d'un ancien `settings.ron` : la touche par défaut).
    pub tiers: String,
    /// Le livre de quêtes (absent d'un ancien `settings.ron` : la touche par défaut).
    pub quests: String,
}

impl Default for Keys {
    fn default() -> Self {
        // Des touches physiques : `KeyW` est Z sur un clavier AZERTY.
        Self {
            forward: "KeyW".into(),
            back: "KeyS".into(),
            left: "KeyA".into(),
            right: "KeyD".into(),
            jump: "Space".into(),
            descend: "ShiftLeft".into(),
            sprint: "ControlLeft".into(),
            inventory: "KeyE".into(),
            drop: "KeyQ".into(),
            fly: "KeyF".into(),
            time: "KeyT".into(),
            backpack: "KeyB".into(),
            tiers: "KeyJ".into(),
            quests: "KeyL".into(),
        }
    }
}

impl Keys {
    pub fn name_mut(&mut self, action: Action) -> &mut String {
        match action {
            Action::Forward => &mut self.forward,
            Action::Back => &mut self.back,
            Action::Left => &mut self.left,
            Action::Right => &mut self.right,
            Action::Jump => &mut self.jump,
            Action::Descend => &mut self.descend,
            Action::Sprint => &mut self.sprint,
            Action::Inventory => &mut self.inventory,
            Action::Drop => &mut self.drop,
            Action::Fly => &mut self.fly,
            Action::Time => &mut self.time,
            Action::Backpack => &mut self.backpack,
            Action::Tiers => &mut self.tiers,
            Action::Quests => &mut self.quests,
        }
    }

    pub fn name_of(&self, action: Action) -> &str {
        match action {
            Action::Forward => &self.forward,
            Action::Back => &self.back,
            Action::Left => &self.left,
            Action::Right => &self.right,
            Action::Jump => &self.jump,
            Action::Descend => &self.descend,
            Action::Sprint => &self.sprint,
            Action::Inventory => &self.inventory,
            Action::Drop => &self.drop,
            Action::Fly => &self.fly,
            Action::Time => &self.time,
            Action::Backpack => &self.backpack,
            Action::Tiers => &self.tiers,
            Action::Quests => &self.quests,
        }
    }

    /// Le nom de la touche d'une action, tel qu'affiché dans les options.
    pub fn name(&self, action: Action) -> &str {
        self.name_of(action)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Pack graphique choisi dans le menu, appliqué à l'ouverture d'un monde.
    pub texture_pack: TexturePack,
    pub shader_pack: Option<String>,
    /// Rayon de chargement, en chunks (appliqué au prochain monde chargé).
    pub render_distance: i32,
    /// Jusqu'où on dessine le terrain lointain simplifié, en chunks (0 : pas de terrain
    /// lointain). Appliqué au prochain monde chargé.
    pub far_distance: i32,
    /// Champ de vision vertical, en degrés.
    pub fov: f32,
    pub mouse_sensitivity: f32,
    /// Volumes de 0 à 1 ; le général multiplie les deux autres.
    pub volume_master: f32,
    pub volume_music: f32,
    pub volume_effects: f32,
    pub vsync: bool,
    pub show_arm: bool,
    pub ui_scale: f32,
    pub target_info: bool,
    /// Voir le contenu d'un coffre en le regardant (dans la fiche de visée).
    pub chest_preview: bool,
    /// La vitesse maximale des wagons sur rail, en blocs par seconde (8 par défaut, 30 au plus).
    pub cart_max_speed: f32,
    pub keys: Keys,
    /// Les favoris de l'explorateur de recettes (identifiants d'objets).
    pub bookmarks: Vec<String>,
    /// Le nom du joueur local.
    pub player_name: String,
    /// L'identifiant du joueur local, 32 chiffres hexadécimaux, tiré au hasard au premier
    /// lancement : il nomme son fichier `players/<uuid>.ron` dans chaque monde.
    pub player_uuid: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            texture_pack: TexturePack::Classic,
            shader_pack: None,
            render_distance: 16,
            far_distance: 64,
            fov: 70.0,
            mouse_sensitivity: 1.0,
            volume_master: 1.0,
            volume_music: 0.35,
            volume_effects: 0.8,
            vsync: true,
            show_arm: true,
            ui_scale: 1.0,
            target_info: true,
            chest_preview: true,
            cart_max_speed: 8.0,
            keys: Keys::default(),
            bookmarks: Vec::new(),
            player_name: "Joueur".into(),
            player_uuid: String::new(),
        }
    }
}

impl Settings {
    pub fn path(root: &Path) -> PathBuf {
        root.join("settings.ron")
    }

    pub fn load(root: &Path) -> Self {
        let path = Self::path(root);
        match std::fs::read_to_string(&path) {
            Ok(text) => ron::from_str(&text).unwrap_or_else(|err| {
                log::warn!("{} illisible ({err}) : options par défaut", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, root: &Path) {
        let path = Self::path(root);
        let pretty = ron::ser::PrettyConfig::default();
        match ron::ser::to_string_pretty(self, pretty) {
            Ok(text) => {
                if let Err(err) = engine_voxel::write_atomic(&path, text.as_bytes()) {
                    log::error!("{} : {err:#}", path.display());
                }
            }
            Err(err) => log::error!("options : {err}"),
        }
    }

    /// Le volume réel des effets et de la musique (le général compris).
    pub fn volumes(&self) -> (f32, f32) {
        (
            self.volume_master * self.volume_effects,
            self.volume_master * self.volume_music,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_settings_keep_the_default_ui_scale_and_enable_target_information() {
        let loaded: Settings = ron::from_str("(fov:80.0,show_arm:false)").unwrap();
        assert_eq!(loaded.ui_scale, 1.0);
        assert_eq!(loaded.texture_pack, TexturePack::Classic);
        assert_eq!(loaded.shader_pack, None);
        assert!(loaded.target_info);
        assert!(
            loaded.chest_preview,
            "l'aperçu des coffres est activé par défaut"
        );
        assert_eq!(loaded.keys.name(Action::Backpack), "KeyB");
        assert_eq!(loaded.keys.name(Action::Tiers), "KeyJ");
        assert_eq!(loaded.keys.name(Action::Quests), "KeyL");
        assert_eq!(loaded.fov, 80.0);
        assert!(!loaded.show_arm);
    }

    #[test]
    fn an_older_settings_file_without_the_backpack_key_still_loads() {
        let loaded: Settings =
            ron::from_str("(fov:80.0,keys:(forward:\"ArrowUp\",inventory:\"KeyI\"))").unwrap();
        assert_eq!(loaded.keys.name(Action::Forward), "ArrowUp");
        assert_eq!(loaded.keys.name(Action::Inventory), "KeyI");
        assert_eq!(loaded.keys.name(Action::Backpack), "KeyB");
        assert_eq!(loaded.keys.name(Action::Tiers), "KeyJ");
        assert_eq!(loaded.keys.name(Action::Quests), "KeyL");
        assert!(loaded.chest_preview);
    }

    #[test]
    fn settings_round_trip_and_keys_remap() {
        let dir = std::env::temp_dir().join(format!("voxel-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut settings = Settings::default();
        assert_eq!(settings.keys.name(Action::Forward), "KeyW");
        *settings.keys.name_mut(Action::Forward) = "ArrowUp".into();
        settings.fov = 90.0;
        settings.texture_pack = TexturePack::Earth64;
        settings.shader_pack = Some("crepuscule".into());
        settings.save(&dir);
        let back = Settings::load(&dir);
        assert_eq!(back, settings);
        assert_eq!(back.keys.name(Action::Forward), "ArrowUp");
        assert_eq!(back.keys.name(Action::Forward), "ArrowUp");
        // Un fichier abîmé : les valeurs par défaut, sans planter.
        std::fs::write(Settings::path(&dir), "(fov: oups").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
    }

    #[test]
    fn missing_fields_take_their_default() {
        let settings: Settings = ron::from_str("(fov: 85.0)").unwrap();
        assert_eq!(settings.fov, 85.0);
        assert_eq!(settings.render_distance, 16);
        assert_eq!(settings.keys, Keys::default());
        // Un fichier d'avant les wagons : la vitesse maximale par défaut.
        assert_eq!(settings.cart_max_speed, 8.0);
    }
}
