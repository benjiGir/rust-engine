//! La part serveur de `console.rs` : ses méthodes sur `VoxelServer` (voir le lot 8 du plan
//! multijoueur ; la fenêtre reste dans `console.rs`).

impl crate::server_side::VoxelServer {
    // (lot 4b) Méthodes du serveur.
    /// Ajoute des lignes à la console (une par `\n`).
    /// Une ligne pour la console du joueur courant (le serveur la lui envoie).
    pub(crate) fn chat(&mut self, text: &str) {
        let id = self.players.current_id();
        self.replies
            .push((id, crate::protocol::ServerMsg::Chat(text.to_string())));
    }

    pub(crate) fn console_mods(&self) -> String {
        let disabled = self.mods.disabled();
        let lines: Vec<String> = self
            .mods
            .mod_stats()
            .into_iter()
            .map(|(id, s)| {
                let state = disabled
                    .iter()
                    .find(|(m, _)| *m == id)
                    .map_or_else(String::new, |(_, why)| format!(" — DÉSACTIVÉ : {why}"));
                format!(
                    "{id} : {} instr. au dernier tick (pic {}, budget {}){state}",
                    s.fuel_last_tick,
                    s.fuel_peak_tick,
                    engine_mods::TICK_BUDGET
                )
            })
            .collect();
        if lines.is_empty() {
            "Aucun mod avec un script.".to_string()
        } else {
            lines.join("\n")
        }
    }

    pub(crate) fn console_help(&self) -> String {
        let mut out = String::from(
            "Commandes de la console :\n/help : cette liste\n/mods : ce que consomme chaque mod",
        );
        for (_, _, help) in GAME_COMMANDS {
            out.push('\n');
            out.push_str(help);
        }
        for (mod_id, name, help) in self.mods.commands() {
            out.push_str(&format!("\n/{name} : {help} ({mod_id})"));
        }
        out
    }
}

/// Les commandes du jeu ouvertes à la console, avec leur nom côté `ctl`.
pub(crate) const GAME_COMMANDS: &[(&str, &str, &str)] = &[
    (
        "give",
        "give",
        "/give <objet> [nombre] : se donner des objets",
    ),
    (
        "time",
        "time",
        "/time <0 à 1> : l'heure du jour (0,5 : midi)",
    ),
    (
        "weather",
        "weather",
        "/weather <clear|rain|storm> : le temps qu'il fait",
    ),
    (
        "summon",
        "summon",
        "/summon <créature> : faire apparaître une créature",
    ),
    ("tp", "goto", "/tp <x> <y> <z> : se téléporter"),
    (
        "travel",
        "travel",
        "/travel <dimension> : changer de dimension (le monde est rechargé)",
    ),
    ("xp", "xp", "/xp <points> : gagner de l'expérience"),
    (
        "break",
        "break",
        "/break <x> <y> <z> : casser un bloc, sans visée ni délai",
    ),
    (
        "place",
        "place",
        "/place <x> <y> <z> <bloc> : poser un bloc, sans visée",
    ),
];
