//! Arkusz klatek: siatka prostokątnych klatek w jednej teksturze.
//!
//! To odpowiednik `Tileset`, ale dla **postaci**: klatka może być
//! prostokątna (np. `64x128`), a nie kwadratowa.
//!
//! ```ignore
//! let sheet = SpriteSheet::new(texture, Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
//! assert_eq!(sheet.cols(), 8);
//! let uv = sheet.uv_for(0, 2).unwrap(); // trzecia klatka w trzecim rzędzie
//! gfx.draw_texture(sheet.texture(), Rect::from_center(pos, Vec2::new(64.0, 128.0)), uv);
//! ```

use uran_asset::{Handle, Image};
use uran_ecs::UvRect;
use uran_math::{Rect, Vec2};

/// Siatka klatek w teksturze: wszystkie klatki mają ten sam rozmiar.
#[derive(Debug, Clone)]
pub struct SpriteSheet {
    texture: Handle<Image>,
    /// Rozmiar jednej klatki w **pikselach obrazu**.
    frame_size: Vec2,
    /// Rozmiar obrazu źródłowego — potrzebny do przeliczenia UV.
    image_size: Vec2,
    cols: u16,
    rows: u16,
}

impl SpriteSheet {
    /// Arkusz bez marginesu i odstępów między klatkami.
    ///
    /// `frame_size` to **piksele obrazu**, `image_size` — rozmiar całej
    /// tekstury. Liczba kolumn i wierszy liczona jest z tych dwóch wartości.
    pub fn new(texture: Handle<Image>, frame_size: Vec2, image_size: Vec2) -> Self {
        let frame_size = frame_size.max(Vec2::splat(1.0));
        let cols = (image_size.x / frame_size.x).floor().max(1.0) as u16;
        let rows = (image_size.y / frame_size.y).floor().max(1.0) as u16;
        Self {
            texture,
            frame_size,
            image_size,
            cols,
            rows,
        }
    }

    /// Arkusz z jawnie podaną liczbą kolumn i wierszy.
    ///
    /// Przydatne, gdy obraz jest większy niż siatka klatek — wtedy `new()`
    /// policzyłoby za dużo kolumn.
    pub fn with_grid(
        texture: Handle<Image>,
        frame_size: Vec2,
        image_size: Vec2,
        cols: u16,
        rows: u16,
    ) -> Self {
        Self {
            texture,
            frame_size: frame_size.max(Vec2::splat(1.0)),
            image_size,
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    pub fn texture(&self) -> Handle<Image> {
        self.texture
    }

    /// Rozmiar klatki w pikselach obrazu.
    pub fn frame_size(&self) -> Vec2 {
        self.frame_size
    }

    /// Rozmiar klatki w jednostkach świata przy skali `1.0`.
    pub fn frame_world_size(&self) -> Vec2 {
        self.frame_size
    }

    pub fn image_size(&self) -> Vec2 {
        self.image_size
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// Łączna liczba klatek w arkuszu.
    pub fn frame_count(&self) -> u32 {
        self.cols as u32 * self.rows as u32
    }

    /// Czy współrzędne klatki mieszczą się w arkuszu.
    pub fn contains(&self, col: u16, row: u16) -> bool {
        col < self.cols && row < self.rows
    }

    /// Prostokąt klatki w **pikselach obrazu** (lewy górny róg + rozmiar).
    pub fn pixel_rect(&self, col: u16, row: u16) -> Rect {
        Rect::from_xywh(
            col as f32 * self.frame_size.x,
            row as f32 * self.frame_size.y,
            self.frame_size.x,
            self.frame_size.y,
        )
    }

    /// UV klatki, gotowe do `Graphics::draw_texture`.
    ///
    /// `None`, gdy klatka wychodzi poza arkusz — wtedy lepiej pominąć ją
    /// niż rysować kawałek sąsiedniej.
    pub fn uv_for(&self, col: u16, row: u16) -> Option<UvRect> {
        if !self.contains(col, row) {
            return None;
        }
        Some(UvRect::from_pixels(
            self.pixel_rect(col, row),
            self.image_size,
        ))
    }

    /// UV klatki po **indeksie liniowym**, czyli wierszami: 0, 1, 2... lecą
    /// po kolumnach pierwszego wiersza, potem drugiego itd.
    ///
    /// To indeksowanie dla **efektów** (zaklęcia, pociski, eksplozje), a nie
    /// dla postaci: arkusz postaci ma cztery wiersze = cztery kierunki, więc
    /// `uv_for(kolumna, wiersz)` jest właściwe. Arkusz efektu jest zwykle
    /// blokiem klatek jednej animacji, gdzie numer klatki ma być po prostu
    /// kolejnym numerem — bez wierszy kierunku.
    pub fn uv_for_flat(&self, index: u16) -> Option<UvRect> {
        if index as u32 >= self.frame_count() {
            return None;
        }
        let col = index % self.cols;
        let row = index / self.cols;
        Some(UvRect::from_pixels(
            self.pixel_rect(col, row),
            self.image_size,
        ))
    }

    /// Rysuje klatkę w prostokącie świata.
    pub fn draw(&self, gfx: &mut uran_render::Graphics<'_>, rect: Rect, col: u16, row: u16) {
        if let Some(uv) = self.uv_for(col, row) {
            gfx.draw_texture(self.texture, rect, uv);
        }
    }

    /// Rysuje klatkę o indeksie liniowym (jak [`SpriteSheet::uv_for_flat`]).
    pub fn draw_flat(&self, gfx: &mut uran_render::Graphics<'_>, rect: Rect, index: u16) {
        if let Some(uv) = self.uv_for_flat(index) {
            gfx.draw_texture(self.texture, rect, uv);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Zmienna `Handle` nie potrzebuje prawdziwego tekstury — do liczenia
    /// geometrii wystarczy dowolny uchwyt.
    fn handle() -> Handle<Image> {
        Handle::default()
    }

    /// Arkusz 512x512 z klatkami 64x128 to 8 kolumn i 4 wiersze.
    #[test]
    fn computes_grid_from_image_size() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        assert_eq!(sheet.cols(), 8);
        assert_eq!(sheet.rows(), 4);
        assert_eq!(sheet.frame_count(), 32);
    }

    /// Arkusz chodzenia ma 10 kolumn (640 px / 64), a idle i bieg tylko 8 —
    /// ten sam rozmiar klatki, różna liczba klatek w arkuszu.
    #[test]
    fn different_frame_counts_per_sheet() {
        let walk = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(640.0, 512.0));
        assert_eq!(walk.cols(), 10);
        assert_eq!(walk.rows(), 4);

        let idle = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        assert_eq!(idle.cols(), 8);
    }

    /// Prostokąt klatki liczy się od lewego górnego rogu — błędna podstawa
    /// przesuwa całą animację o jeden kafel.
    #[test]
    fn pixel_rect_is_row_major() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        assert_eq!(sheet.pixel_rect(0, 0).min, Vec2::new(0.0, 0.0));
        assert_eq!(sheet.pixel_rect(1, 0).min, Vec2::new(64.0, 0.0));

        // klatka z trzeciego wiersza zaczyna się pod dwoma pełnymi rzędami
        let next_row = sheet.pixel_rect(0, 2);
        assert_eq!(next_row.min, Vec2::new(0.0, 256.0));
        assert_eq!(next_row.size(), Vec2::new(64.0, 128.0));
    }

    /// UV klatki musi być znormalizowane (0..1) i mieścić się w obrazie.
    #[test]
    fn uv_is_normalized_and_inside_image() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        let uv = sheet.uv_for(7, 3).expect("klatka 7,3 mieści się w 8x4");
        assert!((uv.min.x - 0.875).abs() < 1e-5, "{}", uv.min.x);
        assert!((uv.min.y - 0.75).abs() < 1e-5, "{}", uv.min.y);
        assert!((uv.max.x - 1.0).abs() < 1e-5);
        assert!((uv.max.y - 1.0).abs() < 1e-5);
    }

    /// Klatka poza arkuszem to `None`, a nie zawinięta klatka z sąsiedniej
    /// kolumny — inaczej animacja pokazywałaby obcą grafikę.
    #[test]
    fn out_of_bounds_returns_none() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        assert!(sheet.uv_for(8, 0).is_none(), "kolumna 8 nie istnieje");
        assert!(sheet.uv_for(0, 4).is_none(), "wiersz 4 nie istnieje");
        assert!(sheet.uv_for(7, 3).is_some());
    }

    /// Rozmiar klatki nie może być zerowy — dzieliłby rozmiar obrazu.
    #[test]
    fn zero_frame_size_is_clamped() {
        let sheet = SpriteSheet::new(handle(), Vec2::ZERO, Vec2::new(512.0, 512.0));
        assert!(sheet.frame_size().x >= 1.0 && sheet.frame_size().y >= 1.0);
        assert_eq!(sheet.cols(), 512);
    }

    /// `with_grid` ma pierwszeństwo nad wyliczaniem z rozmiaru obrazu.
    #[test]
    fn explicit_grid_overrides_image_size() {
        let sheet = SpriteSheet::with_grid(
            handle(),
            Vec2::new(64.0, 128.0),
            Vec2::new(640.0, 512.0),
            8,
            4,
        );
        assert_eq!(sheet.cols(), 8);
        assert!(sheet.uv_for(8, 0).is_none());
        assert!(sheet.uv_for(7, 3).is_some());
    }

    /// Indeks liniowy idzie **wierszami**: klatka 5 w arkuszu 5x4 to pierwsza
    /// klatka drugiego wiersza, a nie szósta kolumna (której nie ma).
    #[test]
    fn flat_index_goes_row_major() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(192.0, 192.0), Vec2::new(960.0, 768.0));
        assert_eq!(sheet.cols(), 5);
        assert_eq!(sheet.rows(), 4);
        assert_eq!(sheet.frame_count(), 20);

        assert_eq!(
            sheet.uv_for_flat(0),
            sheet.uv_for(0, 0),
            "klatka 0 to początek arkusza"
        );
        assert_eq!(sheet.uv_for_flat(4), sheet.uv_for(4, 0));
        assert_eq!(
            sheet.uv_for_flat(5),
            sheet.uv_for(0, 1),
            "po 5 klatkach zaczynamy drugi wiersz"
        );
        assert_eq!(sheet.uv_for_flat(19), sheet.uv_for(4, 3), "ostatnia klatka");
    }

    /// Klatka liniowa spoza arkusza to `None`, a nie zawinięta klatka
    /// z początku — inaczej efekt zapęliłby się zamiast zakończyć.
    #[test]
    fn flat_index_out_of_bounds_returns_none() {
        let sheet = SpriteSheet::new(handle(), Vec2::new(192.0, 192.0), Vec2::new(960.0, 768.0));
        assert!(sheet.uv_for_flat(20).is_none(), "20 klatek to już koniec");
        assert!(sheet.uv_for_flat(21).is_none());
        assert!(sheet.uv_for_flat(19).is_some());
    }
}
