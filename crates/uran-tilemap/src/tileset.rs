//! Arkusz kafli: opisuje, jak wyciąć pojedynczy kafel z tekstury.
//!
//! Kafel to **indeks wierszowy** liczony od lewej góry: `0` to lewy górny
//! róg, `1` — obok niego, a `cols` — pierwszy kafel w drugim rzędzie.
//! Numeracja jest taka jak w plikach graficznych i w większości formatów
//! tilemap, więc mapę można projektować w edytorze bez przeliczania
//! współrzędnych.
//!
//! ## Dlaczego własny typ zamiast surowego obrazu
//!
//! `Graphics::draw_texture` bierze `Rect` w pikselach i `UvRect`, czyli
//! graficzny kafel trzeba by było za każdym razem liczyć w grze. `Tileset`
//! robi to raz (`uv_for`) i dodaje resztę, której potrzebują prawdziwe mapy:
//! margines, odstęp między kaflami i **flipping**, dzięki któremu jeden plik
//! tekstury wystarcza na kilka wariantów kafla zamiast kopiowania go.

use uran_asset::{Handle, Image};
use uran_ecs::UvRect;
use uran_math::{Rect, Vec2};

/// Wariant kafla — pozwala odwrócić teksturę bez duplikowania wpisów
/// w mapie (np. podłoga płaska vs. podłoga odwrócona).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TileFlags(u8);

impl TileFlags {
    pub const NONE: Self = Self(0b00);
    pub const FLIP_X: Self = Self(0b01);
    pub const FLIP_Y: Self = Self(0b10);
    pub const FLIP_BOTH: Self = Self(0b11);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Surowy bajt flag (potrzebny do pakowania kafla w `u32`).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Wariant złożony z kilku flag naraz (np. `FLIP_X | FLIP_Y`).
    pub const fn combined(a: Self, b: Self) -> Self {
        Self(a.0 | b.0)
    }

    /// Odczytuje wariant z bajta flag (górne bity nie są zachowywane).
    ///
    /// Uwaga: poprawne warianty to `0b0000`..`0b0011`, dlatego wartość
    /// składowa nigdy nie przekracza `0b0011`.
    pub const fn from_bits_truncate(bits: u8) -> Self {
        Self(bits & 0b0011)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Obraca wariant o 90° w prawo.
    ///
    /// `NONE <-> FLIP_BOTH` oraz `FLIP_X <-> FLIP_Y`: obrót o 90° zamienia
    /// osie, więc odbicie poziome staje się pionowym. Dwa obroty dają ten sam
    /// wariant co zapis w mapie, a cztery wracają do `NONE`.
    pub const fn rotate_quarter(self) -> Self {
        match self.0 {
            0 => Self::FLIP_BOTH,
            1 => Self::FLIP_Y,
            2 => Self::FLIP_X,
            _ => Self::NONE,
        }
    }
}

/// Opis pojedynczego kafla wewnątrz arkusza.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TileRef {
    /// Kolumna w arkuszu (od lewej).
    pub col: u16,
    /// Wiersz w arkuszu (od góry).
    pub row: u16,
    /// Wariant (obroty/odwrócenia).
    pub flags: TileFlags,
}

impl TileRef {
    pub const fn new(col: u16, row: u16) -> Self {
        Self {
            col,
            row,
            flags: TileFlags::NONE,
        }
    }

    pub const fn with_flags(col: u16, row: u16, flags: TileFlags) -> Self {
        Self { col, row, flags }
    }

    /// Kafel o indeksie wierszowym (od lewej góry).
    pub fn from_index(index: u32, cols: u16) -> Self {
        Self {
            col: (index % cols as u32) as u16,
            row: (index / cols as u32) as u16,
            flags: TileFlags::NONE,
        }
    }

    /// Indeks wierszowy (od lewej góry).
    pub const fn index(self, cols: u16) -> u32 {
        self.row as u32 * cols as u32 + self.col as u32
    }
}

/// Arkusz kafli: tekstura + siatka kafli o stałym rozmiarze.
///
/// ```ignore
/// let ts = Tileset::new(texture, 16, Vec2::new(256.0, 256.0));
/// let uv = ts.uv_for(TileRef::new(2, 0)); // trzeci kafel w górnym rzędzie
/// gfx.draw_texture(texture, world_rect, uv);
/// ```
#[derive(Debug, Clone)]
pub struct Tileset {
    texture: Handle<Image>,
    /// Rozmiar kafla **w teksturze** (piksele obrazu).
    tile_size: u32,
    /// Margines wokół arkusza (piksele obrazu).
    margin: u32,
    /// Odstęp między kaflami (piksele obrazu).
    spacing: u32,
    cols: u16,
    rows: u16,
    /// Rozmiar obrazu źródłowego — potrzebny do przeliczenia UV.
    size: Vec2,
}

impl Tileset {
    /// Arkusz kafli `tile_size × tile_size` bez marginesu i odstępów.
    pub fn new(texture: Handle<Image>, tile_size: u32, size: Vec2) -> Self {
        let tile_size = tile_size.max(1);
        let cols = ((size.x as u32) / tile_size).max(1) as u16;
        let rows = ((size.y as u32) / tile_size).max(1) as u16;
        Self {
            texture,
            tile_size,
            margin: 0,
            spacing: 0,
            cols,
            rows,
            size,
        }
    }

    /// Arkusz z marginesem i odstępem między kaflami.
    ///
    /// Przydatne dla atlasów, w których kafle nie leżą bezpośrednio obok
    /// siebie — bez `spacing` nachodziłyby na siebie o połowę.
    pub fn with_layout(
        texture: Handle<Image>,
        tile_size: u32,
        size: Vec2,
        margin: u32,
        spacing: u32,
    ) -> Self {
        let tile_size = tile_size.max(1);
        // Wymiar arkusza liczymy z marginesem: dostępna szerokość to
        // `size.x - 2*margin`, a w niej mieści się `n` kafli i `n-1` odstępów.
        let usable = (size.x as i64 - 2 * margin as i64).max(1);
        let step = tile_size as i64 + spacing as i64;
        let cols = ((usable + spacing as i64) / step).max(1) as u16;
        let usable_y = (size.y as i64 - 2 * margin as i64).max(1);
        let rows = ((usable_y + spacing as i64) / step).max(1) as u16;
        Self {
            texture,
            tile_size,
            margin,
            spacing,
            cols,
            rows,
            size,
        }
    }

    pub fn texture(&self) -> Handle<Image> {
        self.texture
    }

    pub fn tile_size(&self) -> u32 {
        self.tile_size
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn size(&self) -> Vec2 {
        self.size
    }

    /// Liczba kafli w arkuszu.
    pub fn tile_count(&self) -> u32 {
        self.cols as u32 * self.rows as u32
    }

    /// Czy indeks mieści się w arkuszu.
    pub fn contains_index(&self, index: u32) -> bool {
        index < self.tile_count()
    }

    /// Czy kafel o podanych współrzędnych istnieje.
    pub fn contains(&self, tile: TileRef) -> bool {
        tile.col < self.cols && tile.row < self.rows
    }

    /// Prostokąt kafla w **pikselach obrazu** (lewy górny róg + rozmiar).
    pub fn pixel_rect(&self, tile: TileRef) -> Rect {
        let step = self.tile_size + self.spacing;
        let x = self.margin + tile.col as u32 * step;
        let y = self.margin + tile.row as u32 * step;
        Rect::from_xywh(
            x as f32,
            y as f32,
            self.tile_size as f32,
            self.tile_size as f32,
        )
    }

    /// UV kafla, gotowe do [`uran_render::Graphics::draw_texture`].
    ///
    /// `None`, gdy kafel wychodzi poza arkusz — lepiej pominąć taki kafel
    /// niż rysować kawałek sąsiedniego.
    pub fn uv_for(&self, tile: TileRef) -> Option<UvRect> {
        if !self.contains(tile) {
            return None;
        }
        Some(UvRect::from_pixels(self.pixel_rect(tile), self.size))
    }

    /// UV kafla o indeksie wierszowym.
    pub fn uv_for_index(&self, index: u32) -> Option<UvRect> {
        if !self.contains_index(index) {
            return None;
        }
        self.uv_for(TileRef::from_index(index, self.cols))
    }

    /// Rysuje kafel w prostokącie świata, respektując wariant z flag.
    pub fn draw(&self, gfx: &mut uran_render::Graphics<'_>, rect: Rect, tile: TileRef) {
        let Some(uv) = self.uv_for(tile) else {
            return;
        };
        // Obracamy sam `Rect` zamiast UV — wtedy próbkowanie tekstury idzie
        // w drugą stronę, ale pozycja kafla w świecie zostaje prawidłowa.
        let dst = match tile.flags.0 {
            1 => Rect::new(
                Vec2::new(rect.max.x, rect.min.y),
                Vec2::new(rect.min.x, rect.max.y),
            ),
            2 => Rect::new(
                Vec2::new(rect.min.x, rect.max.y),
                Vec2::new(rect.max.x, rect.min.y),
            ),
            3 => Rect::new(
                Vec2::new(rect.max.x, rect.max.y),
                Vec2::new(rect.min.x, rect.min.y),
            ),
            _ => rect,
        };
        gfx.draw_texture(self.texture, dst, uv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(cols: u16, rows: u16, tile: u32) -> Tileset {
        Tileset::new(
            Handle::new(0, 0),
            tile,
            Vec2::new(cols as f32 * tile as f32, rows as f32 * tile as f32),
        )
    }

    #[test]
    fn grid_is_derived_from_image_size() {
        let ts = sheet(16, 16, 16);
        assert_eq!((ts.cols(), ts.rows()), (16, 16));
        assert_eq!(ts.tile_count(), 256);
        assert_eq!(ts.tile_size(), 16);
    }

    #[test]
    fn non_divisible_size_does_not_overflow() {
        // Obraz 250x250 przy kaflu 16 -> 15 kolumn; ostatni kafel sięga
        // poza obraz i nie może zepsuć liczenia UV.
        let ts = Tileset::new(Handle::new(0, 0), 16, Vec2::new(250.0, 250.0));
        assert_eq!((ts.cols(), ts.rows()), (15, 15));
    }

    #[test]
    fn index_roundtrip() {
        let ts = sheet(16, 16, 16);
        let tile = TileRef::from_index(37, ts.cols());
        assert_eq!((tile.col, tile.row), (5, 2));
        assert_eq!(tile.index(ts.cols()), 37);
    }

    #[test]
    fn uv_covers_exact_pixel_cell() {
        let ts = sheet(16, 16, 16);
        // Kafel (1,0) to piksele x=16..32, y=0..16 -> UV 1/16..2/16.
        let uv = ts.uv_for(TileRef::new(1, 0)).unwrap();
        assert!((uv.min.x - 16.0 / 256.0).abs() < 1e-6);
        assert!((uv.max.x - 32.0 / 256.0).abs() < 1e-6);
        assert!(uv.min.y.abs() < 1e-6);
        assert!((uv.max.y - 16.0 / 256.0).abs() < 1e-6);
    }

    #[test]
    fn out_of_range_tiles_have_no_uv() {
        let ts = sheet(4, 4, 16);
        assert!(ts.uv_for(TileRef::new(4, 0)).is_none());
        assert!(ts.uv_for(TileRef::new(0, 4)).is_none());
        assert!(ts.uv_for_index(16).is_none());
        assert!(ts.uv_for_index(15).is_some());
    }

    #[test]
    fn margin_and_spacing_shift_pixels() {
        // 2x2 kafle 16 px, margines 1 px, odstęp 2 px: kafel (1,1) zaczyna
        // się w 1 + 1*(16+2) = 19 px.
        let ts = Tileset::with_layout(Handle::new(0, 0), 16, Vec2::new(52.0, 52.0), 1, 2);
        assert_eq!((ts.cols(), ts.rows()), (2, 2));
        let r = ts.pixel_rect(TileRef::new(1, 1));
        assert_eq!(r.min, Vec2::new(19.0, 19.0));
        assert_eq!(r.size(), Vec2::splat(16.0));
    }

    #[test]
    fn flags_rotate_quarter_turn() {
        // Obrót o 90° zamienia osie: X <-> Y, a NONE <-> FLIP_BOTH.
        assert_eq!(TileFlags::NONE.rotate_quarter(), TileFlags::FLIP_BOTH);
        assert_eq!(TileFlags::FLIP_BOTH.rotate_quarter(), TileFlags::NONE);
        assert_eq!(TileFlags::FLIP_X.rotate_quarter(), TileFlags::FLIP_Y);
        assert_eq!(TileFlags::FLIP_Y.rotate_quarter(), TileFlags::FLIP_X);
        // Cztery obroty tożsamość.
        let mut f = TileFlags::FLIP_X;
        for _ in 0..4 {
            f = f.rotate_quarter();
        }
        assert_eq!(f, TileFlags::FLIP_X);
        assert!(TileFlags::NONE.is_empty());
        assert!(TileFlags::FLIP_X.contains(TileFlags::FLIP_X));
        assert!(!TileFlags::NONE.contains(TileFlags::FLIP_X));
    }

    #[test]
    fn from_bits_truncate_drops_unknown_bits() {
        // Dolne 2 bity to wariant; wszystko powyżej jest ucinane.
        assert_eq!(TileFlags::from_bits_truncate(0b1111_0000), TileFlags::NONE);
        assert_eq!(
            TileFlags::from_bits_truncate(0b1111_0001),
            TileFlags::FLIP_X
        );
        assert_eq!(
            TileFlags::from_bits_truncate(0b1111_0011),
            TileFlags::FLIP_BOTH
        );
    }
}
