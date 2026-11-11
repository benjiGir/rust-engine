//! Le menu qu'un mod décrit pour une de ses machines (voir `mod_sdk::ui`). Le script envoie
//! une suite d'octets ; ce module la lit, la vérifie (taille, nombre d'éléments, textes
//! valides : un mod ne doit pas pouvoir faire planter l'interface) et la range dans un
//! [`ModMenu`] que le jeu dessine.

use anyhow::{Result, bail, ensure};

/// Au plus ce nombre d'octets, d'éléments et de cases par menu.
pub const MAX_MENU_BYTES: usize = 16 * 1024;
pub const MAX_WIDGETS: usize = 256;
pub const MAX_SLOTS: usize = 64;

/// Les types d'actions du joueur envoyées au script (`on_ui`).
pub const UI_BUTTON: i32 = 1;
pub const UI_TOGGLE: i32 = 2;
pub const UI_SLIDER: i32 = 3;
pub const UI_REFRESH: i32 = 4;
pub const UI_CLOSED: i32 = 5;

/// Un élément d'un menu.
#[derive(Debug, Clone, PartialEq)]
pub enum Widget {
    Label(String),
    Separator,
    /// Une jauge de 0 à 1 avec son texte.
    Bar {
        text: String,
        value: f32,
    },
    Button {
        id: u32,
        text: String,
    },
    Toggle {
        id: u32,
        text: String,
        on: bool,
    },
    Slider {
        id: u32,
        text: String,
        min: f32,
        max: f32,
        value: f32,
    },
    /// `count` cases de l'inventaire de la machine, à partir de `first`, sur `columns` colonnes.
    Slots {
        first: usize,
        count: usize,
        columns: usize,
    },
    RowBegin,
    RowEnd,
}

/// Un menu : un titre et ses éléments, de haut en bas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModMenu {
    pub title: String,
    pub widgets: Vec<Widget>,
}

/// Un lecteur d'octets qui refuse de dépasser la fin.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        ensure!(self.0.len() >= n, "menu tronqué");
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }

    /// Un décimal fini (un NaN ferait planter les curseurs de l'interface).
    fn f32(&mut self) -> Result<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into()?);
        ensure!(v.is_finite(), "nombre invalide dans le menu");
        Ok(v)
    }

    fn text(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        Ok(String::from_utf8(self.take(len)?.to_vec())?)
    }
}

impl ModMenu {
    /// Lit un menu. Une erreur dit pourquoi il est refusé (le jeu l'écrit dans les logs).
    pub fn parse(bytes: &[u8]) -> Result<ModMenu> {
        ensure!(
            bytes.len() <= MAX_MENU_BYTES,
            "menu trop gros ({} octets, {MAX_MENU_BYTES} au maximum)",
            bytes.len()
        );
        let mut r = Reader(bytes);
        let mut menu = ModMenu::default();
        let (mut depth, mut slots) = (0usize, 0usize);
        while !r.0.is_empty() {
            let tag = r.u8()?;
            let widget = match tag {
                0 => {
                    menu.title = r.text()?;
                    continue;
                }
                1 => Widget::Label(r.text()?),
                2 => Widget::Separator,
                3 => Widget::Bar {
                    text: r.text()?,
                    value: r.f32()?.clamp(0.0, 1.0),
                },
                4 => Widget::Button {
                    id: r.u32()?,
                    text: r.text()?,
                },
                5 => Widget::Toggle {
                    id: r.u32()?,
                    text: r.text()?,
                    on: r.u8()? != 0,
                },
                6 => {
                    let (id, text) = (r.u32()?, r.text()?);
                    let (min, max, value) = (r.f32()?, r.f32()?, r.f32()?);
                    ensure!(
                        min < max,
                        "curseur « {text} » : min doit être inférieur à max"
                    );
                    Widget::Slider {
                        id,
                        text,
                        min,
                        max,
                        value: value.clamp(min, max),
                    }
                }
                7 => {
                    let (first, count, columns) = (r.u16()? as usize, r.u16()? as usize, r.u8()?);
                    slots += count;
                    ensure!(slots <= MAX_SLOTS, "trop de cases ({MAX_SLOTS} au maximum)");
                    Widget::Slots {
                        first,
                        count,
                        columns: (columns as usize).max(1),
                    }
                }
                8 => {
                    depth += 1;
                    ensure!(depth == 1, "les lignes ne s'imbriquent pas");
                    Widget::RowBegin
                }
                9 => {
                    ensure!(depth == 1, "fin de ligne sans début");
                    depth = 0;
                    Widget::RowEnd
                }
                other => bail!("élément de menu inconnu ({other})"),
            };
            menu.widgets.push(widget);
            ensure!(
                menu.widgets.len() <= MAX_WIDGETS,
                "trop d'éléments ({MAX_WIDGETS} au maximum)"
            );
        }
        // Une ligne laissée ouverte est refermée plutôt que de refuser le menu.
        if depth > 0 {
            menu.widgets.push(Widget::RowEnd);
        }
        Ok(menu)
    }
}

#[cfg(test)]
mod tests {
    use mod_sdk::ui::Menu;

    use super::*;

    #[test]
    fn the_sdk_encoding_is_read_back() {
        let bytes = Menu::new("Presse")
            .label("Prête")
            .bar("Progression", 0.4)
            .row_begin()
            .slots(0, 2, 2)
            .slots(2, 1, 1)
            .row_end()
            .toggle(1, "Active", true)
            .slider(3, "Débit", 0.0, 10.0, 99.0)
            .separator()
            .button(2, "Vider");
        let menu = ModMenu::parse(bytes.as_bytes()).unwrap();
        assert_eq!(menu.title, "Presse");
        assert_eq!(menu.widgets.len(), 10);
        assert_eq!(
            menu.widgets[1],
            Widget::Bar {
                text: "Progression".into(),
                value: 0.4
            }
        );
        assert_eq!(
            menu.widgets[2..6],
            [
                Widget::RowBegin,
                Widget::Slots {
                    first: 0,
                    count: 2,
                    columns: 2
                },
                Widget::Slots {
                    first: 2,
                    count: 1,
                    columns: 1
                },
                Widget::RowEnd
            ]
        );
        // Un curseur hors bornes est ramené dans ses bornes.
        assert!(matches!(menu.widgets[7], Widget::Slider { value, .. } if value == 10.0));
    }

    #[test]
    fn the_event_numbers_match_the_sdk() {
        assert_eq!(
            [UI_BUTTON, UI_TOGGLE, UI_SLIDER, UI_REFRESH, UI_CLOSED],
            [
                mod_sdk::ui::UI_BUTTON,
                mod_sdk::ui::UI_TOGGLE,
                mod_sdk::ui::UI_SLIDER,
                mod_sdk::ui::UI_REFRESH,
                mod_sdk::ui::UI_CLOSED
            ]
        );
        assert_eq!(MAX_MENU_BYTES, mod_sdk::ui::MAX_MENU_BYTES);
    }

    #[test]
    fn broken_menus_are_refused_with_a_reason() {
        let err = |bytes: &[u8]| ModMenu::parse(bytes).unwrap_err().to_string();
        assert!(err(&[1, 5, 0, b'a']).contains("tronqué"));
        assert!(err(&[99]).contains("inconnu"));
        assert!(
            err(&[1, 2, 0, 0xff, 0xfe]).contains("utf"),
            "texte invalide"
        );
        assert!(err(&[9]).contains("sans début"));
        assert!(err(&[8, 8]).contains("imbriquent"));
        // Un décimal NaN.
        let mut nan = vec![3, 0, 0];
        nan.extend(f32::NAN.to_le_bytes());
        assert!(err(&nan).contains("invalide"));
        // Trop gros, trop d'éléments, trop de cases.
        assert!(err(&vec![2; MAX_MENU_BYTES + 1]).contains("trop gros"));
        assert!(err(&vec![2; MAX_WIDGETS + 1]).contains("trop d'éléments"));
        let slots = Menu::default().slots(0, 40, 8).slots(40, 40, 8);
        assert!(err(slots.as_bytes()).contains("trop de cases"));
        // Une ligne ouverte est refermée.
        let open = ModMenu::parse(Menu::default().row_begin().label("a").as_bytes()).unwrap();
        assert_eq!(open.widgets.last(), Some(&Widget::RowEnd));
    }
}
