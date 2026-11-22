//! Les touches du clavier (winit) derrière les noms de `settings.ron` : la fenêtre seule en a besoin,
//! les réglages ne gardent que les noms.

use engine_core::winit::keyboard::KeyCode;

use voxel_sim::settings::{Action, Keys};

/// La touche réelle d'une action, et son changement depuis les options.
pub trait KeyCodes {
    /// La touche d'une action (celle par défaut si le nom est inconnu).
    fn key(&self, action: Action) -> KeyCode;
    fn set(&mut self, action: Action, key: KeyCode);
}

impl KeyCodes for Keys {
    fn key(&self, action: Action) -> KeyCode {
        key_code(self.name_of(action))
            .or_else(|| key_code(voxel_sim::settings::Keys::default().name_of(action)))
            .unwrap_or(KeyCode::Escape)
    }

    fn set(&mut self, action: Action, key: KeyCode) {
        if let Some(name) = key_name(key) {
            *self.name_mut(action) = name.to_string();
        }
    }
}

/// Les touches qu'on peut associer à une action, avec leur nom.
const KEYS: &[(&str, KeyCode)] = &[
    ("KeyA", KeyCode::KeyA),
    ("KeyB", KeyCode::KeyB),
    ("KeyC", KeyCode::KeyC),
    ("KeyD", KeyCode::KeyD),
    ("KeyE", KeyCode::KeyE),
    ("KeyF", KeyCode::KeyF),
    ("KeyG", KeyCode::KeyG),
    ("KeyH", KeyCode::KeyH),
    ("KeyI", KeyCode::KeyI),
    ("KeyJ", KeyCode::KeyJ),
    ("KeyK", KeyCode::KeyK),
    ("KeyL", KeyCode::KeyL),
    ("KeyM", KeyCode::KeyM),
    ("KeyN", KeyCode::KeyN),
    ("KeyO", KeyCode::KeyO),
    ("KeyP", KeyCode::KeyP),
    ("KeyQ", KeyCode::KeyQ),
    ("KeyR", KeyCode::KeyR),
    ("KeyS", KeyCode::KeyS),
    ("KeyT", KeyCode::KeyT),
    ("KeyU", KeyCode::KeyU),
    ("KeyV", KeyCode::KeyV),
    ("KeyW", KeyCode::KeyW),
    ("KeyX", KeyCode::KeyX),
    ("KeyY", KeyCode::KeyY),
    ("KeyZ", KeyCode::KeyZ),
    ("Space", KeyCode::Space),
    ("ShiftLeft", KeyCode::ShiftLeft),
    ("ShiftRight", KeyCode::ShiftRight),
    ("ControlLeft", KeyCode::ControlLeft),
    ("ControlRight", KeyCode::ControlRight),
    ("AltLeft", KeyCode::AltLeft),
    ("Tab", KeyCode::Tab),
    ("CapsLock", KeyCode::CapsLock),
    ("ArrowUp", KeyCode::ArrowUp),
    ("ArrowDown", KeyCode::ArrowDown),
    ("ArrowLeft", KeyCode::ArrowLeft),
    ("ArrowRight", KeyCode::ArrowRight),
];

pub fn key_code(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

pub fn key_name(key: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(_, k)| *k == key).map(|(n, _)| *n)
}
