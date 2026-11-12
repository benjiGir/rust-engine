//! Des entrées simulées dans l'interface, pour `ctl` : clics, texte, touches. Elles passent par egui
//! comme celles de la vraie souris et du vrai clavier, une image après l'autre (un clic = appui,
//! puis relâchement à l'image suivante). Les textes affichés à la dernière image sont gardés avec
//! leur position, pour retrouver un bouton par son libellé.

use std::collections::VecDeque;

use engine_render::egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

#[derive(Default)]
pub struct UiInput {
    /// Les événements à donner à egui, un lot par image.
    frames: VecDeque<Vec<Event>>,
    /// Les textes dessinés à la dernière image : (texte, rectangle en points).
    texts: Vec<(String, Rect)>,
}

impl UiInput {
    /// Un clic en `pos` (en points egui) : appui à la prochaine image, relâchement à la suivante.
    pub fn click(&mut self, pos: Pos2, button: PointerButton) {
        let modifiers = Modifiers::NONE;
        self.frames.push_back(vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button,
                pressed: true,
                modifiers,
            },
        ]);
        self.frames.push_back(vec![Event::PointerButton {
            pos,
            button,
            pressed: false,
            modifiers,
        }]);
    }

    /// Du texte tapé (ce que produit le clavier pour les champs de saisie).
    pub fn text(&mut self, text: &str) {
        self.frames.push_back(vec![Event::Text(text.to_string())]);
    }

    /// Une touche (nom egui : `Enter`, `ArrowUp`, `A`...), avec ses modificateurs.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) {
        let event = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        };
        self.frames.push_back(vec![event(true)]);
        self.frames.push_back(vec![event(false)]);
    }

    /// Les événements de cette image.
    pub fn next_frame(&mut self) -> Vec<Event> {
        self.frames.pop_front().unwrap_or_default()
    }

    /// Retient les textes de l'image qui vient d'être dessinée.
    pub fn capture(&mut self, shapes: &[engine_render::egui::epaint::ClippedShape]) {
        self.texts.clear();
        for clipped in shapes {
            collect(&clipped.shape, &mut self.texts);
        }
    }

    /// Le centre du premier texte qui contient `needle` (sans tenir compte de la casse).
    pub fn find(&self, needle: &str) -> Option<Pos2> {
        let needle = needle.to_lowercase();
        self.texts
            .iter()
            .find(|(text, _)| text.to_lowercase().contains(&needle))
            .map(|(_, rect)| rect.center())
    }

    /// Les textes de la dernière image, en JSON : `[{"text":..,"x":..,"y":..,"w":..,"h":..}]`.
    pub fn texts_json(&self) -> String {
        let rows: Vec<String> = self
            .texts
            .iter()
            .map(|(text, rect)| {
                format!(
                    "{{\"text\":{},\"x\":{:.0},\"y\":{:.0},\"w\":{:.0},\"h\":{:.0}}}",
                    crate::control::json_str(text),
                    rect.center().x,
                    rect.center().y,
                    rect.width(),
                    rect.height()
                )
            })
            .collect();
        format!("[{}]", rows.join(","))
    }
}

fn collect(shape: &engine_render::egui::epaint::Shape, out: &mut Vec<(String, Rect)>) {
    use engine_render::egui::epaint::Shape;
    match shape {
        Shape::Vec(shapes) => shapes.iter().for_each(|s| collect(s, out)),
        Shape::Text(text) => {
            let content = text.galley.text().trim().to_string();
            if !content.is_empty() {
                let rect = text.galley.rect.translate(text.pos.to_vec2());
                out.push((content, rect));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_click_is_a_press_then_a_release_on_the_next_frame() {
        let mut input = UiInput::default();
        input.click(Pos2::new(10.0, 20.0), PointerButton::Primary);
        let first = input.next_frame();
        assert!(matches!(first[0], Event::PointerMoved(_)));
        assert!(matches!(
            first[1],
            Event::PointerButton { pressed: true, .. }
        ));
        let second = input.next_frame();
        assert!(matches!(
            second[0],
            Event::PointerButton { pressed: false, .. }
        ));
        assert!(input.next_frame().is_empty());
    }

    #[test]
    fn commands_keep_their_order() {
        let mut input = UiInput::default();
        input.text("a");
        input.key(Key::Enter, Modifiers::NONE);
        assert!(matches!(input.next_frame()[0], Event::Text(_)));
        assert!(matches!(
            input.next_frame()[0],
            Event::Key { pressed: true, .. }
        ));
    }
}
