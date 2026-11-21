//! Le trait [`Inspect`] : montrer une valeur et la modifier, sans savoir comment elle est dessinée.
//!
//! Une valeur se décrit à un [`Inspector`] (un nombre, un booléen, un vecteur, un groupe de champs) ;
//! c'est l'inspecteur qui dessine, par exemple avec egui dans la fenêtre du jeu (l'inspecteur F3).
//! Le moteur et la simulation n'ont donc pas besoin d'une bibliothèque d'interface : un serveur sans
//! fenêtre dérive `Inspect` sans lier egui.
//!
//! Les types de base l'implémentent ici ; une struct l'obtient avec `#[derive(Inspect)]`, qui décrit
//! chaque champ dans un groupe.

pub use engine_inspect_derive::Inspect;

/// Ce qui dessine les valeurs (et rend vrai quand l'utilisateur en a changé une).
pub trait Inspector {
    fn float(&mut self, label: &str, value: &mut f64) -> bool;
    fn int(&mut self, label: &str, value: &mut i64) -> bool;
    fn boolean(&mut self, label: &str, value: &mut bool) -> bool;
    fn text(&mut self, label: &str, value: &mut String) -> bool;
    /// Un vecteur de réels (x, y, z…).
    fn floats(&mut self, label: &str, values: &mut [f32]) -> bool;
    /// Un vecteur d'entiers.
    fn ints(&mut self, label: &str, values: &mut [i32]) -> bool;
    /// Une valeur facultative absente.
    fn absent(&mut self, label: &str);
    /// Un groupe de champs, décrits par `body`.
    fn group(&mut self, label: &str, body: &mut dyn FnMut(&mut dyn Inspector) -> bool) -> bool;
}

/// Une valeur modifiable dans un inspecteur.
pub trait Inspect {
    /// Décrit la valeur ; renvoie `true` si l'utilisateur l'a modifiée.
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool;

    /// Les noms des champs (vide pour les types simples).
    fn field_names() -> &'static [&'static str]
    where
        Self: Sized,
    {
        &[]
    }
}

/// Les réels passent par `f64`.
macro_rules! impl_float {
    ($($t:ty),*) => {$(
        impl Inspect for $t {
            fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
                let mut value = f64::from(*self);
                let changed = inspector.float(label, &mut value);
                if changed {
                    *self = value as $t;
                }
                changed
            }
        }
    )*};
}

/// Les entiers passent par `i64` ; une valeur hors de leur intervalle est bornée.
macro_rules! impl_int {
    ($($t:ty),*) => {$(
        impl Inspect for $t {
            fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
                let mut value = i64::try_from(*self).unwrap_or(i64::MAX);
                let changed = inspector.int(label, &mut value);
                if changed {
                    *self = <$t>::try_from(value).unwrap_or(if value < 0 {
                        <$t>::MIN
                    } else {
                        <$t>::MAX
                    });
                }
                changed
            }
        }
    )*};
}

impl_float!(f32, f64);
impl_int!(i32, u32, i64, u64, u8, u16, usize);

impl Inspect for bool {
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
        inspector.boolean(label, self)
    }
}

impl Inspect for String {
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
        inspector.text(label, self)
    }
}

impl Inspect for glam::Vec3 {
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
        let mut values = self.to_array();
        let changed = inspector.floats(label, &mut values);
        *self = glam::Vec3::from_array(values);
        changed
    }
}

impl Inspect for glam::IVec3 {
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
        let mut values = self.to_array();
        let changed = inspector.ints(label, &mut values);
        *self = glam::IVec3::from_array(values);
        changed
    }
}

impl<T: Inspect> Inspect for Option<T> {
    fn inspect(&mut self, inspector: &mut dyn Inspector, label: &str) -> bool {
        match self {
            Some(value) => value.inspect(inspector, label),
            None => {
                inspector.absent(label);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un inspecteur qui écrit ce qu'on lui décrit, et change les entiers en 7.
    #[derive(Default)]
    struct Log(Vec<String>);

    impl Inspector for Log {
        fn float(&mut self, label: &str, value: &mut f64) -> bool {
            self.0.push(format!("{label}={value}"));
            false
        }
        fn int(&mut self, label: &str, value: &mut i64) -> bool {
            self.0.push(format!("{label}={value}"));
            *value = 7;
            true
        }
        fn boolean(&mut self, label: &str, value: &mut bool) -> bool {
            self.0.push(format!("{label}={value}"));
            false
        }
        fn text(&mut self, label: &str, value: &mut String) -> bool {
            self.0.push(format!("{label}={value}"));
            false
        }
        fn floats(&mut self, label: &str, values: &mut [f32]) -> bool {
            self.0.push(format!("{label}={values:?}"));
            false
        }
        fn ints(&mut self, label: &str, values: &mut [i32]) -> bool {
            self.0.push(format!("{label}={values:?}"));
            false
        }
        fn absent(&mut self, label: &str) {
            self.0.push(format!("{label} absent"));
        }
        fn group(&mut self, label: &str, body: &mut dyn FnMut(&mut dyn Inspector) -> bool) -> bool {
            self.0.push(format!("[{label}]"));
            body(self)
        }
    }

    #[test]
    fn values_are_described_and_changed_through_the_inspector() {
        let mut log = Log::default();
        let mut health = 3u8;
        assert!(health.inspect(&mut log, "vie"));
        assert_eq!(health, 7);
        let mut far = 1u64 << 63;
        far.inspect(&mut log, "loin");
        assert_eq!(far, 7);
        let mut pos = glam::Vec3::ONE;
        pos.inspect(&mut log, "pos");
        Inspect::inspect(&mut None::<f32>, &mut log, "rien");
        assert_eq!(
            log.0,
            [
                "vie=3",
                &format!("loin={}", i64::MAX),
                "pos=[1.0, 1.0, 1.0]",
                "rien absent"
            ]
        );
    }
}
