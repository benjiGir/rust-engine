//! État du clavier et de la souris, interrogeable comme `Input.GetKey` dans Unity.
//!
//! Les touches utilisent des `KeyCode` *physiques* : `KeyCode::KeyW` désigne la
//! touche en haut à gauche du pavé de déplacement, c'est-à-dire Z sur un clavier
//! AZERTY et W sur un QWERTY. Les déplacements marchent donc partout sans réglage.

use std::collections::HashSet;

use glam::Vec2;
use winit::event::MouseButton;
use winit::keyboard::KeyCode;

#[derive(Debug, Default)]
pub struct Input {
    keys_held: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    keys_released: HashSet<KeyCode>,
    buttons_held: HashSet<MouseButton>,
    buttons_pressed: HashSet<MouseButton>,
    mouse_delta: Vec2,
}

impl Input {
    /// La touche est enfoncée (vrai à chaque frame tant qu'on appuie).
    pub fn key_held(&self, key: KeyCode) -> bool {
        self.keys_held.contains(&key)
    }

    /// La touche vient d'être enfoncée (vrai pendant une seule frame).
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    /// La touche vient d'être relâchée (vrai pendant une seule frame).
    pub fn key_released(&self, key: KeyCode) -> bool {
        self.keys_released.contains(&key)
    }

    pub fn mouse_held(&self, button: MouseButton) -> bool {
        self.buttons_held.contains(&button)
    }

    pub fn mouse_pressed(&self, button: MouseButton) -> bool {
        self.buttons_pressed.contains(&button)
    }

    /// Déplacement brut de la souris depuis la frame précédente, en pixels.
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    // Les méthodes `pub(crate)` ne sont visibles que dans `engine_core` :
    // un jeu peut lire l'état, mais seule la boucle peut le modifier.

    pub(crate) fn on_key(&mut self, key: KeyCode, pressed: bool) {
        if pressed {
            // `insert` renvoie `false` si la touche était déjà tenue.
            if self.keys_held.insert(key) {
                self.keys_pressed.insert(key);
            }
        } else if self.keys_held.remove(&key) {
            self.keys_released.insert(key);
        }
    }

    pub(crate) fn on_mouse_button(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            if self.buttons_held.insert(button) {
                self.buttons_pressed.insert(button);
            }
        } else {
            self.buttons_held.remove(&button);
        }
    }

    pub(crate) fn on_mouse_motion(&mut self, dx: f64, dy: f64) {
        self.mouse_delta += Vec2::new(dx as f32, dy as f32);
    }

    /// Appelé à la fin de chaque frame : efface ce qui ne dure qu'une frame.
    pub(crate) fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.keys_released.clear();
        self.buttons_pressed.clear();
        self.mouse_delta = Vec2::ZERO;
    }

    /// Quand la fenêtre perd le focus, on oublie tout pour éviter une touche « collée ».
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressed_lasts_one_frame_held_lasts_until_release() {
        let mut input = Input::default();
        input.on_key(KeyCode::KeyW, true);
        assert!(input.key_pressed(KeyCode::KeyW));
        assert!(input.key_held(KeyCode::KeyW));

        input.end_frame();
        assert!(!input.key_pressed(KeyCode::KeyW));
        assert!(input.key_held(KeyCode::KeyW));

        input.on_key(KeyCode::KeyW, false);
        assert!(input.key_released(KeyCode::KeyW));
        assert!(!input.key_held(KeyCode::KeyW));
    }

    #[test]
    fn mouse_delta_accumulates_then_resets() {
        let mut input = Input::default();
        input.on_mouse_motion(3.0, -1.0);
        input.on_mouse_motion(2.0, -1.0);
        assert_eq!(input.mouse_delta(), Vec2::new(5.0, -2.0));
        input.end_frame();
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
    }
}
