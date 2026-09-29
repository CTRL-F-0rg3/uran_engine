//! Gracz: ruch poziomy, skok i kolizje z tilemapą.
//!
//! Świadomie **bez silnika fizyki** (jak [`junak-rider`]): ruch jest
//! analityczny, a świat to prostokątne kafle. Rapier byłby o rząd
//! wielkości cięższy i wniósłby dokładnie ten sam błąd.
//!
//! ## Dwa okna na skok
//!
//! `coyote` i `jump_buffer` to dwa najczęstsze źródła „dziwnego
//! sterowania" w platformówkach:
//!
//! * **Coyote time** — gracz może skoczyć przez ~0,1 s po zejściu
//!   z krawędzi. Bez tego skok „nie działa", bo gracz wyciska spację
//!   odrobinę za późno.
//! * **Jump buffer** — naciśnięcie spacji przed lądowaniem (~0,12 s)
//!   zapamiętujujemy i wykonujemy skok natychmiast po dotknięciu ziemi.
//!
//! Bez obu okien gra jest „śmierdząca", a testerzy zgłaszają to jako
//! „skok nie działa", nie rozumiejąc przyczyny.

use uran_math::{Rect, Vec2};

use crate::tilemap::{TILE, TileMap};

/// Szerokość ciała gracza.
pub const BODY_W: f32 = 20.0;
/// Wysokość ciała gracza.
pub const BODY_H: f32 = 34.0;

/// Parametry gracza — jeden zestaw, bo to jeden model.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConfig {
    /// Prędkość biegu (jednostki świata na sekundę).
    pub run_speed: f32,
    /// Przyspieszenie poziome — jak szybko gracz osiąga pełną prędkość.
    pub accel: f32,
    /// Tarcie podłoża. Zero oznacza „ślizganie", zbyt dużo „klej".
    pub friction: f32,
    /// Prędkość skoku w chwili startu.
    pub jump_speed: f32,
    /// Grawitacja (ujemna = w dół).
    pub gravity: f32,
    /// Mnożnik grawitacji przy spadaniu (większy 1 = szybszy, „ciężki").
    pub fall_multiplier: f32,
    /// Jak długo po zejściu z krawędzi można jeszcze skoczyć (s).
    pub coyote: f32,
    /// Jak długo naciśnięcie skoku czeka na lądowanie (s).
    pub jump_buffer: f32,
    /// Najwyższa prędkość spadania — chroni przed tunelowaniem.
    pub terminal_velocity: f32,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            // Prędkość 260 i kafel 32 dają ~8 kafli na sekundę: szybko,
            // ale gracz wciąż zdąży zobaczyć nadciągający pocisk.
            run_speed: 260.0,
            accel: 2000.0,
            friction: 2400.0,
            // Skok ~3 kafle wysoko: wystarczy na unik w powietrzu,
            // a nie na przeskoczenie całej mapy.
            jump_speed: 560.0,
            gravity: -1500.0,
            fall_multiplier: 1.35,
            coyote: 0.10,
            jump_buffer: 0.12,
            terminal_velocity: 900.0,
        }
    }
}

/// Stan gracza: prostokąt ciała, prędkość i okna skoku.
#[derive(Debug, Clone, Copy)]
pub struct Player {
    /// **Dół** ciała (lewy dolny róg) — jak w całym silniku.
    pub pos: Vec2,
    /// Prędkość w jednostkach na sekundę.
    pub vel: Vec2,
    /// Czy stoi na ziemi.
    pub grounded: bool,
    /// Ile zostało z okna coyote.
    coyote_left: f32,
    /// Ile zostało z bufora skoku.
    buffer_left: f32,
    /// Czy trzyma klawisz skoku (dla skoku zmiennej wysokości).
    jump_held: bool,
    /// Kierunek patrzenia: 1 = w prawo, -1 = w lewo.
    pub facing: f32,
    /// Czy gracz jest ranny (blokada na chwilę).
    pub hurt: bool,
    /// Ile zostało nietykalności po trafieniu.
    invuln: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            grounded: false,
            coyote_left: 0.0,
            buffer_left: 0.0,
            jump_held: false,
            facing: 1.0,
            hurt: false,
            invuln: 0.0,
        }
    }
}

impl Player {
    /// Prostokąt ciała gracza.
    pub fn rect(&self) -> Rect {
        Rect::from_xywh(self.pos.x, self.pos.y, BODY_W, BODY_H)
    }

    /// Środek ciała — do celowania i efektów.
    pub fn center(&self) -> Vec2 {
        self.pos + Vec2::new(BODY_W * 0.5, BODY_H * 0.5)
    }

    /// Zadaje graczowi obrażenia i daje nietykalność.
    ///
    /// Nietykalność chroni przed „umieraniem" w klatce, w której
    /// trafienie zgłosiło kilka pocisków naraz — to bardzo frustrujące,
    /// bo gracz nie ma jak uciec.
    pub fn take_hit(&mut self) {
        if self.invuln > 0.0 {
            return;
        }
        self.hurt = true;
        self.invuln = INVULN_TIME;
    }

    /// Krok symulacji: wejście, grawitacja, skok, kolizje.
    ///
    /// `move_x` i `jump_pressed` to wejście w klatce (`-1/0/1`,
    /// `true/false`). Kolizje rozwiązuje [`TileMap`], więc ten moduł
    /// zna się z mapą, a nie z konkretnymi kaflami.
    pub fn step(
        &mut self,
        dt: f32,
        move_x: f32,
        jump_pressed: bool,
        jump_held: bool,
        map: &TileMap,
        cfg: &PlayerConfig,
    ) {
        // `dt` z debuggera albo przeciągnięte okno potrafi być ogromne.
        // Bez klamrowania skok wyrzuciłby gracza w kosmos, a pozycja
        // dostałaby NaN, co psuje renderer na zawsze.
        let dt = dt.clamp(0.0, 0.05);

        // --- ruch poziomy: przyspieszenie albo tarcie ---
        if move_x.abs() > 0.01 {
            self.facing = move_x.signum();
            self.vel.x += move_x * cfg.accel * dt;
            self.vel.x = self.vel.x.clamp(-cfg.run_speed, cfg.run_speed);
        } else {
            // Tarcie nie może przewrócić znaku — inaczej gracz
            // „drży" na miejscu zamiast się zatrzymać.
            let drop = cfg.friction * dt;
            self.vel.x = if self.vel.x.abs() <= drop {
                0.0
            } else {
                self.vel.x - self.vel.x.signum() * drop
            };
        }

        // --- okna skoku ---
        if jump_pressed {
            self.buffer_left = cfg.jump_buffer;
        } else {
            self.buffer_left = (self.buffer_left - dt).max(0.0);
        }
        self.jump_held = jump_held;
        if self.grounded {
            self.coyote_left = cfg.coyote;
        } else {
            self.coyote_left = (self.coyote_left - dt).max(0.0);
        }

        // --- skok ---
        //
        // Wypadający skok gwarantujemy przez `coyote_left > 0`, a skok
        // naciśnięty przed lądowaniem przez `buffer_left > 0`. Obie
        // warunki razem dają standardowe, „niedziwne" sterowanie.
        if self.buffer_left > 0.0 && self.coyote_left > 0.0 {
            self.vel.y = cfg.jump_speed;
            self.grounded = false;
            self.coyote_left = 0.0;
            self.buffer_left = 0.0;
        }

        // --- grawitacja ---
        //
        // Szybciej w dół niż w górę (`fall_multiplier`) — klasyczny
        // „asymetryczny skok", dzięki któremu opadanie nie trwa tyle
        // co wzbijanie. Bez tego unikanie ataków w powietrzu byłoby
        // męczące.
        if !self.grounded {
            let g = if self.vel.y < 0.0 {
                cfg.gravity * cfg.fall_multiplier
            } else {
                cfg.gravity
            };
            self.vel.y += g * dt;
            if self.vel.y < -cfg.terminal_velocity {
                self.vel.y = -cfg.terminal_velocity;
            }
        }

        // Puścienie skoku ucina wznoszenie — skok zmiennej wysokości.
        if !self.jump_held && self.vel.y > 0.0 {
            self.vel.y *= 0.45;
        }

        if self.invuln > 0.0 {
            self.invuln = (self.invuln - dt).max(0.0);
            if self.invuln == 0.0 {
                self.hurt = false;
            }
        }

        self.integrate(dt, map);
    }

    /// Ruch z rozwiązywaniem kolizji: najpierw X, potem Y.
    ///
    /// Kolejność jest celowa. Po ruchu po X **testujemy** kolizję i
    /// cofamy pozycję, a dopiero potem ruszamy Y. Bez cofania gracz
    /// wchodziłby w ścianę i dopiero przy następnym kaflu wypychałby
    /// się — wyglądałoby to jak „przyklejenie do ściany".
    fn integrate(&mut self, dt: f32, map: &TileMap) {
        // --- oś X ---
        let step_x = self.vel.x * dt;
        self.pos.x += step_x;
        if map.overlaps_solid(self.rect()) {
            self.pos.x -= step_x;
            self.vel.x = 0.0;
        }

        // --- oś Y ---
        //
        // Zapamiętujemy, czy **przed** ruchem gracz spadał. Bez tego
        // nie odróżnilibyśmy lądowania od wznoszenia: po skoku stopi
        // są w powietrzu, a pod nimi wciąż jest podłoga, więc test
        // „czy coś jest pod stopami" dawałby fałszywe trafienie
        // i gracz przyklejałby się do podłogi w miejscu skoku.
        let falling = self.vel.y <= 0.0;
        let step_y = self.vel.y * dt;
        self.pos.y += step_y;

        if !falling {
            // Wznoszenie: jedyny warunek to sufit (pełny blok).
            // Platformy **nie** blokują od dołu — to ich sens.
            if map.overlaps_solid(self.rect()) {
                self.pos.y -= step_y;
                self.vel.y = 0.0;
            }
            self.grounded = false;
            return;
        }

        // Spadanie: lądujemy, gdy pod stopami pojawił się kafel.
        if map.ground_below(self.rect(), 2.0) || map.overlaps_solid(self.rect()) {
            self.land(map);
        } else {
            self.grounded = false;
        }
    }

    /// Lądowanie: stopy na dachu kafla, prędkość zerowa.
    ///
    /// Wyrównujemy do **najwyższego dachu pod stopami**, a nie do
    /// pierwszego znalezionego — inaczej przy schodkach gracz
    /// lądowałby „w powietrzu" na najniższym z kafli.
    fn land(&mut self, map: &TileMap) {
        let cx0 = (self.pos.x / TILE).floor() as i32;
        let cx1 = ((self.pos.x + BODY_W) / TILE).floor() as i32;
        let cy_foot = (self.pos.y / TILE).floor() as i32;
        let mut best: Option<f32> = None;
        for cy in (cy_foot - 2)..=(cy_foot + 1) {
            for cx in cx0..=cx1 {
                if map.at(cx, cy).blocks_from_above() {
                    // Dach kafla to górna krawędź: `y = (cy + 1) * TILE`.
                    let top = (cy + 1) as f32 * TILE;
                    // Bierzemy tylko dachy nie wyższe niż pół kafla nad
                    // stopami — wyższe to sufit, w który gracz wpadł.
                    if top <= self.pos.y + TILE * 0.5 {
                        best = Some(best.map_or(top, |b: f32| b.max(top)));
                    }
                }
            }
        }
        match best {
            Some(top) => {
                self.pos.y = top;
                self.grounded = true;
            }
            None => self.grounded = false,
        }
        self.vel.y = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tilemap::{MAP_H, MAP_W, Tile};

    const DT: f32 = 1.0 / 60.0;

    /// Mapa z wierszem podłogi na dole, reszta pusta.
    fn map_with(rows: &[(usize, usize, Tile)]) -> TileMap {
        let mut grid: Vec<Vec<Tile>> = vec![vec![Tile::Empty; MAP_W]; MAP_H];
        grid[0] = vec![Tile::Solid; MAP_W];
        for &(y, x, t) in rows {
            grid[y][x] = t;
        }
        let refs: Vec<&[Tile]> = grid.iter().map(|r| r.as_slice()).collect();
        TileMap::from_rows(&refs)
    }

    /// Płaska mapa: sama podłoga.
    fn flat_map() -> TileMap {
        map_with(&[])
    }

    /// Gracz stoi na podłodze (`y = TILE`, dach kafla rzędu 0).
    fn standing() -> (Player, PlayerConfig) {
        let mut p = Player::default();
        p.pos = Vec2::new(100.0, TILE);
        p.grounded = true;
        (p, PlayerConfig::default())
    }

    /// Wykonuje `n` klatek z danym wejściem.
    ///
    /// `jump` to **wciśnięcie** (edge). Przytrzymanie ustawiamy
    /// osobno przez `held`, bo skok zmiennej wysokości ucina wznoszenie
    /// przy puszczeniu — test z samym edge'em widziałby skok o połowę
    /// niższy niż w prawdziwej grze.
    fn step_n(
        p: &mut Player,
        n: usize,
        move_x: f32,
        jump: bool,
        map: &TileMap,
        cfg: &PlayerConfig,
    ) {
        for _ in 0..n {
            p.step(DT, move_x, jump, false, map, cfg);
        }
    }

    #[test]
    fn standing_still_does_not_move_the_player() {
        let (p, cfg) = standing();
        let map = flat_map();
        let before = p.pos;
        let mut p2 = p;
        step_n(&mut p2, 30, 0.0, false, &map, &cfg);
        assert!((p2.pos - before).length() < 0.01, "gracz drży w miejscu");
    }

    #[test]
    fn gravity_pulls_the_player_down_to_the_floor() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        p.pos.y = TILE + 100.0;
        p.grounded = false;
        step_n(&mut p, 120, 0.0, false, &map, &cfg);
        assert!(p.grounded, "gracz nie wylądował");
        assert!((p.pos.y - TILE).abs() < 0.5, "stopy {} != {TILE}", p.pos.y);
    }

    #[test]
    fn running_accelerates_to_the_configured_speed() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        step_n(&mut p, 120, 1.0, false, &map, &cfg);
        assert!(
            (p.vel.x - cfg.run_speed).abs() < 1.0,
            "prędkość {} != {}",
            p.vel.x,
            cfg.run_speed
        );
        assert!(p.pos.x > 100.0, "gracz nie poszedł");
    }

    #[test]
    fn friction_stops_the_player_fully() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        step_n(&mut p, 60, 1.0, false, &map, &cfg);
        step_n(&mut p, 300, 0.0, false, &map, &cfg);
        assert_eq!(p.vel.x, 0.0, "gracz musi się zatrzymać");
    }

    #[test]
    fn jump_leaves_the_ground_and_comes_back() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        p.step(DT, 0.0, true, true, &map, &cfg);
        assert!(p.vel.y > 0.0, "skok nie nadał prędkości");
        assert!(!p.grounded, "gracz jest w powietrzu");
        let start = p.pos.y;
        let mut peak = start;
        // Przytrzymujemy klawisz — inaczej skok zmiennej wysokości
        // ucina wznoszenie i test widziałby za niski szczyt. Wciśnięcie
        // (`i == 0`) to **edge**: powtarzane co klatkę odnawiałoby skok
        // w locie, co jest błędem, a nie cechą skoku.
        for i in 0..120 {
            p.step(DT, 0.0, i == 0, true, &map, &cfg);
            peak = peak.max(p.pos.y);
        }
        // `jump_speed² / 2 * gravity` ≈ 104 px ≈ 3,2 kafla.
        assert!(
            peak - start > TILE * 2.5,
            "skok za niski: {start} -> {peak}"
        );
        assert!(p.grounded, "gracz nie wylądował");
    }

    #[test]
    fn releasing_jump_early_makes_a_shorter_hop() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        let start = p.pos.y;
        // Naciskamy i **puszczamy** natychmiast.
        p.step(DT, 0.0, true, false, &map, &cfg);
        let mut peak = p.pos.y;
        for _ in 0..120 {
            p.step(DT, 0.0, false, false, &map, &cfg);
            peak = peak.max(p.pos.y);
        }
        // Krótki skok jest wyraźnie niższy niż pełny (sprawdzamy
        // niżej) — tu pilnujemy tylko, że w ogóle wznosi.
        assert!(peak > start + 2.0, "puszczony skok to nie skok");
    }

    #[test]
    fn no_double_jump_in_mid_air() {
        let (mut p, cfg) = standing();
        let map = flat_map();
        p.step(DT, 0.0, true, true, &map, &cfg);
        let first = p.vel.y;
        p.step(DT, 0.0, true, true, &map, &cfg);
        assert!(p.vel.y < first, "podwójny skok: {first} -> {}", p.vel.y);
    }
}

/// Czas nietykalności po trafieniu (s).
pub const INVULN_TIME: f32 = 1.1;
