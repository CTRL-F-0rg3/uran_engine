//! Chód, grawitacja i skok gracza.
//!
//! Świadomie **bez silnika fizyki** (jak [`junak-rider`]): ruch jest
//! w pełni analityczny, a świat to płaska ziemia z kilkunastoma
//! prostopadłościanami. Rapier byłby dwa rzędy wielkości cięższy i
//! wniósłby dokładnie ten sam błąd, tylko trudniej widoczny.
//!
//! Wszystko w metrach i sekundach, zgodnie z resztą silnika.

use uran_math::Vec3;

use crate::collide::World;

/// Parametry gracza — jeden zestaw, bo to jeden model.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConfig {
    /// Prędkość chodu (m/s).
    pub walk_speed: f32,
    /// Prędkość skoku w chwili startu (m/s, w górę).
    pub jump_speed: f32,
    /// Grawitacja (m/s²). Ujemna.
    pub gravity: f32,
    /// Najwyższa prędkość spadania (m/s) — ogranicza tunelowanie
    /// przy krótkich klatkach.
    pub terminal_velocity: f32,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            walk_speed: 5.0,
            // 7,4 m/s przy grawitacji 19,6 daje szczyt `v²/2g` ≈ 1,40 m.
            // To najmniej, co przeskakuje płot (1,1 m) z zapasem, a
            // jednocześnie tyle, ile potrzeba, żeby skok nie był
            // „balonem" unoszącym gracza na półtora metra.
            jump_speed: 7.4,
            gravity: -19.6,
            terminal_velocity: 45.0,
        }
    }
}

/// Stan gracza: pozycja dna ciała i prędkość pionowa.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    /// **Dno** gracza (stopy). Nie środek — patrz [`crate::collide`].
    pub pos: Vec3,
    /// Prędkość pionowa (m/s). Ujemna = spadanie.
    pub vy: f32,
    /// Czy gracz stoi na ziemi.
    pub grounded: bool,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            vy: 0.0,
            grounded: true,
        }
    }
}

impl Player {
    /// Krok symulacji: ruch poziomy ze świata, grawitacja i skok.
    ///
    /// `wish_dir` to kierunek ruchu **bez** składowej Y (liczony przez
    /// [`crate::look::move_dir`], który normalizuje wynik), a `world`
    /// rozwiązuje wejścia w przeszkody.
    pub fn step(
        &mut self,
        dt: f32,
        wish_dir: Vec3,
        want_jump: bool,
        world: &World,
        radius: f32,
        height: f32,
        cfg: &PlayerConfig,
    ) {
        // `dt` z debuggera albo przeciągnięte okno potrafi być ogromne.
        // Bez tego skok wyrzuciłby gracza w kosmos, a pozycja dostałaby
        // NaN, co psuje renderer na zawsze (macierze z NaN nie wracają
        // do poprawnego stanu).
        let dt = dt.clamp(0.0, 0.1);

        // --- skok ---
        //
        // `grounded` z **poprzedniej** klatki: dzięki temu skok działa
        // także wtedy, gdy gracz właśnie wylądował (w klatce lądowania
        // `pos.y` jest już 0, ale grawitacja jeszcze nie zdążyła przyjąć
        // ujemnej prędkości).
        if want_jump && self.grounded {
            self.vy = cfg.jump_speed;
            self.grounded = false;
        }

        // --- grawitacja ---
        if !self.grounded {
            self.vy += cfg.gravity * dt;
            if self.vy < -cfg.terminal_velocity {
                self.vy = -cfg.terminal_velocity;
            }
        }

        // --- ruch ---
        let delta = wish_dir * (cfg.walk_speed * dt) + Vec3::new(0.0, self.vy * dt, 0.0);
        world.move_player(&mut self.pos, delta, radius, height);

        // --- lądowanie ---
        //
        // Podłoga jest wszędzie na `y = 0` (płaski teren), więc
        // lądowanie sprowadza się do testu wysokości.
        //
        // Warunek to `pos.y <= 0.0`, a nie `pos.y <= ground_epsilon`.
        // Tolerancja wygląda na sensowną (gracz stojący „nad" ziemią
        // powinien być na ziemi), ale **zjada skok**: pierwsza klatka
        // po wciśnięciu unosi gracza tylko o `v*dt` ≈ 0,09 m, więc
        // przy tolerancji 0,35 m warunek byłby prawdziwy od razu i
        // `vy` zerowałby się w tej samej klatce, w której skok się
        // zaczął. Gracz nigdy by nie oderwał się od ziemi.
        if self.pos.y <= 0.0 {
            self.pos.y = 0.0;
            self.vy = 0.0;
            self.grounded = true;
        } else {
            self.grounded = false;
        }
    }

    /// Resetuje gracza do stanu stojącego w danym miejscu.
    pub fn teleport(&mut self, x: f32, z: f32) {
        self.pos = Vec3::new(x, 0.0, z);
        self.vy = 0.0;
        self.grounded = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::{PLAYER_HEIGHT, PLAYER_RADIUS, Solid};

    const DT: f32 = 1.0 / 60.0;

    /// Świat pusty — do testów samej grawitacji i skoku.
    fn open_world() -> World {
        World::new(50.0)
    }

    /// Wykonuje `n` klatek chodu w kierunku `dir`.
    fn walk(p: &mut Player, n: usize, dir: Vec3, w: &World, c: &PlayerConfig) {
        for _ in 0..n {
            p.step(DT, dir, false, w, PLAYER_RADIUS, PLAYER_HEIGHT, c);
        }
    }

    #[test]
    fn starts_on_the_ground() {
        let p = Player::default();
        assert!(p.grounded);
        assert_eq!(p.pos.y, 0.0);
        assert_eq!(p.vy, 0.0);
    }

    #[test]
    fn walking_moves_along_the_requested_direction() {
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();
        walk(&mut p, 60, Vec3::new(1.0, 0.0, 0.0), &w, &c);
        // 1 s chodu z prędkością 5 m/s = ~5 m.
        assert!((p.pos.x - 5.0).abs() < 0.1, "poszedł {} m", p.pos.x);
        assert!(p.pos.z.abs() < 1e-3, "uciekł w bok: {:?}", p.pos);
    }

    #[test]
    fn jump_rises_then_lands_back_grounded() {
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();

        p.step(DT, Vec3::ZERO, true, &w, PLAYER_RADIUS, PLAYER_HEIGHT, &c);
        assert!(p.vy > 0.0, "skok nie nadał prędkości w górę: {}", p.vy);
        assert!(!p.grounded, "w trakcie skoku gracz jest w powietrzu");

        // Szczyt: prędkość przechodzi przez zero.
        let mut peak = 0.0f32;
        for _ in 0..200 {
            p.step(DT, Vec3::ZERO, false, &w, PLAYER_RADIUS, PLAYER_HEIGHT, &c);
            peak = peak.max(p.pos.y);
        }
        assert!(peak > 0.8, "szczyt za niski: {peak}");
        assert!(peak < 2.0, "skok za wysoki: {peak}");
        assert!(p.grounded, "nie wylądował z powrotem");
        assert_eq!(p.pos.y, 0.0);
        assert_eq!(p.vy, 0.0, "po lądowaniu prędkość musi się wyzerować");
    }

    #[test]
    fn jump_is_allowed_only_when_grounded() {
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();
        p.step(DT, Vec3::ZERO, true, &w, PLAYER_RADIUS, PLAYER_HEIGHT, &c);
        // W powietrzu trzymanie spacji nie może dawać kolejnych skoków.
        let after_first = p.vy;
        p.step(DT, Vec3::ZERO, true, &w, PLAYER_RADIUS, PLAYER_HEIGHT, &c);
        assert!(
            p.vy < after_first,
            "podwójny skok w powietrzu: {} -> {}",
            after_first,
            p.vy
        );
    }

    #[test]
    fn jump_and_walk_combine_in_the_air() {
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();
        p.step(
            DT,
            Vec3::new(1.0, 0.0, 0.0),
            true,
            &w,
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
            &c,
        );
        // W powietrzu gracz nadal się przesuwa — brak „zawieszenia".
        for _ in 0..10 {
            p.step(
                DT,
                Vec3::new(1.0, 0.0, 0.0),
                false,
                &w,
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
                &c,
            );
        }
        assert!(p.pos.x > 0.5, "w powietrzu stoi w miejscu: {}", p.pos.x);
    }

    #[test]
    fn player_stops_at_a_wall_instead_of_walking_through() {
        let mut w = open_world();
        w.add(Solid {
            pos: Vec3::new(4.0, 0.0, 0.0),
            width: 2.0,
            depth: 6.0,
            height: 3.0,
        });
        let c = PlayerConfig::default();
        let mut p = Player::default();
        walk(&mut p, 300, Vec3::new(1.0, 0.0, 0.0), &w, &c);
        // Ściana zaczyna się w x = 3, gracz ma promień 0,35.
        assert!(p.pos.x < 3.0, "przeszedł przez ścianę: {:?}", p.pos);
        assert!(p.pos.x > 2.0, "zatrzymał się za daleko: {:?}", p.pos);
    }

    #[test]
    fn wall_blocks_even_when_jumping_into_it() {
        // Gracz wskakuje pod niski dach: ściana 3 m jest za wysoka na
        // skok, więc nie może jej przekroczyć w powietrzu.
        let mut w = open_world();
        w.add(Solid {
            pos: Vec3::new(4.0, 0.0, 0.0),
            width: 2.0,
            depth: 6.0,
            height: 3.0,
        });
        let c = PlayerConfig::default();
        let mut p = Player::default();
        for _ in 0..180 {
            p.step(
                DT,
                Vec3::new(1.0, 0.0, 0.0),
                true,
                &w,
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
                &c,
            );
        }
        assert!(p.pos.x < 3.0, "wskoczył na 3-metrową ścianę: {:?}", p.pos);
    }

    #[test]
    fn low_wall_can_be_crossed_by_jumping() {
        // Płot 0,4 m — niższy niż stopy w szczycie skoku (~1,5 m),
        // więc da się go przeskoczyć.
        let mut w = open_world();
        w.add(Solid {
            pos: Vec3::ZERO,
            width: 0.4,
            depth: 6.0,
            height: 0.4,
        });
        let c = PlayerConfig::default();
        let mut p = Player::default();
        for _ in 0..90 {
            p.step(
                DT,
                Vec3::new(1.0, 0.0, 0.0),
                true,
                &w,
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
                &c,
            );
        }
        assert!(p.pos.x > 1.0, "nie przeskoczył niskiego płotu: {:?}", p.pos);
    }

    #[test]
    fn dt_spike_does_not_launch_the_player_into_space() {
        // Okno przeciągnięte: dt = 3 s w jednej klatce.
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();
        p.step(
            3.0,
            Vec3::new(1.0, 0.0, 0.0),
            true,
            &w,
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
            &c,
        );
        assert!(p.pos.is_finite(), "pozycja ma NaN: {:?}", p.pos);
        assert!(p.pos.y < 1.0, "wyrzuciło w górę: {}", p.pos.y);
        assert!(p.pos.length() < 3.0, "wyrzuciło w bok: {:?}", p.pos);
    }

    #[test]
    fn negative_dt_does_not_break_physics() {
        let w = open_world();
        let c = PlayerConfig::default();
        let mut p = Player::default();
        p.step(
            -1.0,
            Vec3::new(1.0, 0.0, 0.0),
            false,
            &w,
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
            &c,
        );
        assert!(p.pos.is_finite(), "pozycja ma NaN: {:?}", p.pos);
    }

    #[test]
    fn teleport_clears_momentum() {
        let mut p = Player::default();
        p.vy = 7.0;
        p.teleport(3.0, -4.0);
        assert_eq!(p.pos, Vec3::new(3.0, 0.0, -4.0));
        assert_eq!(p.vy, 0.0);
        assert!(p.grounded);
    }
}
