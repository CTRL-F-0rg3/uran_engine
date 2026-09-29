//! Gracz: ruch po mapie, kolizje z kaflami i animacja sprite'a.
//!
//! Kolizje rozwiązujemy **osiami niezależnie** (najpierw X, potem Y) —
//! dzięki temu gracz ślizga się wzdłuż ściany zamiast utykać na rogu, a
//! ruch nigdy nie „wpycha" gracza w kafel.

use uran_math::{Rect, Vec2};

/// Prostokąt kolizji — mniejszy niż sprite, żeby gracz nie „łapał" rogów.
const HITBOX: Vec2 = Vec2::new(10.0, 8.0);

/// Jak długo trwa krok animacji (s).
const STEP_TIME: f32 = 0.22;

/// Stan gracza.
#[derive(Debug, Clone)]
pub struct Player {
    /// Środek sprite'a w świecie (oś Y w górę).
    pub position: Vec2,
    /// Kierunek patrzenia: 1 = w prawo, -1 = w lewo.
    pub facing: i32,
    /// Czy gracz się porusza (decyduje o animacji chodzenia).
    pub moving: bool,
    /// Czy gracz trzyma narzędzie (animacja pracy).
    pub working: bool,
    /// Czas od ostatniego kroku — napędza animację.
    step_time: f32,
    /// Numer klatki animacji chodu (0..3).
    anim: u32,
}

impl Player {
    pub fn at(position: Vec2) -> Self {
        Self {
            position,
            facing: 1,
            moving: false,
            working: false,
            step_time: 0.0,
            anim: 0,
        }
    }

    /// Prostokąt kolizji gracza (środek w `position`).
    pub fn hitbox(&self) -> Rect {
        Rect::from_center(self.position, HITBOX)
    }

    /// Czy gracz trzyma się jeszcze w obrębie mapy.
    fn clamp_to_map(&mut self, world: Rect) {
        let half = HITBOX * 0.5;
        self.position.x = self
            .position
            .x
            .clamp(world.min.x + half.x, world.max.x - half.x);
        self.position.y = self
            .position
            .y
            .clamp(world.min.y + half.y, world.max.y - half.y);
    }

    /// Przesuwa gracza o `delta`, nie wchodząc w kafle blokujące.
    ///
    /// `is_solid` to predykat z Tilemapy: czy prostokąt na (x, y) jest
    /// nieprzechodni.
    pub fn move_by(&mut self, delta: Vec2, world: Rect, is_solid: impl Fn(Rect) -> bool) {
        // Oś X: próbujemy całego ruchu, a przy kolizji — tylko do ściany.
        if delta.x != 0.0 {
            let mut next = self.position;
            next.x += delta.x;
            let probe = Self {
                position: next,
                ..self.clone()
            }
            .hitbox();
            if is_solid(probe) {
                // Cofamy do granicy ściany: krok po pikselu w stronę gracza
                // daje kontakt z kaflem bez „wpadnięcia" w niego.
                let step = delta.x.signum() * 0.5;
                let mut probe_x = self.position.x;
                while (probe_x - next.x).abs() > 0.0 {
                    probe_x -= step;
                    let mut trial = self.position;
                    trial.x = probe_x;
                    if is_solid(
                        Self {
                            position: trial,
                            ..self.clone()
                        }
                        .hitbox(),
                    ) {
                        break;
                    }
                }
                self.position.x = probe_x;
            } else {
                self.position.x = next.x;
            }
            if delta.x > 0.0 {
                self.facing = 1;
            } else if delta.x < 0.0 {
                self.facing = -1;
            }
        }

        // Oś Y: identycznie, ale pionowo.
        if delta.y != 0.0 {
            let mut next = self.position;
            next.y += delta.y;
            let probe = Self {
                position: next,
                ..self.clone()
            }
            .hitbox();
            if is_solid(probe) {
                let step = delta.y.signum() * 0.5;
                let mut probe_y = self.position.y;
                while (probe_y - next.y).abs() > 0.0 {
                    probe_y -= step;
                    let mut trial = self.position;
                    trial.y = probe_y;
                    if is_solid(
                        Self {
                            position: trial,
                            ..self.clone()
                        }
                        .hitbox(),
                    ) {
                        break;
                    }
                }
                self.position.y = probe_y;
            } else {
                self.position.y = next.y;
            }
        }

        self.clamp_to_map(world);
    }

    /// Aktualizuje animację chodu.
    pub fn tick_anim(&mut self, dt: f32) {
        if !self.moving {
            // W spoczynku wracamy do klatki 0, żeby sprite „stał".
            self.anim = 0;
            self.step_time = 0.0;
            return;
        }
        self.step_time += dt;
        while self.step_time >= STEP_TIME {
            self.step_time -= STEP_TIME;
            self.anim = (self.anim + 1) % 4;
        }
    }

    /// Numer klatki animacji do rysowania.
    pub fn anim_frame(&self) -> usize {
        match (self.working, self.moving) {
            (true, _) => 3,
            (false, true) => self.anim as usize,
            (false, false) => 0,
        }
    }
}
