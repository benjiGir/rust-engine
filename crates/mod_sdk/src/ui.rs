//! Les menus des machines : le mod **décrit** un menu (titre, textes, jauges, boutons,
//! cases d'inventaire...), le jeu le dessine et renvoie les actions du joueur. Aucun
//! dessin ne passe par le script : un menu ouvert ne coûte rien tant que le joueur n'agit pas.
//!
//! ```ignore
//! let menu = Menu::new("Presse")
//!     .label("Prête")
//!     .bar("Progression", 0.4)
//!     .row_begin().slots(0, 2, 2).slots(2, 1, 1).row_end()
//!     .toggle(1, "Active", true)
//!     .button(2, "Vider");
//! mod_sdk::show_menu(pos, &menu);
//! ```
//!
//! Les actions arrivent dans [`crate::Mod::ui`]. Pendant que le menu est ouvert, le jeu envoie
//! aussi [`UiEvent::Refresh`] cinq fois par seconde : le mod y rappelle `show_menu` pour
//! mettre les jauges à jour.

/// Au plus ce nombre d'octets pour un menu.
pub const MAX_MENU_BYTES: usize = 16 * 1024;

/// Les types d'actions du joueur (le paramètre `kind` de `on_ui`).
pub const UI_BUTTON: i32 = 1;
pub const UI_TOGGLE: i32 = 2;
pub const UI_SLIDER: i32 = 3;
pub const UI_REFRESH: i32 = 4;
pub const UI_CLOSED: i32 = 5;

// Les étiquettes du format (voir `docs/modding.md`).
pub const TAG_TITLE: u8 = 0;
pub const TAG_LABEL: u8 = 1;
pub const TAG_SEPARATOR: u8 = 2;
pub const TAG_BAR: u8 = 3;
pub const TAG_BUTTON: u8 = 4;
pub const TAG_TOGGLE: u8 = 5;
pub const TAG_SLIDER: u8 = 6;
pub const TAG_SLOTS: u8 = 7;
pub const TAG_ROW_BEGIN: u8 = 8;
pub const TAG_ROW_END: u8 = 9;

/// Une action du joueur dans un menu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEvent {
    /// Un bouton (son identifiant) a été pressé.
    Button(u32),
    /// Une case à cocher a changé.
    Toggle(u32, bool),
    /// Un curseur a bougé.
    Slider(u32, f32),
    /// Le menu est ouvert : c'est le moment de le rafraîchir (5 fois par seconde).
    Refresh,
    /// Le joueur a fermé le menu.
    Closed,
}

impl UiEvent {
    pub(crate) fn decode(kind: i32, id: i32, value: i32) -> Option<UiEvent> {
        Some(match kind {
            UI_BUTTON => UiEvent::Button(id as u32),
            UI_TOGGLE => UiEvent::Toggle(id as u32, value != 0),
            UI_SLIDER => UiEvent::Slider(id as u32, f32::from_bits(value as u32)),
            UI_REFRESH => UiEvent::Refresh,
            UI_CLOSED => UiEvent::Closed,
            _ => return None,
        })
    }
}

/// Un menu en construction. Chaque méthode ajoute un élément, de haut en bas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Menu {
    bytes: Vec<u8>,
}

impl Menu {
    /// Un menu avec son titre.
    pub fn new(title: &str) -> Menu {
        let mut menu = Menu::default();
        menu.bytes.push(TAG_TITLE);
        menu.text(title);
        menu
    }

    fn text(&mut self, text: &str) {
        // Les textes sont tronqués à 255 octets, sur une frontière de caractère.
        let mut end = text.len().min(255);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.bytes.extend((end as u16).to_le_bytes());
        self.bytes.extend(&text.as_bytes()[..end]);
    }

    fn float(&mut self, value: f32) {
        self.bytes.extend(value.to_le_bytes());
    }

    /// Une ligne de texte.
    pub fn label(mut self, text: &str) -> Menu {
        self.bytes.push(TAG_LABEL);
        self.text(text);
        self
    }

    /// Un trait de séparation.
    pub fn separator(mut self) -> Menu {
        self.bytes.push(TAG_SEPARATOR);
        self
    }

    /// Une jauge de 0 à 1 (une progression, un niveau), avec son texte.
    pub fn bar(mut self, text: &str, value: f32) -> Menu {
        self.bytes.push(TAG_BAR);
        self.text(text);
        self.float(value);
        self
    }

    /// Un bouton : le mod reçoit [`UiEvent::Button`] avec cet identifiant.
    pub fn button(mut self, id: u32, text: &str) -> Menu {
        self.bytes.push(TAG_BUTTON);
        self.bytes.extend(id.to_le_bytes());
        self.text(text);
        self
    }

    /// Une case à cocher.
    pub fn toggle(mut self, id: u32, text: &str, on: bool) -> Menu {
        self.bytes.push(TAG_TOGGLE);
        self.bytes.extend(id.to_le_bytes());
        self.text(text);
        self.bytes.push(on as u8);
        self
    }

    /// Un curseur de `min` à `max`.
    pub fn slider(mut self, id: u32, text: &str, min: f32, max: f32, value: f32) -> Menu {
        self.bytes.push(TAG_SLIDER);
        self.bytes.extend(id.to_le_bytes());
        self.text(text);
        self.float(min);
        self.float(max);
        self.float(value);
        self
    }

    /// Des cases de l'inventaire de la machine (voir `add_inventory`) : `count` cases à partir
    /// de la case `first`, sur `columns` colonnes. Le joueur y dépose et en retire des objets.
    pub fn slots(mut self, first: u16, count: u16, columns: u8) -> Menu {
        self.bytes.push(TAG_SLOTS);
        self.bytes.extend(first.to_le_bytes());
        self.bytes.extend(count.to_le_bytes());
        self.bytes.push(columns);
        self
    }

    /// Commence une ligne : les éléments suivants se placent côte à côte, jusqu'à `row_end`.
    pub fn row_begin(mut self) -> Menu {
        self.bytes.push(TAG_ROW_BEGIN);
        self
    }

    pub fn row_end(mut self) -> Menu {
        self.bytes.push(TAG_ROW_END);
        self
    }

    /// Le menu encodé, tel que le jeu le lit.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
