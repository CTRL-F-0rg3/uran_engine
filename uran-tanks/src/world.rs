//! Świat gry: teren, czołgi, pociski, AI przeciwników.
//!
//! Zasada: pozycje trzymamy na CPU, bo to kilkanaście obiektów — GPU
//! dostaje tylko macierze modeli. Renderer 3D rysuje je z jednego bufora
//! modeli (patrz `uran-render3d::DrawCmd`).

use uran_math::Vec3;

use crate::tank::{Shell, Tank, Team};

/// Półrozmiar mapy w jednostkach świata.
pub const ARENA_HALF: f32 = 90.0;

/// Generator xorshift32 — powtarzalny, żeby demo wyglądało tak samo
/// po każdym uruchomieniu (ułatwia porównywanie zrzutów).
#[derive(Debug, Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Rng(seed | 1) // 0 to punkt stały — xorshift z zerem nie rusza
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// Liczba z przedziału `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32
    }

    /// Liczba z przedziału `[min, max)`.
    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + self.f32() * (max - min)
    }
}

/// Teren analityczny — wysokość z kilku fal, bez siatki wierzchołków.
///
/// Dzięki temu czołg stoi dokładnie na gruncie, a pobranie wysokości
/// to jedno mnożenie i suma sinusoid zamiast przeszukiwania siatki.
#[derive(Debug, Clone, Copy)]
pub struct Terrain {
    pub phase_x: f32,
    pub phase_z: f32,
}

impl Terrain {
    pub fn new(rng: &mut Rng) -> Self {
        Self {
            phase_x: rng.range(0.0, std::f32::consts::TAU),
            phase_z: rng.range(0.0, std::f32::consts::TAU),
        }
    }

    /// Wysokość gruntu. Zawsze `>= 0` — czołgi nie potrafią pływać.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let a = (x * 0.045 + self.phase_x).sin();
        let b = (z * 0.037 + self.phase_z).sin();
        let c = ((x + z) * 0.021).sin();
        // kombinacja fal o różnych długościach: niska i wysoka
        let h = 1.5 * a + 0.9 * b + 0.6 * c;
        h.max(0.0)
    }

    /// Nachylenie gruntu (normalna powierzchni) w punkcie.
    ///
    /// Różniczka centralna — taniej niż wzór analityczny, a przy tej
    /// skali terenu dokładność w zupełności wystarcza.
    pub fn normal(&self, x: f32, z: f32) -> Vec3 {
        let e = 0.5;
        let hx = self.height(x + e, z) - self.height(x - e, z);
        let hz = self.height(x, z + e) - self.height(x, z - e);
        Vec3::new(-hx, 2.0 * e, -hz).normalize_or_zero()
    }
}

/// Cały świat gry.
pub struct World {
    pub terrain: Terrain,
    pub player: Tank,
    pub enemies: Vec<Tank>,
    pub shells: Vec<Shell>,
    /// Iskry po trafieniu: `(pozycja, prędkość, czas życia)`.
    pub sparks: Vec<(Vec3, Vec3, f32)>,
    pub player_hits: u32,
    pub enemy_kills: u32,
    pub rng: Rng,
}

impl World {
    /// Tworzy świat: gracz na środku, `enemy_count` przeciwników
    /// rozsianych na pierścieniu (nie w jednym miejscu).
    pub fn new(enemy_count: usize) -> Self {
        let mut rng = Rng::new(0xC0FFEE);
        let terrain = Terrain::new(&mut rng);

        let player_pos = Vec3::new(0.0, terrain.height(0.0, 0.0), 30.0);
        let mut player = Tank::new(player_pos, Team::Player);
        // twarzą w stronę środka pierścienia, gdzie są przeciwnicy
        player.heading = std::f32::consts::PI;
        // lufa wyrównana z kadłubem — inaczej czołg stałby bokiem do
        // kierunku jazdy i kamera patrzyłaby na jego bok
        player.turret_yaw = player.heading;

        let mut enemies = Vec::with_capacity(enemy_count);
        for i in 0..enemy_count {
            let angle = i as f32 / enemy_count.max(1) as f32 * std::f32::consts::TAU;
            let radius = rng.range(45.0, ARENA_HALF - 15.0);
            let x = angle.cos() * radius;
            let z = angle.sin() * radius;
            let mut e = Tank::new(Vec3::new(x, terrain.height(x, z), z), Team::Enemy);
            e.heading = (player_pos.x - x).atan2(player_pos.z - z);
            enemies.push(e);
        }

        Self {
            terrain,
            player,
            enemies,
            shells: Vec::new(),
            sparks: Vec::new(),
            player_hits: 0,
            enemy_kills: 0,
            rng,
        }
    }

    /// Przyciąga czołg do terenu (tylko `World` zna teren).
    pub fn snap_to_ground(&self, tank: &mut Tank) {
        snap_to_ground(&self.terrain, tank);
    }

    /// Czy gra się skończyła.
    pub fn is_over(&self) -> bool {
        !self.player.is_alive()
    }

    /// Ile przeciwników zostało.
    pub fn enemies_left(&self) -> usize {
        self.enemies.iter().filter(|e| e.is_alive()).count()
    }

    /// Odpala pocisk gracza (przy naciśnięciu LPM).
    pub fn player_fire(&mut self) {
        if let Some(s) = self.player.fire() {
            self.shells.push(s);
        }
    }
}

/// Przyciąga czołg do powierzchni gruntu.
///
/// WOLNA funkcja, a nie metoda `World`, bo wywołujemy ją w pętli
/// `self.enemies.iter_mut()` — metoda brałaby `&self` i kolidowała
/// z pożyczką `&mut self.enemies`.
fn snap_to_ground(terrain: &Terrain, tank: &mut Tank) {
    tank.pos.y = terrain.height(tank.pos.x, tank.pos.z);
}

/// Trzyma czołg w granicach areny (ta sama uwaga co wyżej).
fn clamp_to_arena(tank: &mut Tank) {
    let r = tank.radius() * 0.6;
    tank.pos.x = tank.pos.x.clamp(-ARENA_HALF + r, ARENA_HALF - r);
    tank.pos.z = tank.pos.z.clamp(-ARENA_HALF + r, ARENA_HALF - r);
}

/// Zawijanie różnicy kątów do `[-PI, PI]`.
///
/// Bez tego czołg skręcałby w drugą stronę, gdy przeciwnik jest „za
/// plecami" — obrót o 350° zamiast o -10°.
fn wrap_angle(mut a: f32) -> f32 {
    while a > std::f32::consts::PI {
        a -= std::f32::consts::TAU;
    }
    while a < -std::f32::consts::PI {
        a += std::f32::consts::TAU;
    }
    a
}

/// Długość wektora w płaszczyźnie XZ.
fn flat_len(x: f32, z: f32) -> f32 {
    (x * x + z * z).sqrt()
}

/// Test trafienia punktu w kulę.
fn sphere_hit(point: Vec3, center: Vec3, radius: f32) -> bool {
    (point - center).length() <= radius
}

impl World {
    /// AI przeciwników: podjeżdża do gracza, obraca wieżę, strzela.
    fn update_ai(&mut self, dt: f32) {
        // Stan świata kopiujemy do zmiennych PRZED pożyczką `enemies`.
        // `self.enemies.iter_mut()` trzyma `&mut self.enemies`, więc
        // każde wywołanie `self.shells.push(..)` w pętli byłoby drugą
        // pożyczką tego samego `&mut self` — kompilator to odrzuci.
        let player_pos = self.player.pos;
        let player_turret = self.player.turret_position();
        let player_alive = self.player.is_alive();
        let mut shells: Vec<Shell> = Vec::new();

        for e in self.enemies.iter_mut() {
            if !e.is_alive() {
                continue;
            }
            let to_player = player_pos - e.pos;
            let dist = flat_len(to_player.x, to_player.z);

            // --- kadłub: skręcamy w stronę gracza
            let steer = wrap_angle(to_player.x.atan2(to_player.z) - e.heading).clamp(-1.0, 1.0);
            // podjeżdżamy z daleka, cofamy się w zwarciu (inaczej wbijają
            // się w gracza i blokują mu strzał)
            let throttle = if dist > 25.0 { 0.55 } else { -0.25 };
            e.update(dt, throttle, steer);

            // --- wieża: celujemy w gracza z ograniczoną prędkością obrotu
            let aim = player_turret - e.turret_position();
            let tdiff = wrap_angle(aim.x.atan2(aim.z) - e.turret_yaw);
            // maks. 2 rad/s — daje graczowi czas na reakcję i celowanie
            e.aim(tdiff.clamp(-2.0 * dt, 2.0 * dt), 0.0);
            // celujemy w kadłub (niżej niż wieża), więc lekki pitch w dół
            let want_pitch = (aim.y - 0.4).atan2(dist.max(0.1));
            e.aim(0.0, (want_pitch - e.gun_pitch).clamp(-0.8 * dt, 0.8 * dt));

            // --- ogień: w zasięgu, wycelowany i gotowy
            if dist < e.config.range && tdiff.abs() < 0.12 && player_alive && e.can_fire() {
                if let Some(s) = e.fire() {
                    shells.push(s);
                }
            }
        }
        self.shells.append(&mut shells);

        // przyciąganie do terenu dopiero po zwolnieniu pożyczki `enemies`
        let terrain = self.terrain;
        for e in self.enemies.iter_mut() {
            clamp_to_arena(e);
            snap_to_ground(&terrain, e);
        }
    }

    /// Krok pocisków: ruch, kolizje z czołgami i ziemią.
    fn update_shells(&mut self, dt: f32) {
        // Całą listę zabieramy przez `std::mem::take`, żeby pętla nie
        // pożyczała `&mut self.shells` w chwili, gdy wołamy `add_spark`.
        let mut incoming: Vec<Shell> = std::mem::take(&mut self.shells);
        let mut alive: Vec<Shell> = Vec::with_capacity(incoming.len());

        for mut s in incoming.drain(..) {
            s.ttl -= dt;
            if s.ttl <= 0.0 {
                continue;
            }
            s.pos += s.vel * dt;

            // --- kolizja z ziemią
            if s.pos.y <= self.terrain.height(s.pos.x, s.pos.z) {
                self.add_spark(s.pos, 0.55);
                continue;
            }

            // --- pocisk gracza: szukamy trafienia w przeciwnika
            if s.from_player {
                let mut hit = false;
                for e in self.enemies.iter_mut() {
                    if !e.is_alive() {
                        continue;
                    }
                    if sphere_hit(s.pos, e.pos, e.radius() * 0.9) {
                        let dmg = e.config.damage;
                        let destroyed = e.take_damage(dmg);
                        if destroyed {
                            self.enemy_kills += 1;
                        } else {
                            self.player_hits += 1;
                        }
                        hit = true;
                        break;
                    }
                }
                if hit {
                    self.add_spark(s.pos, 1.0);
                    continue;
                }
            } else if self.player.is_alive()
                && sphere_hit(s.pos, self.player.pos, self.player.radius() * 0.9)
            {
                // --- pocisk przeciwnika trafia gracza
                let dmg = self.player.config.damage;
                self.player.take_damage(dmg);
                self.add_spark(s.pos, 1.0);
                continue;
            }

            alive.push(s);
        }

        self.shells = alive;
    }

    /// Iskry po trafieniu: kilka cząstek w losowych kierunkach.
    fn add_spark(&mut self, pos: Vec3, intensity: f32) {
        for _ in 0..6 {
            let v = Vec3::new(
                self.rng.range(-1.0, 1.0),
                self.rng.range(0.2, 1.4),
                self.rng.range(-1.0, 1.0),
            ) * (2.0 * intensity);
            self.sparks.push((pos, v, 0.45 + 0.3 * self.rng.f32()));
        }
    }

    fn update_sparks(&mut self, dt: f32) {
        self.sparks.retain_mut(|(pos, vel, life)| {
            *life -= dt;
            *pos += *vel * dt;
            vel.y -= 9.0 * dt; // grawitacja
            *life > 0.0
        });
    }

    /// Pełny klatka świata. `throttle`/`steer` dotyczą gracza.
    pub fn update(&mut self, dt: f32, throttle: f32, steer: f32) {
        self.player.update(dt, throttle, steer);
        clamp_to_arena(&mut self.player);
        snap_to_ground(&self.terrain, &mut self.player);
        self.update_ai(dt);
        self.update_shells(dt);
        self.update_sparks(dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..10 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }

    #[test]
    fn rng_f32_is_in_unit_interval() {
        let mut r = Rng::new(7);
        for _ in 0..100 {
            let v = r.f32();
            assert!((0.0..1.0).contains(&v), "wartość {v} poza [0,1)");
        }
    }

    #[test]
    fn terrain_is_never_below_zero() {
        let mut rng = Rng::new(1);
        let t = Terrain::new(&mut rng);
        for i in 0..200 {
            let x = (i as f32) * 1.7 - 170.0;
            for j in 0..50 {
                let z = (j as f32) * 3.1 - 75.0;
                assert!(t.height(x, z) >= 0.0, "teren zszedł poniżej zera");
            }
        }
    }

    #[test]
    fn terrain_normal_points_up() {
        let mut rng = Rng::new(3);
        let t = Terrain::new(&mut rng);
        for i in 0..100 {
            let n = t.normal((i as f32) * 2.0 - 100.0, 5.0);
            assert!(n.y > 0.0, "normalna terenu nie wskazuje w górę: {n:?}");
        }
    }

    #[test]
    fn tanks_start_inside_arena_and_on_ground() {
        let w = World::new(4);
        assert!(w.player.pos.x.abs() < ARENA_HALF && w.player.pos.z.abs() < ARENA_HALF);
        let g = w.terrain.height(w.player.pos.x, w.player.pos.z);
        assert!((w.player.pos.y - g).abs() < 1e-3, "gracz leży w ziemi");
        for e in &w.enemies {
            assert!(e.pos.x.abs() < ARENA_HALF && e.pos.z.abs() < ARENA_HALF);
            let g = w.terrain.height(e.pos.x, e.pos.z);
            assert!((e.pos.y - g).abs() < 1e-3, "przeciwnik leży w ziemi");
        }
    }

    #[test]
    fn player_stays_in_arena_when_driving_at_wall() {
        let mut w = World::new(2);
        // pchamy gracza w jednym kierunku przez 20 s symulacji
        for _ in 0..400 {
            w.update(0.05, 1.0, 0.0);
        }
        assert!(
            w.player.pos.x.abs() <= ARENA_HALF && w.player.pos.z.abs() <= ARENA_HALF,
            "gracz wydostał się z areny: {:?}",
            w.player.pos
        );
    }

    #[test]
    fn ai_closes_distance_over_time() {
        let mut w = World::new(3);
        let before = w
            .enemies
            .iter()
            .map(|e| flat_len(e.pos.x - w.player.pos.x, e.pos.z - w.player.pos.z))
            .fold(0.0f32, f32::max);
        for _ in 0..200 {
            w.update(0.05, 0.0, 0.0);
        }
        let after = w
            .enemies
            .iter()
            .map(|e| flat_len(e.pos.x - w.player.pos.x, e.pos.z - w.player.pos.z))
            .fold(0.0f32, f32::max);
        assert!(after < before, "AI nie zbliżyło się: {before} -> {after}");
    }

    #[test]
    fn player_fire_creates_a_shell() {
        let mut w = World::new(1);
        w.player_fire();
        assert_eq!(w.shells.len(), 1, "strzał nie utworzył pocisku");
        assert!(w.shells[0].from_player);
    }

    #[test]
    fn shells_expire() {
        let mut w = World::new(1);
        w.player_fire();
        for _ in 0..100 {
            w.update(0.05, 0.0, 0.0);
        }
        assert!(w.shells.is_empty(), "pociski nie zniknęły");
    }

    #[test]
    fn wrap_angle_stays_in_range() {
        assert!(wrap_angle(0.5).abs() <= std::f32::consts::PI);
        assert!((wrap_angle(std::f32::consts::TAU + 0.3) - 0.3).abs() < 1e-4);
        assert!((wrap_angle(-std::f32::consts::TAU - 0.3) + 0.3).abs() < 1e-4);
    }

    #[test]
    fn hit_survives_many_seconds_of_combat() {
        // pełna symulacja walki: nikt nie może „uciec" z areny,
        // pociski muszą znikać, a świat nie może się posypać
        let mut w = World::new(3);
        for _ in 0..600 {
            if w.player.can_fire() {
                w.player_fire();
            }
            w.update(0.05, 0.6, 0.2);
        }
        for e in &w.enemies {
            assert!(e.pos.x.abs() < ARENA_HALF + 1.0 && e.pos.z.abs() < ARENA_HALF + 1.0);
        }
        assert!(w.sparks.len() < 500, "iskry się nie sprzątają: {}", w.sparks.len());
    }
}