//! Mechanika farmy: siatka działek, sadzenie, wzrost, zbiór.
//!
//! Cała rozgrywka to jedna pętla na trzech stanach działki:
//!
//! ```text
//!    Pusta ──sadzenie──> Rosnąca ──upływ czasu──> Dojrzała
//!      ^                      │                        │
//!      └──────────────────────┴──────zbiór──────────────┘
//!                          (wraca do Pustej)
//! ```
//!
//! ## Dlaczego stan zamiast licznika
//!
//! `u8` z procentem wzrostu wymagałby osobnych przypadków dla
//! „puste", „zasadzone" i „dojrzałe", a błędy w takim kodzie
//! (np. `progress == 0` znaczy „puste" albo „właśnie zasadzone")
//! dają obiekty, które rosną bez właściciela. Enum z trzema
//! wariantami uniemożliwia taki stan: kompilator wymusza obsługę
//! każdego przypadku.
//!
//! ## Czas wzrostu
//!
//! `GROW_SECONDS` to sekundy, nie minuty — do przetestowania pętli
//! nie chce się czekać. Dojrzałość jest osiągalna, bo `progress`
//! rośnie o `dt / GROW_SECONDS` i jest przycięty do 1.0.

use uran_math::Vec3;

/// Stan jednej działki.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tile {
    /// Niezasadzona — można posadzić.
    Empty,
    /// Zasadzona i rosnąca. `t` to postęp 0.0..1.0.
    Growing { t: f32 },
    /// Dojrzała — można zebrać.
    Ready,
}

impl Tile {
    /// Czy na tej działce da się coś zrobić (sadzić albo zbierać)?
    pub fn is_actionable(&self) -> bool {
        matches!(self, Tile::Empty | Tile::Ready)
    }

    /// Postęp wzrostu 0.0..1.0 — do wysokości i koloru rośliny.
    pub fn growth(&self) -> f32 {
        match self {
            Tile::Empty => 0.0,
            Tile::Growing { t } => t.clamp(0.0, 1.0),
            Tile::Ready => 1.0,
        }
    }

    /// Czy roślina jest już dojrzała (żółta, do zbioru).
    pub fn is_ripe(&self) -> bool {
        matches!(self, Tile::Ready)
    }
}

/// Ile sekund trwa wyrost od sadu do dojrzałości.
pub const GROW_SECONDS: f32 = 24.0;

/// Szerokość pola w działkach.
pub const FIELD_W: usize = 6;
/// Głębokość pola w działkach.
pub const FIELD_D: usize = 6;
/// Rozmiar jednej działki w metrach.
pub const TILE: f32 = 2.0;

/// Odległość gracza od działki, przy której `E` działa.
pub const REACH: f32 = 1.9;

/// Wymiar pola wzdłuż osi X (liczba działek × ich rozmiar).
fn field_w_m() -> f32 {
    (FIELD_W as f32 - 1.0) * TILE
}

/// Wymiar pola wzdłuż osi Z.
fn field_d_m() -> f32 {
    (FIELD_D as f32 - 1.0) * TILE
}

/// Środek pola w świecie. Trzymamy pole w centrum, żeby postać i
/// kamera nie startowały na rogu mapy.
pub fn field_center() -> Vec3 {
    Vec3::ZERO
}

/// Światowa pozycja środka działki `(ix, iz)`.
pub fn tile_center(ix: usize, iz: usize) -> Vec3 {
    Vec3::new(
        ix as f32 * TILE - field_w_m() * 0.5,
        0.0,
        iz as f32 * TILE - field_d_m() * 0.5,
    )
}

/// Indeks działki najbliższej pozycji, albo `None` poza polem.
///
/// Zaokrąglamy zamiast sprawdzać sąsiadów: siatka jest równa, więc
/// `round` daje dokładnie ten indeks, którego środek jest najbliżej,
/// a wynik i tak odrzucamy poza zakresem.
pub fn tile_at(p: Vec3) -> Option<(usize, usize)> {
    let fx = (p.x + field_w_m() * 0.5) / TILE;
    let fz = (p.z + field_d_m() * 0.5) / TILE;
    let ix = fx.round();
    let iz = fz.round();
    if ix < 0.0 || iz < 0.0 || ix > (FIELD_W - 1) as f32 || iz > (FIELD_D - 1) as f32 {
        return None;
    }
    Some((ix as usize, iz as usize))
}

/// Stan wszystkich działek pola.
#[derive(Debug, Clone)]
pub struct Farm {
    tiles: Vec<Tile>,
    /// Zebrane sztuki (do HUD-a).
    pub harvested: u32,
    /// Ostatnia akcja gracza — do komunikatu na ekranie.
    pub message: String,
    /// Ile sekund zostało do zniknięcia komunikatu.
    pub message_time: f32,
}

impl Default for Farm {
    fn default() -> Self {
        Self::new()
    }
}

impl Farm {
    pub fn new() -> Self {
        Self {
            tiles: vec![Tile::Empty; FIELD_W * FIELD_D],
            harvested: 0,
            message: String::new(),
            message_time: 0.0,
        }
    }

    /// Stan działki `(ix, iz)`.
    pub fn get(&self, ix: usize, iz: usize) -> Tile {
        self.tiles[iz * FIELD_W + ix]
    }

    /// Ustawia stan działki `(ix, iz)`.
    pub fn set(&mut self, ix: usize, iz: usize, t: Tile) {
        self.tiles[iz * FIELD_W + ix] = t;
    }

    /// Czy wszystkie działki są dojrzałe (wygrana).
    pub fn is_won(&self) -> bool {
        self.tiles.iter().all(|t| t.is_ripe())
    }

    /// Ile działek jest w każdym ze stanów — do HUD-a.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut empty = 0;
        let mut growing = 0;
        let mut ready = 0;
        for t in &self.tiles {
            match t {
                Tile::Empty => empty += 1,
                Tile::Growing { .. } => growing += 1,
                Tile::Ready => ready += 1,
            }
        }
        (empty, growing, ready)
    }

    /// Czas świata — rośliny dojrzewają niezależnie od gracza.
    pub fn step(&mut self, dt: f32) {
        let rate = dt / GROW_SECONDS.max(0.001);
        for t in &mut self.tiles {
            if let Tile::Growing { t: g } = t {
                *g += rate;
                if *g >= 1.0 {
                    // `Ready` zamiast surowego `1.0` daje czytelny stan
                    // w HUD-zie i jeden warunek w testach.
                    *t = Tile::Ready;
                }
            }
        }
        if self.message_time > 0.0 {
            // `max(0.0)` jest potrzebny: samo `message_time -= dt` przy
            // dużym kroku daje wartość UJEMNĄ, a HUD sprawdza
            // `> 0.0` — komunikat znikałby dopiero przy losowym
            // obrocie wartości w okolicach zera. Przycinamy w dół.
            self.message_time = (self.message_time - dt).max(0.0);
        }
    }

    /// Sadzi lub zbiera na działce `(ix, iz)`.
    ///
    /// Zwraca `true`, gdy coś się stało — gracz dostaje wtedy
    /// informację, a HUD może zamigotać.
    pub fn interact(&mut self, ix: usize, iz: usize) -> bool {
        match self.get(ix, iz) {
            Tile::Empty => {
                self.set(ix, iz, Tile::Growing { t: 0.0 });
                self.say("Zasadzono!");
                true
            }
            Tile::Ready => {
                self.set(ix, iz, Tile::Empty);
                self.harvested += 1;
                self.say("Zebrane! +1");
                true
            }
            // Rosnąca działka: trzeba poczekać, nie da się przyspieszyć.
            Tile::Growing { .. } => {
                self.say("Jeszcze rośnie...");
                false
            }
        }
    }

    fn say(&mut self, m: &str) {
        self.message = m.to_string();
        self.message_time = 1.6;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_farm_is_all_empty() {
        let f = Farm::new();
        assert_eq!(f.get(0, 0), Tile::Empty);
        assert_eq!(f.counts(), (FIELD_W * FIELD_D, 0, 0));
        assert_eq!(f.harvested, 0);
    }

    #[test]
    fn planting_makes_a_tile_growing() {
        let mut f = Farm::new();
        assert!(f.interact(2, 3));
        assert_eq!(f.get(2, 3), Tile::Growing { t: 0.0 });
    }

    #[test]
    fn growing_tile_is_ripe_after_full_time() {
        let mut f = Farm::new();
        f.interact(1, 1);
        // Mniej niż czas: nie może być jeszcze dojrzała.
        f.step(GROW_SECONDS * 0.5);
        assert!(!f.get(1, 1).is_ripe());
        // Reszta czasu, dokładnie do końca.
        f.step(GROW_SECONDS * 0.5);
        assert!(f.get(1, 1).is_ripe());
        assert_eq!(f.get(1, 1), Tile::Ready);
    }

    #[test]
    fn overstepping_time_does_not_exceed_ripe() {
        let mut f = Farm::new();
        f.interact(0, 0);
        // 4× za dużo — stan nadal musi być `Ready`, a nie
        // `Growing { t: 4.0 }`, bo ten byłby nie do wyrenderowania.
        f.step(GROW_SECONDS * 4.0);
        assert_eq!(f.get(0, 0), Tile::Ready);
    }

    #[test]
    fn harvest_resets_tile_and_counts() {
        let mut f = Farm::new();
        f.interact(2, 2);
        f.step(GROW_SECONDS);
        assert!(f.interact(2, 2), "zbiór dojrzałej działa");
        assert_eq!(f.get(2, 2), Tile::Empty, "działka wraca do stanu pustego");
        assert_eq!(f.harvested, 1);
    }

    #[test]
    fn growing_tile_cannot_be_harvested() {
        let mut f = Farm::new();
        f.interact(1, 1);
        f.step(1.0);
        assert!(!f.interact(1, 1), "niedojrzałej nie da się zebrać");
        assert_eq!(f.harvested, 0);
        assert!(matches!(f.get(1, 1), Tile::Growing { .. }));
    }

    #[test]
    fn full_loop_can_be_repeated() {
        // sadz → czekaj → zbierz → sadz ponownie
        let mut f = Farm::new();
        for _ in 0..3 {
            f.interact(0, 0);
            f.step(GROW_SECONDS);
            f.interact(0, 0);
        }
        assert_eq!(f.harvested, 3);
        assert_eq!(f.get(0, 0), Tile::Empty);
    }

    #[test]
    fn field_is_centred_on_the_origin() {
        // Środek pola musi być w (0,0), inaczej postać i kamera
        // startowałyby na rogu mapy.
        let a = tile_center(0, 0);
        let b = tile_center(FIELD_W - 1, FIELD_D - 1);
        assert!(a.x < 0.0 && a.z < 0.0, "lewy górny róg to (-,-): {a:?}");
        assert!(b.x > 0.0 && b.z > 0.0, "prawy dolny to (+,+): {b:?}");
        assert!(
            (a.x.abs() - b.x.abs()).abs() < 1e-5,
            "pole nie jest symetryczne"
        );
    }

    #[test]
    fn tile_centre_and_lookup_agree() {
        for iz in 0..FIELD_D {
            for ix in 0..FIELD_W {
                let c = tile_center(ix, iz);
                assert_eq!(
                    tile_at(c),
                    Some((ix, iz)),
                    "działka ({ix},{iz}) w środku {c:?} nie zgadza się"
                );
            }
        }
    }

    #[test]
    fn lookup_rejects_positions_outside_the_field() {
        assert_eq!(tile_at(Vec3::new(1000.0, 0.0, 0.0)), None);
        assert_eq!(tile_at(Vec3::new(0.0, 0.0, -1000.0)), None);
    }

    #[test]
    fn all_tiles_ripe_means_won() {
        let mut f = Farm::new();
        assert!(!f.is_won(), "puste pole to nie wygrana");
        for iz in 0..FIELD_D {
            for ix in 0..FIELD_W {
                f.interact(ix, iz);
            }
        }
        assert!(!f.is_won(), "zasadzone to nie wygrana");
        f.step(GROW_SECONDS);
        assert!(f.is_won(), "wszystkie dojrzałe = wygrana");
    }

    #[test]
    fn growth_is_monotonic() {
        let mut f = Farm::new();
        f.interact(0, 0);
        let mut last = 0.0;
        for _ in 0..20 {
            f.step(0.5);
            let g = f.get(0, 0).growth();
            assert!(g >= last, "wzrost cofnął się: {last} -> {g}");
            last = g;
        }
    }

    #[test]
    fn message_expires() {
        let mut f = Farm::new();
        f.interact(0, 0);
        assert!(f.message_time > 0.0, "komunikat się pojawia");
        f.step(2.0);
        assert_eq!(f.message_time, 0.0, "komunikat znika po czasie");
    }
}
