//! La console : on l'ouvre avec Entrée (ou `/`), on y tape `/commande arguments`, la réponse
//! s'affiche au-dessus de la barre d'objets. Échap la ferme.
//!
//! Les commandes viennent de trois endroits :
//! - **celles de la console** : `/help` (la liste) et `/mods` (ce que consomme chaque mod) ;
//! - **celles du jeu**, les mêmes que `cargo xtask ctl` : `/give`, `/time`, `/weather`,
//!   `/summon`, `/tp`, `/xp` ;
//! - **celles des mods** : un script déclare `/nom` avec `register_command` et répond dans
//!   `Mod::command`. Si deux mods ont la même commande, `/mod:nom` désigne la bonne.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use engine_core::Context;
use engine_core::winit::keyboard::KeyCode;
use engine_render::egui::{self, Color32, RichText};

use crate::VoxelGame;
use crate::control;
use crate::ui::Screen;

/// Les lignes gardées, et combien de temps elles restent affichées console fermée.
const KEPT_LINES: usize = 60;
const SHOWN_LINES: usize = 12;
const LINE_LIFETIME: Duration = Duration::from_secs(8);
/// Les commandes du jeu ouvertes à la console, avec leur nom côté `ctl`.
const GAME_COMMANDS: &[(&str, &str, &str)] = &[
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
];

#[derive(Debug, Default)]
pub struct ConsoleState {
    /// Ce que le joueur est en train de taper.
    pub input: String,
    /// Les lignes affichées (texte, heure), les plus récentes à la fin.
    pub lines: VecDeque<(String, Instant)>,
    /// Les commandes déjà tapées, et la position dans cette liste (flèches haut et bas).
    history: Vec<String>,
    history_at: Option<usize>,
    /// Les lignes validées, à exécuter à la prochaine mise à jour du jeu.
    pub submitted: Vec<String>,
}

impl VoxelGame {
    /// La console est ouverte (le jeu ne doit pas prendre les touches tapées pour des commandes).
    pub(crate) fn console_open(&self) -> bool {
        self.ui.inventory_open && self.ui.screen == Screen::Console
    }

    pub(crate) fn open_console(&mut self) {
        self.ui.screen = Screen::Console;
        self.ui.inventory_open = true;
        self.ui.console.input.clear();
        self.ui.console.history_at = None;
    }

    /// Ouvre la console sur Entrée ou `/`, et exécute les lignes validées.
    pub(crate) fn update_console(&mut self, ctx: &mut Context) {
        let free = self.mode == crate::Mode::Playing && !self.ui.inventory_open;
        if free && (ctx.input.key_pressed(KeyCode::Enter) || ctx.input.key_pressed(KeyCode::Slash))
        {
            self.open_console();
        }
        for line in std::mem::take(&mut self.ui.console.submitted) {
            self.run_console_line(ctx, &line);
        }
    }

    /// Ajoute des lignes à la console (une par `\n`).
    pub(crate) fn chat(&mut self, text: &str) {
        let console = &mut self.ui.console;
        for line in text.lines() {
            console.lines.push_back((line.to_string(), Instant::now()));
        }
        while console.lines.len() > KEPT_LINES {
            console.lines.pop_front();
        }
    }

    /// Exécute une ligne : `/commande arguments` (la barre oblique est facultative).
    pub(crate) fn run_console_line(&mut self, ctx: &mut Context, line: &str) {
        let line = line.trim().trim_start_matches('/').trim();
        if line.is_empty() {
            return;
        }
        self.chat(&format!("> /{line}"));
        let (name, args) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let (name, args) = (name.to_lowercase(), args.trim());
        let reply = match name.as_str() {
            "help" => self.console_help(),
            "mods" => self.console_mods(),
            _ => self.console_command(ctx, &name, args),
        };
        self.chat(&reply);
    }

    fn console_help(&self) -> String {
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

    fn console_mods(&self) -> String {
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

    /// Une commande du jeu ou d'un mod ; sinon une explication.
    fn console_command(&mut self, ctx: &mut Context, name: &str, args: &str) -> String {
        if let Some((_, ctl, _)) = GAME_COMMANDS.iter().find(|(n, _, _)| *n == name) {
            let line = format!("{ctl} {args}");
            return match control::parse(&line).and_then(|c| self.execute(ctx, c)) {
                Ok(_) => "fait".to_string(),
                Err(err) => format!("Erreur : {err:#}"),
            };
        }
        // Une commande de mod : `nom`, ou `mod:nom` pour lever une ambiguïté.
        let all = self.mods.commands();
        let (wanted_mod, wanted_name) = match name.split_once(':') {
            Some((m, n)) => (Some(m), n),
            None => (None, name),
        };
        let found = all
            .iter()
            .find(|(m, n, _)| n == wanted_name && wanted_mod.is_none_or(|w| w == m))
            .cloned();
        let Some((mod_id, command, _)) = found else {
            return format!("Commande inconnue : /{name} (voir /help)");
        };
        let (reply, actions) = self.mods.command(&mod_id, &command, args);
        self.apply_machine_actions(actions);
        reply.unwrap_or_else(|| "fait".to_string())
    }

    /// Les lignes récentes, au-dessus de la barre d'objets (console fermée).
    pub(crate) fn draw_chat(&mut self, ctx: &egui::Context) {
        if self.console_open() {
            return;
        }
        let recent: Vec<&str> = self
            .ui
            .console
            .lines
            .iter()
            .rev()
            .take(SHOWN_LINES)
            .filter(|(_, at)| at.elapsed() < LINE_LIFETIME)
            .map(|(text, _)| text.as_str())
            .collect();
        if recent.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::unique("chat"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -90.0))
            .interactable(false)
            .show(ctx, |ui| {
                for text in recent.iter().rev() {
                    ui.label(
                        RichText::new(*text)
                            .color(Color32::WHITE)
                            .background_color(Color32::from_black_alpha(140)),
                    );
                }
            });
    }

    /// La console ouverte : les dernières lignes et la ligne de saisie.
    pub(crate) fn draw_console(&mut self, ctx: &egui::Context) {
        egui::Window::new("Console")
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -90.0))
            .resizable(false)
            .collapsible(false)
            .title_bar(false)
            .show(ctx, |ui| {
                ui.set_width(680.0);
                for (text, _) in self.ui.console.lines.iter().rev().take(SHOWN_LINES).rev() {
                    ui.label(text.as_str());
                }
                let console = &mut self.ui.console;
                let response = ui.add(
                    egui::TextEdit::singleline(&mut console.input)
                        .hint_text("/commande (Échap : fermer)")
                        .desired_width(f32::INFINITY)
                        .lock_focus(true),
                );
                let (enter, up, down) = ctx.input(|i| {
                    (
                        i.key_pressed(egui::Key::Enter),
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                    )
                });
                if response.lost_focus() && enter {
                    let line = std::mem::take(&mut console.input);
                    if !line.trim().is_empty() {
                        console.history.push(line.clone());
                        console.submitted.push(line);
                    }
                    console.history_at = None;
                }
                // Les flèches parcourent les commandes déjà tapées.
                if (up || down) && !console.history.is_empty() {
                    let last = console.history.len() - 1;
                    let next = match (console.history_at, up) {
                        (None, true) => last,
                        (Some(i), true) => i.saturating_sub(1),
                        (Some(i), false) if i < last => i + 1,
                        _ => last,
                    };
                    console.history_at = Some(next);
                    console.input = console.history[next].clone();
                }
                response.request_focus();
            });
    }
}
