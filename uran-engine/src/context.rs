//! Kontekst klatki — to, co każdy system dostaje do dyspozycji.

use uran_asset::{AssetServer, FontData, Handle, Image};
use uran_core::{Input, Time};
use uran_ecs::World;
use uran_math::Vec2;
use uran_render::{Camera2d, Graphics};

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

    /// Delta czasu w sekundach — skrót najczęstszej operacji w grze.
    pub fn dt(&self) -> f32 {
        self.time.delta_seconds()
    }
}

/// Sygnatura systemu gry: ma dostęp do świata i do kontekstu klatki.
pub type SystemFn = Box<dyn FnMut(&mut Ctx) + Send + 'static>;
