//! Autotiling: dobieranie kafla brzegowego do otoczenia.
//!
//! Bez autotilingu woda jest prostokątem z widocznymi narożnikami. Klasyczne
//! rozwiązanie („blob", „terrain autotile”) to **16 wariantów** opisanych
//! czterema bitami: czy sąsiedzi góra/dół/lewo/prawa należą do tego samego
//! terenu. Kafel środkowy to 0b1111, narożnik zewnętrzny to 0b0101 itd.
//!
//! ## Dlaczego 4 bity, a nie 8
//!
//! Pełne 256 wariantów liczyłoby też narożniki „przekątne", których w
//! klasycznym stylu pixel art w ogóle się nie rysuje. Cztery bity dają 16
//! kafli na teren — tyle właśnie zawiera arkusz `terrains` z assets gry.

use crate::layer::TileLayer;

/// Sposób liczenia maski sąsiadów.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    /// Sąsiad musi mieć **dokładnie** ten sam indeks kafla.
    Exact,
    /// Liczy się tylko, czy sąsiad jest pusty — pozwala łączyć różne kafle
    /// tego samego terenu (np. dwa warianty trawy) w jeden spójny brzeg.
    NonEmpty,
}

/// Opis zestawu 16 kafli tworzących jeden teren.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoTile {
    /// Indeks pierwszego kafla wariantów; kolejne 16 leżą w kolejności
    /// `mask = 0, 1, 2, ...` (jak `terrains` w assets).
    pub base: u32,
    /// Czy środek terenu ma rysować wariant 0b1111, czy bazowy kafel.
    pub include_self: bool,
    /// Jak porównywać sąsiadów.
    pub mode: MatchMode,
}

impl AutoTile {
    /// Autotiling po identycznym indeksie kafla.
    pub const fn exact(base: u32) -> Self {
        Self {
            base,
            include_self: true,
            mode: MatchMode::Exact,
        }
    }

    /// Autotiling po „kafel pusty / niepusty".
    pub const fn by_presence(base: u32) -> Self {
        Self {
            base,
            include_self: true,
            mode: MatchMode::NonEmpty,
        }
    }

    /// Maska sąsiedów wokół kafla (x, y) w warstwie.
    ///
    /// Bity, od najmniej do najbardziej znaczącego:
    /// `1` góra, `2` prawa, `4` dół, `8` lewa.
    pub fn mask_at(&self, layer: &TileLayer, x: i32, y: i32) -> u8 {
        let center = layer.get(x, y);
        let up = self.matches(layer, x, y + 1, center);
        let right = self.matches(layer, x + 1, y, center);
        let down = self.matches(layer, x, y - 1, center);
        let left = self.matches(layer, x - 1, y, center);
        u8::from(up) | (u8::from(right) << 1) | (u8::from(down) << 2) | (u8::from(left) << 3)
    }

    /// Czy sąsiad na (nx, ny) należy do tego samego terenu.
    fn matches(&self, layer: &TileLayer, nx: i32, ny: i32, center: u32) -> bool {
        let t = layer.get(nx, ny);
        match self.mode {
            // `TILE_EMPTY` poza mapą to ta sama wartość co „brak kafla",
            // więc porównanie musi odrzucić pusty środek inaczej każdy
            // pusty kafel na krawędzi mapy dostałby maskę 0b1111.
            MatchMode::Exact => center != crate::layer::TILE_EMPTY && t == center,
            MatchMode::NonEmpty => t != crate::layer::TILE_EMPTY,
        }
    }

    /// Indeks kafla wariantu dla maski sąsiadów.
    pub fn index_for_mask(&self, mask: u8) -> u32 {
        // Maska 15 oznacza środek terenu. Jeśli `include_self` jest wyłączone,
        // środek ma być rysowany jako zwykły kafel (indeks bazowy), a nie
        // wariant — inaczej woda w środku jeziorka dostałaby brzeg ze środka.
        let slot = if mask == 0b1111 && !self.include_self {
            0
        } else {
            mask as u32 % 16
        };
        self.base + slot
    }

    /// Przelicza warianty w podanym miejscu i w promieniu `radius`.
    ///
    /// Po zmianie jednego kafla trzeba przeliczyć też jego sąsiadów, bo ich
    /// maski też się zmieniły — inaczej wzdłuż wykopanego rowu zostawałby
    /// stary, nieaktualny brzeg.
    pub fn refresh(&self, layer: &mut TileLayer, changed: (i32, i32), radius: i32) {
        let (cx, cy) = changed;
        let mut updates = Vec::new();
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let (x, y) = (cx + dx, cy + dy);
                if !layer.in_bounds(x, y) {
                    continue;
                }
                let mask = self.mask_at(layer, x, y);
                updates.push((x, y, self.index_for_mask(mask)));
            }
        }
        for (x, y, index) in updates {
            layer.set(x, y, index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer::{TileLayer, TILE_EMPTY};

    /// Warstwa 5x5 z polem 3x3 pośrodku, otoczonym pustkiem.
    fn field() -> TileLayer {
        let mut l = TileLayer::new("t", 5, 5);
        l.fill_rect(1, 1, 3, 3, 10);
        l
    }

    #[test]
    fn corner_tile_has_two_neighbours() {
        let l = field();
        let at = AutoTile::exact(0);
        // (1,1) to róg pola 3x3: sąsieduje góra (1,2) i prawa (2,1);
        // dołu (1,0) i lewej (0,1) nie ma, bo tam pusto.
        assert_eq!(at.mask_at(&l, 1, 1), 0b0011);
    }

    #[test]
    fn interior_tile_has_all_neighbours() {
        let l = field();
        let at = AutoTile::exact(0);
        assert_eq!(at.mask_at(&l, 2, 2), 0b1111, "centrum pola 3x3");
    }

    #[test]
    fn bit_order_is_up_right_down_left() {
        let mut l = TileLayer::new("t", 3, 3);
        l.set(1, 1, 7);
        let at = AutoTile::exact(0);

        l.set(1, 2, 7);
        assert_eq!(at.mask_at(&l, 1, 1) & 0b0001, 0b0001, "góra");
        l.set(1, 2, TILE_EMPTY);

        l.set(2, 1, 7);
        assert_eq!(at.mask_at(&l, 1, 1) & 0b0010, 0b0010, "prawa");
        l.set(2, 1, TILE_EMPTY);

        l.set(1, 0, 7);
        assert_eq!(at.mask_at(&l, 1, 1) & 0b0100, 0b0100, "dół");
        l.set(1, 0, TILE_EMPTY);

        l.set(0, 1, 7);
        assert_eq!(at.mask_at(&l, 1, 1) & 0b1000, 0b1000, "lewa");
    }

    #[test]
    fn exact_mode_ignores_other_tiles() {
        let mut l = TileLayer::new("t", 3, 3);
        l.fill_rect(0, 0, 3, 3, 1);
        l.set(1, 1, 2);
        let at = AutoTile::exact(0);
        // Sąsiad (2,1) to kafel 1, a środek to 2 -> brak dopasowania.
        assert_eq!(at.mask_at(&l, 1, 1) & 0b0010, 0);
    }

    #[test]
    fn presence_mode_joins_different_tiles() {
        let mut l = TileLayer::new("t", 3, 3);
        l.fill_rect(0, 0, 3, 3, 1);
        l.set(1, 1, 2);
        let at = AutoTile::by_presence(0);
        assert_eq!(at.mask_at(&l, 1, 1), 0b1111, "każdy sąsiad jest niepusty");
    }

    #[test]
    fn out_of_bounds_counts_as_no_neighbour() {
        let mut l = TileLayer::new("t", 2, 2);
        // Tylko narożny kafel (0,0); poza mapą nie ma sąsiadów.
        l.set(0, 0, 3);
        let at = AutoTile::exact(0);
        // (0,0): góra (0,1) i prawa (1,0) są puste, dół i lewa poza mapą.
        assert_eq!(at.mask_at(&l, 0, 0), 0, "dół i lewa są poza mapą");
        // (1,1) jest pusty, więc w ogóle nie ma czego liczyć — regresja na
        // `TILE_EMPTY` poza mapą, który w `Exact` wyglądałby jak sąsiad.
        assert_eq!(at.mask_at(&l, 1, 1), 0);
    }

    #[test]
    fn presence_mode_ignores_out_of_bounds() {
        let mut l = TileLayer::new("t", 2, 2);
        l.set(1, 1, 3);
        let at = AutoTile::by_presence(0);
        // (1,1) nie ma niepustych sąsiadów w obrębie mapy: (0,1) i (1,0)
        // są puste, a góra i prawa wychodzą poza mapę.
        assert_eq!(at.mask_at(&l, 1, 1), 0);
    }

    #[test]
    fn include_self_off_keeps_base_tile_in_the_middle() {
        let at = AutoTile {
            base: 100,
            include_self: false,
            mode: MatchMode::NonEmpty,
        };
        assert_eq!(at.index_for_mask(0b1111), 100, "środek wraca do bazowego");
        assert_ne!(at.index_for_mask(0b1010), 100, "brzeg to wariant");
    }

    #[test]
    fn refresh_updates_neighbourhood() {
        let mut l = field();
        AutoTile::exact(0).refresh(&mut l, (2, 2), 1);
        assert_eq!(l.get(2, 2), 15, "środek -> maska 0b1111");
        assert_eq!(l.get(1, 1), 0b0011, "narożnik: góra + prawa");
    }

    #[test]
    fn refresh_ignores_out_of_bounds() {
        let mut l = field();
        AutoTile::exact(0).refresh(&mut l, (0, 0), 2);
        assert_eq!((l.width(), l.height()), (5, 5));
        assert_eq!(l.get(-1, -1), TILE_EMPTY, "nic nie przyrosło poza mapą");
    }
}
