//! Les événements : « un bloc a été cassé », « une entité est morte »...
//!
//! Un système envoie un événement ; d'autres le lisent, sans se connaître. Le code des
//! sons n'a pas besoin de savoir qui casse les blocs : il écoute `BlockBroken`. C'est
//! aussi l'API que les mods utiliseront pour réagir au jeu.
//!
//! # Double tampon
//!
//! Les événements envoyés pendant une image restent lisibles pendant cette image et la
//! suivante, puis disparaissent : chaque lecteur a le temps de les voir, quel que soit
//! l'ordre des systèmes, et la mémoire ne grossit pas. `update()`, appelé une fois par
//! image, fait tourner les deux tampons.
//!
//! Chaque lecteur garde un curseur (le numéro du dernier événement lu) pour ne pas
//! traiter deux fois le même.

/// Les événements d'un type `T`. À ranger comme ressource du monde.
#[derive(Debug)]
pub struct Events<T> {
    /// Les événements de l'image précédente, puis ceux de l'image en cours.
    previous: Vec<(u64, T)>,
    current: Vec<(u64, T)>,
    next_id: u64,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self {
            previous: Vec::new(),
            current: Vec::new(),
            next_id: 0,
        }
    }
}

impl<T> Events<T> {
    pub fn send(&mut self, event: T) {
        self.current.push((self.next_id, event));
        self.next_id += 1;
    }

    /// Fin d'image : les événements de l'image précédente sont oubliés.
    pub fn update(&mut self) {
        self.previous = std::mem::take(&mut self.current);
    }

    /// Un lecteur qui ne verra que les événements envoyés à partir de maintenant.
    pub fn reader(&self) -> EventReader {
        EventReader { next: self.next_id }
    }

    pub fn len(&self) -> usize {
        self.previous.len() + self.current.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Tous les événements encore disponibles (sans curseur).
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.previous.iter().chain(&self.current).map(|(_, e)| e)
    }
}

/// Le curseur d'un lecteur d'événements.
#[derive(Debug, Clone, Copy, Default)]
pub struct EventReader {
    next: u64,
}

impl EventReader {
    /// Les événements pas encore vus par ce lecteur.
    pub fn read<'a, T>(&mut self, events: &'a Events<T>) -> impl Iterator<Item = &'a T> {
        let from = self.next;
        self.next = events.next_id;
        events
            .previous
            .iter()
            .chain(&events.current)
            .filter(move |(id, _)| *id >= from)
            .map(|(_, e)| e)
    }
}
