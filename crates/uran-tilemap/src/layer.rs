//! Warstwa kafli: prostokątna siatka indeksów do arkusza.
//!
//! Warstwa to **najmniejsza jednostka mapy**, którą da się narysować jako
//! jeden przebieg: ma swoją teksturę, własny rozmiar kafla w świecie i
//! kolejność rysowania. Dzięki temu podłoga, dekoracje i rośliny mogą być
//! osobnymi warstwami, a gracz decyduje, którą z nich edytuje.
//!
//! ## Oś Y w górę
//!
//! Wiersz `0` jest na **dole** i indeks rośnie w górę — zgodnie z resztą
//! silnika (`Rect::min`/`max`, kamera z osią Y w górę). Odwrócenie tej osi
//! to klasyczny błąd, który objawia się grawitacją w górę.

use uran_math::Rect;

use crate::tileset::{TileFlags, TileRef, Tileset};

/// Wartość oznaczająca „tu nic nie ma" (kafel przezroczysty).
pub const TILE_EMPTY: u32 = u32::MAX;

/// Maska bitów zarezerwowanych na wariant (flipping) kafla.
///
/// Dolne 28 bitów trzyma indeks w arkuszu, górne 4 — flagi. Dzięki temu
/// odwrócenie kafla nie wymaga osobnej warstwy ani duplikowania wpisów.
const FLAGS_SHIFT: u32 = 28;
/// Maska indeksu kafla (bez bitów wariantu).
pub const INDEX_MASK: u32 = (1 << FLAGS_SHIFT) - 1;
/// Maska bitów wariantu.
pub const FLAGS_MASK: u32 = !INDEX_MASK;

/// Pakuje indeks kafla wraz z wariantem w jedną `u32`.
pub fn pack_tile(index: u32, flags: TileFlags) -> u32 {
    debug_assert!(
        index <= INDEX_MASK,
        "indeks kafla nie mieści się w 28 bitach"
    );
    index | ((flags.bits() as u32) << FLAGS_SHIFT)
}

/// Wyodrębnia wariant z zapakowanej wartości kafla.
pub fn flags_from_packed(packed: u32) -> TileFlags {
    TileFlags::from_bits_truncate((packed >> FLAGS_SHIFT) as u8)
}

/// Wyodrębnia indeks z zapakowanej wartości kafla.
pub fn index_from_packed(packed: u32) -> u32 {
    packed & INDEX_MASK
}

/// Pojedyncza warstwa kafli: siatka `width × height` indeksów.
#[derive(Debug, Clone, PartialEq)]
pub struct TileLayer {
    /// Nazwa warstwy (do XML i diagnostyki).
    pub name: String,
    width: u32,
    height: u32,
    /// Indeksy kafli; `TILE_EMPTY` = brak kafla.
    tiles: Vec<u32>,
    /// Warstwa ukryta — rysowana jest pomijana (przydatne w edytorze).
    pub hidden: bool,
}

impl TileLayer {
    /// Nowa warstwa wypełniona pustymi kaflami.
    pub fn new(name: impl Into<String>, width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize);
        Self {
            name: name.into(),
            width,
            height,
            tiles: vec![TILE_EMPTY; len],
            hidden: false,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Czy (x, y) mieści się w siatce.
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && x < self.width as i32 && y < self.height as i32
    }

    /// Kafel na pozycji (x, y); `TILE_EMPTY` poza mapą.
    ///
    /// Poza mapą zwracamy `TILE_EMPTY`, a nie błąd, bo „nie ma kafla" i
    /// „jest pusty kafel" dają w grze ten sam efekt: brak kolizji.
    pub fn get(&self, x: i32, y: i32) -> u32 {
        if !self.in_bounds(x, y) {
            return TILE_EMPTY;
        }
        self.tiles[y as usize * self.width as usize + x as usize]
    }

    /// Ustawia kafel (x, y) i zwraca poprzednią wartość.
    pub fn set(&mut self, x: i32, y: i32, tile: u32) -> u32 {
        if !self.in_bounds(x, y) {
            return TILE_EMPTY;
        }
        let index = y as usize * self.width as usize + x as usize;
        std::mem::replace(&mut self.tiles[index], tile)
    }

    /// Wypełnia prostokąt kaflami (poza mapą pomija).
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, tile: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                self.set(xx, yy, tile);
            }
        }
    }

    /// Surowy bufor indeksów (do serializacji i szybkiego zapisu).
    pub fn tiles(&self) -> &[u32] {
        &self.tiles
    }

    /// Podmienia cały bufor — pozycje spoza `width × height` są obcinane,
    /// a brakujące uzupełniane `TILE_EMPTY`.
    pub fn set_tiles(&mut self, mut tiles: Vec<u32>) {
        let len = (self.width as usize) * (self.height as usize);
        tiles.resize(len, TILE_EMPTY);
        self.tiles = tiles;
    }

    /// Zmienia rozmiar warstwy, zachowując zawartość w obszarze wspólnym.
    pub fn resize(&mut self, width: u32, height: u32) {
        let mut next = vec![TILE_EMPTY; (width as usize) * (height as usize)];
        let w = width.min(self.width) as usize;
        let h = height.min(self.height) as usize;
        for y in 0..h {
            for x in 0..w {
                next[y * width as usize + x] = self.tiles[y * self.width as usize + x];
            }
        }
        self.width = width;
        self.height = height;
        self.tiles = next;
    }

    /// Prostokąt świata obejmujący całą warstwę.
    pub fn world_rect(&self, tile_size: f32) -> Rect {
        Rect::from_xywh(
            0.0,
            0.0,
            self.width as f32 * tile_size,
            self.height as f32 * tile_size,
        )
    }

    /// Prostokąt świata kafla o współrzędnych (x, y).
    pub fn cell_rect(x: i32, y: i32, tile_size: f32) -> Rect {
        Rect::from_xywh(
            x as f32 * tile_size,
            y as f32 * tile_size,
            tile_size,
            tile_size,
        )
    }

    /// Kafel jako [`TileRef`] z wariantem zakodowanym w górnych bitach.
    ///
    /// `None`, gdy indeks wychodzi poza arkusz — wtedy lepiej pominąć kafel
    /// niż rysować kawałek sąsiedniego.
    fn tile_ref(&self, tileset: &Tileset, packed: u32) -> Option<TileRef> {
        let index = index_from_packed(packed);
        if index >= tileset.tile_count() {
            return None;
        }
        Some(TileRef::with_flags(
            (index % tileset.cols() as u32) as u16,
            (index / tileset.cols() as u32) as u16,
            flags_from_packed(packed),
        ))
    }

    /// Kafle widoczne w prostokącie `view` (z marginesem 1 kafla).
    ///
    /// Margines chroni przed „migotaniem" kafli na krawędzi kadru: gdyby
    /// ciąć dokładnie po widocznym prostokącie, kafel o niecałkowitym
    /// położeniu znikałby i pojawiał się znowu przy każdym przesunięciu
    /// kamery o ułamek piksela.
    pub fn visible_tiles(&self, view: Rect, tile_size: f32) -> Vec<(i32, i32, u32)> {
        let mut out = Vec::new();
        if tile_size <= 0.0 {
            return out;
        }
        let x0 = (view.min.x / tile_size).floor() as i32 - 1;
        let x1 = (view.max.x / tile_size).ceil() as i32 + 1;
        let y0 = (view.min.y / tile_size).floor() as i32 - 1;
        let y1 = (view.max.y / tile_size).ceil() as i32 + 1;
        for y in y0.max(0)..y1.min(self.height as i32) {
            for x in x0.max(0)..x1.min(self.width as i32) {
                let t = self.get(x, y);
                if t != TILE_EMPTY {
                    out.push((x, y, t));
                }
            }
        }
        out
    }

    /// Rysuje widoczną część warstwy, pomijając kafle poza kadrem.
    ///
    /// Culling jest tu nie optymalizacją, a **koniecznością**: mapa 200×200
    /// to 40 000 kafli, czyli 40 000 draw calli w jednej klatce, gdyby
    /// rysować wszystkie.
    pub fn draw(
        &self,
        gfx: &mut uran_render::Graphics<'_>,
        tileset: &Tileset,
        tile_size: f32,
        view: Rect,
    ) {
        if self.hidden {
            return;
        }
        for (x, y, packed) in self.visible_tiles(view, tile_size) {
            if let Some(tile) = self.tile_ref(tileset, packed) {
                tileset.draw(gfx, Self::cell_rect(x, y, tile_size), tile);
            }
        }
    }

    /// Czy prostokąt świata styka się z jakimś niepustym kaflem.
    ///
    /// Używane przez kolizje: gracz nie powinien wchodzić w kafle ścian,
    /// domu czy drzew, ale przechodzić nad pustymi polami.
    pub fn overlaps_any(&self, r: Rect, tile_size: f32) -> bool {
        self.overlaps_where(r, tile_size, |t| t != TILE_EMPTY)
    }

    /// Czy prostokąt styka się z kaflem spełniającym warunek `predicate`.
    ///
    /// Iterujemy po komórkach, które prostokąt faktycznie zajmuje, więc
    /// koszt zależy od rozmiaru obiektu, a nie od rozmiaru mapy.
    pub fn overlaps_where(&self, r: Rect, tile_size: f32, predicate: impl Fn(u32) -> bool) -> bool {
        if tile_size <= 0.0 || r.is_empty() {
            return false;
        }
        let x0 = (r.min.x / tile_size).floor() as i32;
        let x1 = ((r.max.x - f32::EPSILON) / tile_size).floor() as i32;
        let y0 = (r.min.y / tile_size).floor() as i32;
        let y1 = ((r.max.y - f32::EPSILON) / tile_size).floor() as i32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                if predicate(self.get(x, y)) {
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tileset::{TileFlags, Tileset};
    use uran_asset::Handle;
    use uran_math::Vec2;

    fn sheet() -> Tileset {
        Tileset::new(Handle::new(0, 0), 16, Vec2::new(64.0, 64.0))
    }

    #[test]
    fn new_layer_is_empty() {
        let l = TileLayer::new("t", 3, 4);
        assert_eq!((l.width(), l.height()), (3, 4));
        assert_eq!(l.get(0, 0), TILE_EMPTY);
        assert_eq!(l.tiles().len(), 12);
    }

    #[test]
    fn get_and_set_roundtrip() {
        let mut l = TileLayer::new("t", 4, 4);
        assert_eq!(l.set(2, 1, 7), TILE_EMPTY, "poprzedni kafel był pusty");
        assert_eq!(l.get(2, 1), 7);
        assert_eq!(l.set(2, 1, 9), 7, "set zwraca poprzednią wartość");
        assert_eq!(l.get(2, 1), 9);
    }

    #[test]
    fn out_of_bounds_reads_and_writes_are_safe() {
        let mut l = TileLayer::new("t", 2, 2);
        assert_eq!(l.get(-1, 0), TILE_EMPTY);
        assert_eq!(l.get(0, 5), TILE_EMPTY);
        assert!(!l.in_bounds(-1, 0));
        // Zapis poza mapą nie może wysadzić indeksu.
        assert_eq!(l.set(-5, -5, 1), TILE_EMPTY);
        assert!(l.tiles().iter().all(|&t| t == TILE_EMPTY));
    }

    #[test]
    fn fill_rect_covers_the_area() {
        let mut l = TileLayer::new("t", 6, 6);
        l.fill_rect(1, 1, 2, 2, 4);
        assert_eq!(l.get(1, 1), 4);
        assert_eq!(l.get(2, 2), 4);
        assert_eq!(l.get(3, 2), TILE_EMPTY, "poza prostokątem pusto");
    }

    #[test]
    fn resize_keeps_overlapping_content() {
        let mut l = TileLayer::new("t", 4, 4);
        l.set(1, 1, 5);
        l.resize(2, 2);
        assert_eq!(l.get(1, 1), 5, "zawartość wspólna zostaje");
        assert_eq!(l.tiles().len(), 4);

        l.set(0, 0, 6);
        l.resize(4, 4);
        assert_eq!(l.get(1, 1), 5);
        assert_eq!(l.get(3, 3), TILE_EMPTY, "nowe kafle są puste");
    }

    #[test]
    fn world_rect_and_cell_rect_agree() {
        let l = TileLayer::new("t", 3, 2);
        assert_eq!(l.world_rect(16.0), Rect::from_xywh(0.0, 0.0, 48.0, 32.0));
        assert_eq!(
            TileLayer::cell_rect(1, 1, 16.0),
            Rect::from_xywh(16.0, 16.0, 16.0, 16.0)
        );
    }

    #[test]
    fn visible_tiles_skips_empty_and_outside_view() {
        let mut l = TileLayer::new("t", 8, 8);
        l.set(0, 0, 1);
        l.set(7, 7, 2);
        // Widok obejmuje tylko kafel (0,0).
        let view = Rect::from_xywh(-16.0, -16.0, 32.0, 32.0);
        let tiles = l.visible_tiles(view, 16.0);
        assert!(tiles.iter().any(|&(x, y, _)| x == 0 && y == 0));
        assert!(!tiles.iter().any(|&(x, y, _)| x == 7 && y == 7));
    }

    #[test]
    fn visible_tiles_has_one_cell_margin() {
        let mut l = TileLayer::new("t", 8, 8);
        // Kafel (1,0) zaczyna się dokładnie na prawej krawędzi kadru
        // (16 px = koniec widoku), więc bez marginesu byłby cięty.
        l.set(1, 0, 3);
        let view = Rect::from_xywh(0.0, 0.0, 16.0, 16.0);
        let tiles = l.visible_tiles(view, 16.0);
        assert!(
            tiles.iter().any(|&(x, _, _)| x == 1),
            "margines chroni przed migotaniem kafli na krawędzi kadru"
        );
    }

    #[test]
    fn zero_tile_size_returns_nothing() {
        let mut l = TileLayer::new("t", 4, 4);
        l.fill_rect(0, 0, 4, 4, 1);
        let view = Rect::from_xywh(0.0, 0.0, 64.0, 64.0);
        assert!(l.visible_tiles(view, 0.0).is_empty());
    }

    #[test]
    fn overlaps_any_detects_touched_cells() {
        let mut l = TileLayer::new("t", 8, 8);
        l.set(3, 3, 1);
        // Kafel (3,3) zajmuje 48..64 w obu osiach.
        assert!(l.overlaps_any(Rect::from_xywh(50.0, 50.0, 4.0, 4.0), 16.0));
        assert!(!l.overlaps_any(Rect::from_xywh(10.0, 10.0, 4.0, 4.0), 16.0));
    }

    #[test]
    fn overlaps_where_uses_predicate() {
        let mut l = TileLayer::new("t", 8, 8);
        l.set(1, 1, 3);
        let r = Rect::from_xywh(17.0, 17.0, 4.0, 4.0);
        assert!(l.overlaps_where(r, 16.0, |t| t == 3));
        assert!(!l.overlaps_where(r, 16.0, |t| t == 99));
    }

    #[test]
    fn pack_and_unpack_flags() {
        let packed = pack_tile(37, TileFlags::FLIP_X);
        assert_eq!(index_from_packed(packed), 37);
        assert_eq!(flags_from_packed(packed), TileFlags::FLIP_X);
        // Bity wariantu nie psują indeksu.
        assert_eq!(index_from_packed(pack_tile(1, TileFlags::FLIP_BOTH)), 1);
        assert_eq!(pack_tile(5, TileFlags::NONE), 5);
    }

    #[test]
    fn draw_emits_sprites_for_visible_tiles() {
        use uran_render::DrawList;
        let mut l = TileLayer::new("t", 4, 4);
        l.fill_rect(0, 0, 4, 4, 1);
        let mut list = DrawList::new();
        {
            let mut gfx = uran_render::Graphics::new(&mut list);
            l.draw(
                &mut gfx,
                &sheet(),
                16.0,
                Rect::from_xywh(0.0, 0.0, 64.0, 64.0),
            );
        }
        assert!(
            !list.sprites().is_empty(),
            "widoczne kafle trafiają do listy"
        );
    }
}
