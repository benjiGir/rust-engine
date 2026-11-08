//! La boucle principale : crée la fenêtre, distribue les événements,
//! appelle le jeu à chaque tick et à chaque image.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::{DeviceEvent, DeviceId, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Window, WindowId};

use crate::input::Input;
use crate::time::FixedTimestep;

/// Réglages de départ de l'application.
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// Fréquence de `Game::fixed_update`.
    pub ticks_per_second: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            title: "rust-game-engine".into(),
            width: 1280,
            height: 720,
            ticks_per_second: 60,
        }
    }
}

/// Ce que le moteur prête au jeu à chaque appel.
pub struct Context {
    /// `Arc` : la fenêtre est partagée entre la boucle et le renderer.
    pub window: Arc<Window>,
    pub input: Input,
    /// Durée d'un tick de `fixed_update`, en secondes.
    pub fixed_dt: f32,
    /// Texte libre ajouté au titre de la fenêtre, après les FPS (ex. « 120 chunks »).
    pub status: String,
    exit_requested: bool,
}

impl Context {
    /// Demande la fermeture propre de l'application.
    pub fn exit(&mut self) {
        self.exit_requested = true;
    }

    /// Capture la souris (vue FPS) ou la libère.
    pub fn set_cursor_captured(&self, captured: bool) {
        if captured {
            // macOS ne gère que `Locked`, Windows préfère `Confined` : on essaie l'un puis l'autre.
            let grabbed = self
                .window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(err) = grabbed {
                log::warn!("impossible de capturer la souris : {err}");
            }
        } else if let Err(err) = self.window.set_cursor_grab(CursorGrabMode::None) {
            log::warn!("impossible de libérer la souris : {err}");
        }
        self.window.set_cursor_visible(!captured);
    }
}

/// Le contrat entre le moteur et un jeu. Seuls `new` et `render` sont obligatoires.
///
/// `Sized` est requis parce que `new` renvoie `Self` par valeur.
pub trait Game: Sized {
    /// Appelé une fois, quand la fenêtre existe. C'est ici qu'on crée le renderer.
    fn new(ctx: &mut Context) -> Result<Self>;

    /// Logique à fréquence fixe (physique, simulation). Comme `FixedUpdate`.
    fn fixed_update(&mut self, _ctx: &mut Context) {}

    /// Une fois par image, avec la durée réelle de la frame en secondes. Comme `Update`.
    /// C'est ici qu'on lit `key_pressed`, qui ne dure qu'une frame.
    fn update(&mut self, _ctx: &mut Context, _dt: f32) {}

    /// Dessine l'image. `alpha` permet d'interpoler entre deux ticks.
    fn render(&mut self, ctx: &mut Context, alpha: f32);

    /// La fenêtre a changé de taille (en pixels physiques).
    fn resized(&mut self, _ctx: &mut Context, _size: PhysicalSize<u32>) {}

    /// Chaque événement brut de la fenêtre, avant que le moteur ne le traite. Utile pour
    /// une interface (egui) qui a besoin des clics, du texte, de la molette...
    /// Renvoyer `true` signifie « consommé » : le moteur ne le compte pas comme une
    /// entrée du jeu (un clic sur un bouton ne casse pas un bloc derrière).
    fn window_event(&mut self, _ctx: &mut Context, _event: &WindowEvent) -> bool {
        false
    }

    /// Appelé une fois juste avant de quitter (fenêtre fermée ou `ctx.exit()`).
    /// C'est le moment de sauvegarder.
    fn exit(&mut self, _ctx: &mut Context) {}
}

/// Lance le jeu `G` et ne rend la main qu'à la fermeture de la fenêtre.
pub fn run<G: Game>(config: AppConfig) -> Result<()> {
    let event_loop = EventLoop::new()?;
    // `Poll` : on redessine en continu, comme tout jeu, au lieu d'attendre des événements.
    event_loop.set_control_flow(ControlFlow::Poll);

    let mut app = App::<G> {
        config,
        running: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;

    // Une erreur survenue dans un callback est remontée ici.
    match app.error {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

/// État interne de la boucle. `G` est générique : le compilateur génère
/// une version de cette boucle spécialisée pour chaque jeu (monomorphisation).
struct App<G: Game> {
    config: AppConfig,
    /// `None` tant que la fenêtre n'est pas créée (elle l'est dans `resumed`).
    running: Option<Running<G>>,
    error: Option<anyhow::Error>,
}

struct Running<G> {
    ctx: Context,
    game: G,
    timestep: FixedTimestep,
    last_frame: Instant,
    fps: FpsCounter,
}

impl<G: Game> App<G> {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        let attributes = Window::default_attributes()
            .with_title(&self.config.title)
            .with_inner_size(LogicalSize::new(self.config.width, self.config.height));
        let window = Arc::new(event_loop.create_window(attributes)?);

        let timestep = FixedTimestep::new(self.config.ticks_per_second);
        let mut ctx = Context {
            window,
            input: Input::default(),
            fixed_dt: timestep.step().as_secs_f32(),
            status: String::new(),
            exit_requested: false,
        };
        let game = G::new(&mut ctx)?;

        self.running = Some(Running {
            ctx,
            game,
            timestep,
            last_frame: Instant::now(),
            fps: FpsCounter::new(),
        });
        Ok(())
    }
}

impl<G: Game> ApplicationHandler for App<G> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        if let Err(err) = self.start(event_loop) {
            self.error = Some(err);
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // `let ... else` : si le jeu n'a pas démarré, il n'y a rien à faire.
        let Some(running) = self.running.as_mut() else {
            return;
        };
        let Running { ctx, game, .. } = running;
        let consumed = game.window_event(ctx, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => game.resized(ctx, size),
            WindowEvent::Focused(false) => {
                ctx.input.clear();
                ctx.set_cursor_captured(false);
            }
            WindowEvent::KeyboardInput { event, .. } if !consumed => {
                // On ignore la répétition automatique du clavier.
                if let PhysicalKey::Code(code) = event.physical_key
                    && !event.repeat
                {
                    ctx.input.on_key(code, event.state.is_pressed());
                }
            }
            WindowEvent::MouseInput { state, button, .. } if !consumed => {
                ctx.input.on_mouse_button(button, state.is_pressed());
            }
            WindowEvent::RedrawRequested => running.frame(&self.config.title),
            _ => {}
        }

        if running.ctx.exit_requested {
            event_loop.exit();
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        // Le mouvement « brut » de la souris continue même quand le curseur est bloqué.
        if let (Some(running), DeviceEvent::MouseMotion { delta }) = (self.running.as_mut(), event)
        {
            running.ctx.input.on_mouse_motion(delta.0, delta.1);
        }
    }

    /// winit nous prévient que la boucle se termine : dernier appel au jeu.
    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(running) = self.running.as_mut() {
            running.game.exit(&mut running.ctx);
        }
    }

    fn about_to_wait(&mut self, _: &ActiveEventLoop) {
        if let Some(running) = &self.running {
            running.ctx.window.request_redraw();
        }
    }
}

impl<G: Game> Running<G> {
    /// Une image : ticks fixes, puis mise à jour libre, puis rendu.
    fn frame(&mut self, title: &str) {
        let now = Instant::now();
        let frame_time = now - self.last_frame;
        self.last_frame = now;

        for _ in 0..self.timestep.advance(frame_time) {
            self.game.fixed_update(&mut self.ctx);
        }
        self.game.update(&mut self.ctx, frame_time.as_secs_f32());
        self.game.render(&mut self.ctx, self.timestep.alpha());
        self.ctx.input.end_frame();

        if let Some((fps, ms)) = self.fps.tick(frame_time) {
            let mut text = format!("{title} | {fps:.0} FPS | {ms:.2} ms");
            if !self.ctx.status.is_empty() {
                text += " | ";
                text += &self.ctx.status;
            }
            // Aussi dans les logs : `RUST_LOG=engine_core=debug` pour suivre les FPS.
            log::debug!("{text}");
            self.ctx.window.set_title(&text);
        }
    }
}

/// Moyenne des FPS sur une demi-seconde, pour un affichage lisible.
struct FpsCounter {
    frames: u32,
    elapsed: Duration,
}

impl FpsCounter {
    fn new() -> Self {
        Self {
            frames: 0,
            elapsed: Duration::ZERO,
        }
    }

    /// Renvoie `Some((fps, ms par frame))` toutes les 0,5 s.
    fn tick(&mut self, frame_time: Duration) -> Option<(f32, f32)> {
        self.frames += 1;
        self.elapsed += frame_time;
        if self.elapsed < Duration::from_millis(500) {
            return None;
        }
        let secs = self.elapsed.as_secs_f32();
        let result = (
            self.frames as f32 / secs,
            secs * 1000.0 / self.frames as f32,
        );
        self.frames = 0;
        self.elapsed = Duration::ZERO;
        Some(result)
    }
}
