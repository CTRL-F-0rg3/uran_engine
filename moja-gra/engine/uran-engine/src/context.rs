//! Kontekst klatki — to, co każdy system dostaje do dyspozycji.

use uran_asset::{AssetServer, FontData, Handle, Image};
use uran_core::{Input, Time};
use uran_ecs::World;
use uran_math::Vec2;
use uran_render::{Camera2d, Graphics, PostFxSettings};
use winit::event::MouseButton;

/// Katalog domyślny dla assetów gry.
pub const DEFAULT_ASSET_DIR: &str = "assets";

/// Stan świata przekazywany systemom w każdej klatce.
pub struct Ctx<'a> {
    /// Świat ECS — encje, komponenty, systemy.
    pub world: &'a mut World,
    /// Rysowanie w trybie natychmiastowym (HUD, efekty, debug).
    pub gfx: Graphics<'a>,
    /// Czas: delta, czas skumulowany, FPS.
    pub time: &'a Time,
    /// Klawiatura i mysz.
    pub input: &'a Input,
    /// Katalog assetów (wczytywanie i tworzenie).
    pub assets: &'a mut AssetServer,
    /// Okno (rozmiar w fizycznych pikselach).
    pub window: WindowState,
    /// Kamera 2D (edytowalna z systemów).
    pub camera: Camera2d,
    /// Kolor tła na tę klatkę.
    pub clear_color: uran_math::Color,
    /// Czy render ma być wywoływany (przydatne przy pauzie).
    pub rendering_enabled: bool,
    /// Licznik klatek od startu (alias `time.frame()`).
    pub frame: u64,
    /// Czcionka do HUD-u (wspólna dla wszystkich systemów).
    pub font: Option<uran_asset::Handle<FontData>>,
    /// Renderer (pożyczany). Daje dostęp do symulacji GPU i kolejki.
    ///
    /// Pożyczamy cały renderer, a nie sam symulator, bo `wgpu::Queue`
    /// nie implementuje `Clone` — kolejki nie da się skopiować z
    /// wnętrza renderera, a `write_buffer` jest potrzebny przy wgrywaniu
    /// armii. Pozycje jednostek na CPU nie wgrywamy w klatce w ogóle.
    pub renderer: Option<&'a mut uran_render::Renderer>,
}

/// Rozmiar okna w jednostkach renderera.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WindowState {
    /// Rozmiar w fizycznych pikselach.
    pub size: Vec2,
    /// Skala DPI (1.0 = 96 DPI).
    pub scale_factor: f32,
}

impl WindowState {
    /// Rozmiar w punktach logicznych (do UI niezależnego od DPI).
    pub fn logical_size(&self) -> Vec2 {
        if self.scale_factor > 0.0 {
            self.size / self.scale_factor
        } else {
            self.size
        }
    }
}

impl<'a> Ctx<'a> {
    /// Wczytuje obraz (z cache) — zwraca `None`, gdy pliku nie ma.
    pub fn load_image(&mut self, path: &str) -> Option<Handle<Image>> {
        match self.assets.load_image(path) {
            Ok(handle) => Some(handle),
            Err(e) => {
                eprintln!("⚠️  nie udało się wczytać obrazu `{path}`: {e}");
                None
            }
        }
    }

    /// Wczytuje czcionkę (z cache).
    pub fn load_font(&mut self, path: &str) -> Option<Handle<FontData>> {
        match self.assets.load_font(path) {
            Ok(handle) => Some(handle),
            Err(e) => {
                eprintln!("⚠️  nie udało się wczytać czcionki `{path}`: {e}");
                None
            }
        }
    }

    /// Wczytuje mapę z pliku XML razem z arkuszami kafli.
    ///
    /// Ścieżka jest relatywna do katalogu assetów (`assets/`), więc
    /// `load_tilemap("maps/farm.xml")` szuka `assets/maps/farm.xml`.
    /// Błąd nie wywraca gry — zwraca `None` i wypisuje komunikat, bo
    /// brak pliku z mapą to błąd zawartości, nie powód do paniki.
    pub fn load_tilemap(&mut self, path: &str) -> Option<uran_tilemap::TileMap> {
        match uran_tilemap::xml::load_map(path, self.assets) {
            Ok(map) => {
                println!(
                    "🗺  mapa `{}`: {}x{} kafli, {} warstw, {} arkuszy",
                    map.name,
                    map.width(),
                    map.height(),
                    map.layers.len(),
                    map.tilesets.len()
                );
                Some(map)
            }
            Err(e) => {
                eprintln!("⚠️  nie udało się wczytać mapy `{path}`: {e}");
                None
            }
        }
    }

    /// Delta czasu w sekundach — skrót najczęstszej operacji w grze.
    pub fn dt(&self) -> f32 {
        self.time.delta_seconds()
    }

    /// Wskazuje pozycję myszy w świecie (używane przez dowodzenie armią).
    pub fn mouse_world(&self) -> uran_math::Vec2 {
        self.camera
            .screen_to_world(self.input.mouse_position(), self.window.size)
    }

    /// Ostatnie statystyki symulacji GPU (mogą być kilka klatek stare —
    /// wynikają z asynchronicznego odczytu liczników).
    pub fn sim_stats(&self) -> uran_render::SimStats {
        self.sim().map(|s| s.stats()).unwrap_or_default()
    }

    /// Symulator jednostek (gdy gra go używa).
    pub fn sim(&self) -> Option<&uran_render::GpuSim> {
        self.renderer.as_ref().and_then(|r| r.sim())
    }

    /// Symulator jednostek do modyfikacji parametrów.
    pub fn sim_mut(&mut self) -> Option<&mut uran_render::GpuSim> {
        self.renderer.as_mut().and_then(|r| r.sim_mut())
    }

    /// Kolejka GPU — potrzebna do jednorazowego wgrania buforów symulacji.
    ///
    /// W klatce nie wolno używać jej do przesyłania pozycji jednostek:
    /// po to mamy compute shader.
    pub fn queue(&self) -> Option<&wgpu::Queue> {
        self.renderer.as_ref().map(|r| r.queue())
    }

    /// Wskrzesza jednostki z rezerwy (mały zapis do bufora, nie cała armia).
    pub fn reinforce_range(
        &mut self,
        from: usize,
        to: usize,
        at: uran_math::Vec2,
        team: uran_render::Team,
    ) {
        if let Some(r) = self.renderer.as_mut() {
            r.revive_sim_range(from, to, at, team);
        }
    }

    /// Wgrywa armię na GPU (jednorazowo, przy starcie gry).
    ///
    /// Pozycje po tym wywołaniu już nigdy nie wracają na CPU.
    pub fn upload_army(&mut self, army: &[uran_render::GpuUnit]) {
        if let Some(r) = self.renderer.as_mut() {
            r.upload_sim_units(army);
        }
    }

    /// Czy przytrzymano lewy przycisk myszy (rozkaz formacji).
    pub fn mouse_held(&self) -> bool {
        self.input.mouse_pressed(MouseButton::Left)
    }

    /// Ustawia post-processing 2D (delikatne rybie oko).
    ///
    /// Skrót od `ctx.renderer.as_mut().map(|r| r.set_post_fx(s))` —
    /// wywołanie jest bezpieczne też wtedy, gdy renderer nie jest
    /// jeszcze podpięty (np. pierwsza klatka przed utworzeniem okna),
    /// po prostu nic się wtedy nie dzieje.
    ///
    /// ```ignore
    /// ctx.set_post_fx(PostFxSettings {
    ///     enabled: true,
    ///     fisheye: 0.08, // delikatny
    /// });
    /// ```
    pub fn set_post_fx(&mut self, settings: PostFxSettings) {
        if let Some(r) = self.renderer.as_mut() {
            r.set_post_fx(settings);
        }
    }
}

/// Sygnatura systemu gry: ma dostęp do świata i do kontekstu klatki.
pub type SystemFn = Box<dyn FnMut(&mut Ctx) + Send + 'static>;
