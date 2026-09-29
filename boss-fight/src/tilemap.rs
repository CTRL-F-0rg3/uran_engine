//! Tilemap 2D: siatka kafelków, kolizje i rysowanie.
//!
//! Świat to **siatka** kafelków o stałym rozmiarze. Kafelek to
//! najprostsza jednostka, którą umie zrozumieć zarówno gracz (kolizja),
//! jak i renderer (rysowanie), więc mapa świata i to, co gracz widzi,
//! nigdy nie rozjadą się ze sobą.
//!
//! ## Dlaczego nie tekstura kafelków
//!
//! Silnik nie ma tilemapa ani atlasu tekstur, więc kafle rysujemy
//! **wektorowo** (`Graphics::draw_rect`). To świadomy wybór: wygląd
//! każdego kafla opisuje funkcja, a nie plik graficzny, więc da się je
//! animować bez ładowania assetów. Koszt: jeden draw call na widoczny
//! kafel, a [`TileMap::visible_tiles`] odsiewa to, co poza kadrem.
//!
//! ## Oś Y w górę
//!
//! Rząd `0` jest na **dole**, a indeks rośnie w górę — zgodnie z resztą
//! silnika (`Rect::min`/`max`, kamera). Odwrócenie tej osi to klasyczny
//! błąd, który objawia się grawitacją w górę.

use uran_math::{Rect, Vec2};

/// Rozmiar jednego kafla w jednostkach świata.
pub const TILE: f32 = 32.0;

/// Szerokość mapy w kaflach.
///
/// **40 × `TILE` = 1280 = szerokość okna**, więc przy `fit_world`
/// zoom wynosi dokładnie 1,0 i 1 kafel = 1 piksel. To nie przypadek:
/// arena o proporcjach 2,7:1 przy oknie 16:9 zostawiałaby szerokie
/// puste pasy po bokach, a boss i pociski byłyby maleńkie.
pub const MAP_W: usize = 40;
/// Wysokość mapy w kaflach (22 × 32 = 704, mieści się w 720 px).
pub const MAP_H: usize = 22;

/// Rodzaj kafla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    /// Pusto — gracz przechodzi.
    Empty,
    /// Pełny blok — gracz staje na nim i nie przechodzi.
    Solid,
    /// Cienka platforma: gracz stoi na niej **z góry**, a przeskakuje
    /// od dołu. To kluczowe dla walki z bossem — platformy muszą
    /// pozwalać uciec w górę, inaczej unikanie ataków w powietrzu
    /// byłoby możliwe tylko grawitacyjnie.
    Platform,
    /// Kolce — ranią, ale nie blokują.
    Spike,
}

impl Tile {
    /// Czy kafel zatrzymuje gracza przy ruchu poziomym.
    pub fn is_solid(self) -> bool {
        matches!(self, Tile::Solid)
    }

    /// Czy kafel zatrzymuje gracza **z góry** (działa też platforma).
    pub fn blocks_from_above(self) -> bool {
        matches!(self, Tile::Solid | Tile::Platform)
    }
}

/// Mapa kafelków wraz z rozmiarem świata w jednostkach.
#[derive(Debug, Clone)]
pub struct TileMap {
    tiles: Vec<Tile>,
}

impl TileMap {
    /// Buduje mapę ze wskazanych kafli, wierszami od dołu.
    pub fn from_rows(rows: &[&[Tile]]) -> Self {
        let mut tiles = vec![Tile::Empty; MAP_W * MAP_H];
        for (y, row) in rows.iter().enumerate().take(MAP_H) {
            for (x, &t) in row.iter().enumerate().take(MAP_W) {
                tiles[y * MAP_W + x] = t;
            }
        }
        Self { tiles }
    }

    /// Kafel w sietce, `Empty` poza mapą.
    pub fn at(&self, x: i32, y: i32) -> Tile {
        if x < 0 || y < 0 || x >= MAP_W as i32 || y >= MAP_H as i32 {
            return Tile::Empty;
        }
        self.tiles[y as usize * MAP_W + x as usize]
    }

    /// Prostokąt kafla w świecie.
    pub fn tile_rect(x: i32, y: i32) -> Rect {
        Rect::from_xywh(x as f32 * TILE, y as f32 * TILE, TILE, TILE)
    }

    /// Czy prostokąt gracza styka się z jakimś pełnym blokiem.
    pub fn overlaps_solid(&self, r: Rect) -> bool {
        let cx0 = (r.min.x / TILE).floor() as i32;
        let cy0 = (r.min.y / TILE).floor() as i32;
        let cx1 = (r.max.x / TILE).floor() as i32;
        let cy1 = (r.max.y / TILE).floor() as i32;
        (cy0..=cy1).any(|cy| (cx0..=cx1).any(|cx| self.at(cx, cy).is_solid()))
    }

    /// Czy na prostokącie gracza jest **koliec** (raniący).
    pub fn touches_spike(&self, r: Rect) -> bool {
        let cx0 = (r.min.x / TILE).floor() as i32;
        let cy0 = (r.min.y / TILE).floor() as i32;
        let cx1 = (r.max.x / TILE).floor() as i32;
        let cy1 = (r.max.y / TILE).floor() as i32;
        (cy0..=cy1).any(|cy| (cx0..=cx1).any(|cx| self.at(cx, cy) == Tile::Spike))
    }

    /// Czy tuż pod prostokątem jest platforma albo blok.
    ///
    /// Osobne od `overlaps_solid`, bo sprawdza miejsce **pod** obiektem,
    /// a nie sam obiekt — to wykrywa lądowanie.
    pub fn ground_below(&self, r: Rect, probe: f32) -> bool {
        let foot = Rect::from_xywh(r.min.x, r.min.y - probe, r.width(), probe);
        let cx0 = (foot.min.x / TILE).floor() as i32;
        let cy0 = (foot.min.y / TILE).floor() as i32;
        let cx1 = (foot.max.x / TILE).floor() as i32;
        let cy1 = (foot.max.y / TILE).floor() as i32;
        (cy0..=cy1).any(|cy| (cx0..=cx1).any(|cx| self.at(cx, cy).blocks_from_above()))
    }

    /// Czy gracz spadł poza mapę (do otchłani).
    pub fn is_below_world(&self, p: Vec2) -> bool {
        p.y < -TILE * 2.0
    }

    /// Kafle potencjalnie widoczne — do rysowania.
    ///
    /// Odsiewamy poza kadrem: rysowanie ~1300 kafli, z czego 40 jest
    /// na ekranie, to marnowanie czasu na GPU.
    pub fn visible_tiles(&self, view: Rect) -> Vec<(i32, i32, Tile)> {
        let cx0 = (view.min.x / TILE).floor() as i32;
        let cy0 = (view.min.y / TILE).floor() as i32;
        let cx1 = (view.max.x / TILE).floor() as i32;
        let cy1 = (view.max.y / TILE).floor() as i32;
        let mut out = Vec::new();
        for cy in cy0.max(0)..=cy1.min(MAP_H as i32 - 1) {
            for cx in cx0.max(0)..=cx1.min(MAP_W as i32 - 1) {
                let t = self.at(cx, cy);
                if t != Tile::Empty {
                    out.push((cx, cy, t));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pusta mapa z jednym blokiem w (2,1) — do testów kolizji.
    fn map_with_block() -> TileMap {
        let mut rows: Vec<Vec<Tile>> = vec![vec![Tile::Empty; MAP_W]; MAP_H];
        rows[1][2] = Tile::Solid;
        let refs: Vec<&[Tile]> = rows.iter().map(|r| r.as_slice()).collect();
        TileMap::from_rows(&refs)
    }

    #[test]
    fn empty_map_has_no_solid_tiles() {
        let rows: Vec<Vec<Tile>> = vec![vec![Tile::Empty; MAP_W]; MAP_H];
        let refs: Vec<&[Tile]> = rows.iter().map(|r| r.as_slice()).collect();
        let m = TileMap::from_rows(&refs);
        assert!(!m.overlaps_solid(Rect::from_xywh(0.0, 0.0, 100.0, 100.0)));
    }

    #[test]
    fn solid_tile_blocks_overlap() {
        let m = map_with_block();
        // Kafel (2,1) zajmuje x∈[64,96], y∈[32,64].
        assert!(m.overlaps_solid(Rect::from_xywh(70.0, 40.0, 10.0, 10.0)));
        assert!(!m.overlaps_solid(Rect::from_xywh(10.0, 40.0, 10.0, 10.0)));
    }

    #[test]
    fn spike_is_not_solid_but_hurts() {
        let mut rows: Vec<Vec<Tile>> = vec![vec![Tile::Empty; MAP_W]; MAP_H];
        rows[1][3] = Tile::Spike;
        let refs: Vec<&[Tile]> = rows.iter().map(|r| r.as_slice()).collect();
        let m = TileMap::from_rows(&refs);
        let r = Rect::from_xywh(100.0, 40.0, 10.0, 10.0);
        assert!(!m.overlaps_solid(r), "kolec nie blokuje ruchu");
        assert!(m.touches_spike(r), "kolec rani");
    }

    #[test]
    fn platform_blocks_from_above_only() {
        assert!(Tile::Platform.blocks_from_above());
        assert!(!Tile::Platform.is_solid());
        assert!(Tile::Solid.is_solid());
        assert!(!Tile::Empty.blocks_from_above());
    }

    #[test]
    fn ground_below_finds_the_floor() {
        let m = map_with_block();
        // Stopi 1 px nad blokiem (2,1), którego góra to y=64.
        assert!(m.ground_below(Rect::from_xywh(70.0, 65.0, 16.0, 20.0), 2.0));
        assert!(!m.ground_below(Rect::from_xywh(70.0, 500.0, 16.0, 20.0), 2.0));
    }

    #[test]
    fn out_of_bounds_is_empty() {
        let m = map_with_block();
        assert_eq!(m.at(-1, 0), Tile::Empty);
        assert_eq!(m.at(MAP_W as i32, 0), Tile::Empty);
        assert_eq!(m.at(0, MAP_H as i32), Tile::Empty);
    }

    #[test]
    fn visible_tiles_skip_empty_and_respect_the_view() {
        let m = map_with_block();
        let view = Rect::from_xywh(32.0, 0.0, 160.0, 128.0);
        let tiles = m.visible_tiles(view);
        assert!(!tiles.is_empty(), "widoczny blok musi być w liście");
        assert!(
            tiles.iter().all(|(_, _, t)| *t != Tile::Empty),
            "puste kafle nie powinny być rysowane"
        );
        // Blok (2,1) musi się znaleźć w wyniku.
        assert!(tiles.iter().any(|(x, y, _)| *x == 2 && *y == 1));
    }

    #[test]
    fn is_below_world_detects_a_fall() {
        let m = map_with_block();
        assert!(m.is_below_world(Vec2::new(10.0, -TILE * 3.0)));
        assert!(!m.is_below_world(Vec2::new(10.0, 100.0)));
    }

    #[test]
    fn world_aspect_matches_the_window() {
        // Regression: arena 60×22 dawała proporcje 2,7:1 przy oknie
        // 16:9, więc `fit_world` zostawiał szerokie puste pasy, a boss
        // i pociski były ledwo widoczne. Wymagamy, żeby świat mieścił
        // się w oknie bez pustych pasów po bokach.
        let world = Vec2::new(MAP_W as f32 * TILE, MAP_H as f32 * TILE);
        let window = Vec2::new(1280.0, 720.0);
        // Zoom z `fit_world` to mniejszy z dwóch współczynników.
        let zoom = (window.x / world.x).min(window.y / world.y);
        // Widoczny obszar nie może być szerszy niż świat o więcej niż
        // 2% — inaczej w kadrze są puste kolumny.
        let visible_w = window.x / zoom;
        assert!(
            visible_w <= world.x * 1.02,
            "widoczna szerokość {visible_w} >> świat {}",
            world.x
        );
        // A w pionie świat musi się zmieścić, bo kamera pokazuje
        // całą arenę naraz.
        assert!(
            visible_w * (world.y / world.x) <= window.y * 1.02,
            "wysokość świata nie mieści się w oknie"
        );
    }
}
