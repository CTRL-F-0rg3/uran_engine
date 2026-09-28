//! Główny punkt wejścia silnika: `App` spinające okno, pętlę zdarzeń,
//! systemy i renderer.

use std::sync::Arc;

use uran_asset::AssetServer;
use uran_core::{windowed, Input, Time, WindowDescriptor};
use uran_ecs::World;
use uran_render::{Camera2d, DrawList, Graphics, Renderer};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes};

use crate::context::{Ctx, SystemFn, WindowState, DEFAULT_ASSET_DIR};

/// Aplikacja — okno + świat ECS + systemy + renderer.
///
/// ```ignore
/// App::new()
///     .window(windowed(1280, 720).title("Moja gra"))
///     .add_startup_system(setup)
///     .add_system(update)
///     .run();
/// ```
pub struct App {
    descriptor: WindowDescriptor,
    asset_dir: String,
    /// Świat ECS dostępny przed uruchomieniem (np. do wczytania gry).
    pub world: World,
    systems: Vec<SystemFn>,
    startup_systems: Vec<SystemFn>,
    camera: Camera2d,
    font: Option<uran_asset::Handle<uran_asset::FontData>>,
    /// Pojemność symulacji GPU (ustawiana przed uruchomieniem pętli).
    gpu_sim_units: Option<usize>,
    screenshot: Option<(std::path::PathBuf, u32)>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            descriptor: windowed(1280, 720),
            asset_dir: DEFAULT_ASSET_DIR.to_string(),
            world: World::new(),
            systems: Vec::new(),
            startup_systems: Vec::new(),
            camera: Camera2d::new(),
            font: None,
            gpu_sim_units: None,
            screenshot: None,
        }
    }

    /// Konfiguracja okna.
    pub fn window(mut self, descriptor: WindowDescriptor) -> Self {
        self.descriptor = descriptor;
        self
    }

    /// System uruchamiany raz, po utworzeniu renderera (wczytywanie assetów,
    /// tworzenie świata).
    pub fn add_startup_system(mut self, system: impl FnMut(&mut Ctx) + Send + 'static) -> Self {
        self.startup_systems.push(Box::new(system));
        self
    }

    /// System uruchamiany w każdej klatce.
    ///
    /// Systemy działają w kolejności rejestracji — logika zależna od
    /// kolejności (ruch → kolizje → rysowanie) powinna być rozdzielona
    /// na osobne systemy w odpowiedniej kolejności.
    pub fn add_system(mut self, system: impl FnMut(&mut Ctx) + Send + 'static) -> Self {
        self.systems.push(Box::new(system));
        self
    }

    /// Katalog z assetami (domyślnie `assets/`).
    pub fn assets(mut self, path: impl Into<String>) -> Self {
        self.asset_dir = path.into();
        self
    }

    /// Domyślna kamera (gry mogą ją nadpisać w systemie).
    pub fn camera(mut self, camera: Camera2d) -> Self {
        self.camera = camera;
        self
    }

    /// Ustawia czcionkę HUD-u dostępną w `Ctx::font`.
    ///
    /// Wartość kopiujemy do stanu, bo systemy nie mają dostępu do
    /// `AssetServer` w trakcie rysowania.
    pub fn font(mut self, font: uran_asset::Handle<uran_asset::FontData>) -> Self {
        self.font = Some(font);
        self
    }

    /// Włącza symulację jednostek na GPU na `capacity` jednostek.
    ///
    /// Bufor alokowany jest raz; potem pozycje żyją już tylko na karcie.
    pub fn gpu_sim_units(mut self, capacity: usize) -> Self {
        self.gpu_sim_units = Some(capacity);
        self
    }

    /// Zapisze klatkę do pliku PNG po `frames` klatkach i zakończy aplikację.
    ///
    /// Przydatne do zautomatyzowanych testów graficznych i w CI.
    /// `path == None` oznacza: zrób zrzut tylko jeśli gra poprosi o to
    /// przez argument `--screenshot` (patrz `uran-game`).
    pub fn screenshot(mut self, path: Option<impl Into<std::path::PathBuf>>, frames: u32) -> Self {
        self.screenshot = path.map(Into::into).map(|p| (p, frames));
        self
    }

    /// Uruchamia aplikację — blokująco, aż do zamknięcia okna.
    pub fn run(self) {
        let event_loop = EventLoop::new().expect("nie udało się utworzyć pętli zdarzeń");
        event_loop.set_control_flow(ControlFlow::Poll);

        let clear_color = self.descriptor.clear_color;
        let mut state = AppState {
            descriptor: self.descriptor,
            assets: AssetServer::new(&self.asset_dir),
            world: self.world,
            systems: self.systems,
            startup_systems: self.startup_systems,
            camera: self.camera,
            window: None,
            renderer: None,
            draw_list: DrawList::new(),
            time: Time::new(),
            input: Input::new(),
            window_state: WindowState::default(),
            clear_color,
            rendering_enabled: true,
            startup_done: false,
            font: self.font,
            gpu_sim_units: self.gpu_sim_units,
            screenshot: self.screenshot,
            exit_after_frames: None,
            exit_requested: false,
        };

        event_loop
            .run_app(&mut state)
            .expect("pętla zdarzeń zakończyła się błędem");
    }
}

/// Stan wewnętrzny pętli aplikacji.
struct AppState {
    descriptor: WindowDescriptor,
    assets: AssetServer,
    world: World,
    systems: Vec<SystemFn>,
    startup_systems: Vec<SystemFn>,
    camera: Camera2d,

    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    draw_list: DrawList,

    time: Time,
    input: Input,
    window_state: WindowState,
    clear_color: uran_math::Color,
    rendering_enabled: bool,
    startup_done: bool,
    /// Czcionka HUD-u — współdzielona przez wszystkie systemy.
    pub font: Option<uran_asset::Handle<uran_asset::FontData>>,
    /// Pojemność symulacji GPU; używana raz przy starcie okna.
    pub gpu_sim_units: Option<usize>,
    /// Zgłoszenie zrzutu ekranu — po N klatkach zapisuje PNG i kończy grę.
    screenshot: Option<(std::path::PathBuf, u32)>,
    /// Ile klatek zostało do zamknięcia po zrzucie.
    exit_after_frames: Option<u32>,
    /// Czy pętla ma się zakończyć.
    exit_requested: bool,
}

impl AppState {
    /// Uruchamia systemy i renderuje jedną klatkę.
    fn tick(&mut self) {
        self.time.tick();

        // Kolejność jest istotna:
        //   1. czyścimy listę rysowania
        //   2. systemy startupowe (raz, po gotowym rendererze)
        //   3. systemy gry — w tym te, które RYSUJĄ
        //   4. dopisujemy encje z ECS
        //   5. renderujemy
        // Gdyby wyczyścić listę po systemach, skasowalibyśmy wszystko, co
        // właśnie narysowały.
        self.draw_list.clear();

        // Systemy startupowe (raz, po gotowym rendererze). Listy systemów
        // są chwilowo zabierane, bo `Ctx` pożycza pola `self`, a `systems`
        // też musi być pożyczone mutowalnie.
        if !self.startup_done {
            self.startup_done = true;
            let mut startup = std::mem::take(&mut self.startup_systems);
            self.with_ctx(&mut startup);
            self.startup_systems = startup;
        }

        let mut systems = std::mem::take(&mut self.systems);
        self.with_ctx(&mut systems);
        self.systems = systems;

        // encje z ECS dopisujemy na końcu (tryb „retained")
        self.draw_list.extract_world(&self.world, &self.assets);

        if self.rendering_enabled {
            if let Some(renderer) = &mut self.renderer {
                renderer.render(
                    &mut self.draw_list,
                    &self.camera,
                    self.clear_color,
                    &self.assets,
                );
            }
        }

        self.input.end_frame();

        // logowanie statystyk renderera (URAN_DEBUG=1)
        if std::env::var("URAN_DEBUG").is_ok() && self.time.frame() % 30 == 0 {
            if let Some(renderer) = &self.renderer {
                eprintln!(
                    "[uran] klatka {} | {} | elementów={}",
                    self.time.frame(),
                    renderer.stats,
                    self.draw_list.len()
                );
            }
        }

        // zrzut ekranu: po zadanej liczbie klatek zapisujemy PNG i kończymy
        if let Some((path, frames)) = self.screenshot.clone() {
            if self.time.frame() > frames as u64 {
                if let Some(renderer) = &mut self.renderer {
                    renderer.request_screenshot(path, 0);
                }
                self.screenshot = None;
                self.exit_after_frames = Some(1);
            }
        } else if let Some(remaining) = self.exit_after_frames {
            // klatka po zrzucie — można bezpiecznie zamknąć aplikację
            if remaining <= 1 {
                self.exit_requested = true;
            } else {
                self.exit_after_frames = Some(remaining - 1);
            }
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Buduje `Ctx` i przepuszcza przez niego systemy.
    ///
    /// `Ctx` musi być zbudowany **wewnątrz** tej funkcji (a nie w osobnej
    /// metodzie zwracającej `Ctx<'_>`), inaczej pożyczka obejmuje całe
    /// `self` zamiast poszczególnych pól i nie da się jednocześnie
    /// pożyczyć `self.systems`.
    fn with_ctx(&mut self, systems: &mut [SystemFn]) {
        // Symulator pożyczamy z renderera na czas systemów. Robimy to
        // przez `take`, bo `self.renderer` i `self.draw_list` muszą być
        // pożyczone w tym samym `Ctx`.
        let mut renderer = self.renderer.take();

        let mut ctx = Ctx {
            world: &mut self.world,
            gfx: Graphics::new(&mut self.draw_list),
            time: &self.time,
            input: &self.input,
            assets: &mut self.assets,
            window: self.window_state,
            camera: self.camera,
            clear_color: self.clear_color,
            rendering_enabled: self.rendering_enabled,
            frame: self.time.frame(),
            font: self.font,
            renderer: renderer.as_mut(),
        };

        for system in systems.iter_mut() {
            system(&mut ctx);
        }

        // systemy mogły zmienić kamerę, tło albo rozmiar okna — zapisujemy
        // to z powrotem (pożyczki na inne pola już tu wygasły)
        self.camera = ctx.camera;
        self.clear_color = ctx.clear_color;
        self.rendering_enabled = ctx.rendering_enabled;
        self.window_state = ctx.window;
        drop(ctx);
        // renderera oddajemy z powrotem, wraz z ewentualnymi zmianami symulacji
        self.renderer = renderer;
    }
}

impl ApplicationHandler for AppState {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = WindowAttributes::default()
            .with_title(&self.descriptor.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.descriptor.width,
                self.descriptor.height,
            ))
            .with_resizable(self.descriptor.resizable);

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("❌ nie udało się utworzyć okna: {e}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        self.window_state = WindowState {
            size: uran_math::Vec2::new(size.width as f32, size.height as f32),
            scale_factor: window.scale_factor() as f32,
        };

        if !self.descriptor.cursor_visible {
            window.set_cursor_visible(false);
        }

        let renderer = match pollster::block_on(Renderer::new(
            window.clone(),
            self.descriptor.vsync,
            self.descriptor.samples,
        )) {
            Ok(mut renderer) => {
                // Symulacja GPU alokuje bufory raz, przy starcie — dlatego
                // robimy to zanim pierwsza klatka przejdzie przez `tick`.
                if let Some(capacity) = gpu_sim_units {
                    renderer.enable_gpu_sim(capacity);
                    println!(
                        "🖥  symulacja GPU: bufor na {capacity} jednostek ({:.1} MB)",
                        capacity as f64 * std::mem::size_of::<uran_render::GpuUnit>() as f64
                            / (1024.0 * 1024.0)
                    );
                }
                Some(renderer)
            }
            Err(e) => {
                eprintln!("❌ inicjalizacja renderera nie powiodła się: {e}");
                event_loop.exit();
                return;
            }
        };

        // Na Wayland okno nie dostaje gwarantowanego `WindowEvent` zaraz po
        // utworzeniu, więc prosimy o pierwszą klatkę jawnie.
        window.request_redraw();
        self.renderer = renderer;
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::Resized(size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(*size);
                }
                self.window_state.size =
                    uran_math::Vec2::new(size.width as f32, size.height as f32);
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.window_state.scale_factor = *scale_factor as f32;
            }
            WindowEvent::RedrawRequested => {
                self.tick();
                if self.exit_requested {
                    event_loop.exit();
                    return;
                }
                return; // `tick` sam prosi o kolejną klatkę
            }
            // zdarzenia wejścia (klawiatura/mysz) — najpierw do `Input`
            _ => self.input.handle_event(&event),
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
