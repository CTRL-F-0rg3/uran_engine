//! Kierunek postaci: cztery strony świata odpowiadają czterem wierszom
//! arkusza.
//!
//! Arkusze graczy trzymają wiersze w stałej kolejności — `0` = przód,
//! `1` = lewo, `2` = prawo, `3` = tył — więc dobór wiersza sprowadza się
//! do jednego `match`.

use uran_math::Vec2;

/// Kierunek, w którym patrzy postać.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    /// W stronę ekranu (w świecie: `-Y`, bo oś Y idzie w górę).
    #[default]
    Down,
    /// W lewo (`-X`).
    Left,
    /// W prawo (`+X`).
    Right,
    /// W dal (`+Y`).
    Up,
}

impl Facing {
    /// Wiersz arkusza z tym kierunkiem.
    pub const fn row(self) -> u16 {
        match self {
            Facing::Down => 0,
            Facing::Left => 1,
            Facing::Right => 2,
            Facing::Up => 3,
        }
    }

    /// Kierunek w postaci wektora jednostkowego.
    pub const fn vector(self) -> Vec2 {
        match self {
            Facing::Down => Vec2::new(0.0, -1.0),
            Facing::Left => Vec2::new(-1.0, 0.0),
            Facing::Right => Vec2::new(1.0, 0.0),
            Facing::Up => Vec2::new(0.0, 1.0),
        }
    }

    /// Czy wektor ruchu zgadza się z tym kierunkiem (choćby częściowo).
    ///
    /// Używane przy odbiciu sprite'a: odbijamy tylko wtedy, gdy postać
    /// faktycznie idzie w bok, a nie gdy stoi z twarzą do kamery.
    pub fn matches(self, motion: Vec2) -> bool {
        motion.dot(self.vector()) > 0.0
    }

    /// Kierunek z wektora ruchu, z progiem ignorowania drobnych składowych.
    ///
    /// `deadzone` chroni przed „drganiem” kierunku, gdy gracz trzyma
    /// klawisz blisko progu: wektor ma wtedy składowe 0.7 i 0.7, a wybór
    /// kierunku nie powinien przestawiać się co klatkę.
    ///
    /// Gdy wektor jest zbyt krótki, zwraca `None` — i to **celowo** nie jest
    /// błędem: wołający kod zachowuje wtedy poprzedni kierunek, dzięki
    /// czemu postać nie odwraca się w losową stronę w miejscu.
    pub fn from_motion(motion: Vec2, deadzone: f32) -> Option<Self> {
        let len = motion.length();
        if len <= deadzone {
            return None;
        }
        // Po normalizacji porównujemy składowe: większa wygra, co daje
        // granicę 45° — „w prawo i trochę w górę” to nadal „w prawo”.
        let n = motion / len;
        Some(if n.x.abs() >= n.y.abs() {
            if n.x < 0.0 {
                Facing::Left
            } else {
                Facing::Right
            }
        } else if n.y < 0.0 {
            Facing::Down
        } else {
            Facing::Up
        })
    }

    /// Kierunek z wektora, z domyślną strefą martwą.
    pub fn from_motion_auto(motion: Vec2) -> Option<Self> {
        Self::from_motion(motion, DEFAULT_DEADZONE)
    }
}

/// Domyślna strefa martwa dla [`Facing::from_motion`].
///
/// Celowo bardzo mała: chcemy reagować na każdy *znormalizowany* ruch, a
/// `None` ma znaczenie tylko dla wektora identycznie zerowego.
pub const DEFAULT_DEADZONE: f32 = 1e-4;
#[cfg(test)]
mod tests {
    use super::*;

    /// Kolejność wierszy musi odpowiadać układowi arkusza gracza.
    #[test]
    fn rows_match_sheet_layout() {
        assert_eq!(Facing::Down.row(), 0);
        assert_eq!(Facing::Left.row(), 1);
        assert_eq!(Facing::Right.row(), 2);
        assert_eq!(Facing::Up.row(), 3);
    }

    /// Proste wektory dają jednoznaczny kierunek.
    #[test]
    fn axis_aligned_motion() {
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(1.0, 0.0)),
            Some(Facing::Right)
        );
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(-1.0, 0.0)),
            Some(Facing::Left)
        );
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(0.0, 1.0)),
            Some(Facing::Up)
        );
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(0.0, -1.0)),
            Some(Facing::Down)
        );
    }

    /// Oś Y świata idzie w górę, a `Down` ma patrzeć na gracza (w dół
    /// ekranu) — pomylenie tych dwóch odwróciłoby postać głową do góry.
    #[test]
    fn down_is_minus_y_in_world_space() {
        assert_eq!(Facing::Down.vector().y, -1.0);
        assert_eq!(Facing::Up.vector().y, 1.0);
    }

    /// Ruch po skosie wybiera oś dominującą.
    #[test]
    fn diagonal_picks_dominant_axis() {
        // dokładna skośna 45° — granica, wybieramy składową poziomą
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(1.0, -1.0)),
            Some(Facing::Right)
        );
        // wyraźnie w dół i w prawo -> w dół
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(1.0, -4.0)),
            Some(Facing::Down)
        );
        // wyraźnie w prawo i lekko w górę -> w prawo
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(4.0, 1.0)),
            Some(Facing::Right)
        );
    }

    /// Wektor poniżej progu nie zmienia kierunku — `None` zamiast losowego
    /// „prawo”, bo w miejscu gracz powinien patrzeć tam, gdzie patrzył.
    #[test]
    fn tiny_motion_keeps_previous_facing() {
        assert_eq!(Facing::from_motion(Vec2::new(0.05, 0.0), 0.1), None);
        assert_eq!(Facing::from_motion(Vec2::ZERO, 0.1), None);
        assert_eq!(Facing::from_motion_auto(Vec2::ZERO), None);
    }

    /// Skala wektora nie wpływa na wybór kierunku — wolniejsze chodzenie nie
    /// może obracać postaci inaczej niż sprint.
    #[test]
    fn direction_is_scale_invariant() {
        assert_eq!(
            Facing::from_motion_auto(Vec2::new(0.5, 0.0)),
            Facing::from_motion_auto(Vec2::new(100.0, 0.0))
        );
    }

    /// `matches` musi zgadzać się z wektorem kierunku, bo decyduje odbiciu.
    #[test]
    fn matches_agrees_with_vector() {
        assert!(Facing::Right.matches(Vec2::new(3.0, 0.5)));
        assert!(!Facing::Left.matches(Vec2::new(3.0, 0.5)));
        // ruch do przodu nie jest ruchem w prawo
        assert!(!Facing::Right.matches(Vec2::new(0.0, -3.0)));
        assert!(Facing::Down.matches(Vec2::new(0.0, -3.0)));
    }

    /// Kierunek domyślny to „w stronę gracza” — nowa postać patrzy na kamerę.
    #[test]
    fn default_is_down() {
        assert_eq!(Facing::default(), Facing::Down);
    }
}
