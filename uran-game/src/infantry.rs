//! Piechota — tysiące żołnierzy symulowanych wyłącznie na GPU.
//!
//! Cała armia żyje w jednym `storage` bufferze na karcie. CPU wykonuje
//! w klatce kilka operacji: aktualizuje punkt zgrupowania, pozycję
//! gracza i wysyła 96 B parametrów. Ani jedna pozycja żołnierza nie
//! wraca na CPU — HUD czyta wyłącznie cztery liczniki statystyk, które
//! GPU wystawia do asynchronicznego odczytu.
//!
//! Podział na drużyny to zwykły podział zakresu indeksów w buforze:
//! `0..friendly` to poddani gracza, dalej przeciwnicy, na końcu rezerwa.

use uran_engine::prelude::*;
use uran_render::{GpuSim, GpuUnit, SimParams, SimStats, Team, UnitFlags};

/// Łączna liczba żołnierzy obu drużyn.
///
/// To jest właśnie sprawdzian: czy płynność zależy od liczby jednostek,
/// czy od liczby *systemów* na CPU.
pub const ARMY_CAPACITY: usize = 100_000;

/// Zapas „rezerwy" (procent pojemności), którą wskrzesza klawisz F.
const RESERVE_PERCENT: usize = 20;

/// Ile żołnierzy startuje w jednej partii przy uzupełnieniu (1/5 rezerwy).
const REINFORCE_BATCHES: u32 = 5;

/// Konfiguracja armii.
#[derive(Debug, Clone, Copy)]
pub struct InfantryConfig {
    /// Pojemność bufora GPU (alokowana raz, przy starcie).
    pub capacity: usize,
    /// Ile żołnierzy startuje po stronie gracza.
    pub friendly: usize,
    /// Ile po stronie przeciwnika.
    pub enemy: usize,
    /// Promień pojedynczego żołnierza (jednostki świata).
    pub unit_size: f32,
    /// Prędkość marszu.
    pub speed: f32,
    /// Zasięg ostrzału.
    pub attack_range: f32,
    /// Obrażenia za strzał.
    pub attack_damage: f32,
    /// Czas między strzałami.
    pub attack_cooldown: f32,
}

impl Default for InfantryConfig {
    fn default() -> Self {
        let capacity = ARMY_CAPACITY;
        // po połowie na drużynę, reszta to rezerwa
        let reserve = capacity * RESERVE_PERCENT / 100;
        let per_team = (capacity - reserve) / 2;
        Self {
            capacity,
            friendly: per_team,
            enemy: per_team,
            unit_size: 2.6,
            speed: 78.0,
            attack_range: 9.0,
            attack_damage: 5.0,
            attack_cooldown: 0.45,
        }
    }
}

impl InfantryConfig {
    /// Pierwszy wolny indeks poza drużynami (początek rezerwy).
    pub fn reserve_start(&self) -> usize {
        self.friendly + self.enemy
    }
}

/// Stan dowodzenia armią po stronie gry.
pub struct Infantry {
    config: InfantryConfig,
    /// Punkt zgrupowania (RMB).
    rally: Vec2,
    /// Czas świata (animacja i szum w shaderze).
    time: f32,
    /// Ile uzupełnień się udało (do HUD).
    reinforcements: u32,
    rng_state: u32,
}

impl Infantry {
    pub fn new(config: InfantryConfig) -> Self {
        Self {
            config,
            rally: Vec2::ZERO,
            time: 0.0,
            reinforcements: 0,
            rng_state: 0x5eed_1234,
        }
    }

    pub fn config(&self) -> &InfantryConfig {
        &self.config
    }

    /// Punkt zgrupowania (potrzebny do rysowania markera).
    pub fn rally(&self) -> Vec2 {
        self.rally
    }

    /// Generator xorshift32 — powtarzalny, więc demo wygląda tak samo
    /// po każdym uruchomieniu (ułatwia porównywanie zrzutów).
    fn rand(&mut self) -> f32 {
        let mut x = self.rng_state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng_state = x;
        (x & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32
    }

    fn rand_range(&mut self, min: f32, max: f32) -> f32 {
        min + self.rand() * (max - min)
    }

    /// Formacja: szyk zwarty (kolumny) albo rozproszony.
    ///
    /// Zwarty szyk wygląda lepiej w boju i jest realistyczny; rozproszony
    /// jest szybszy przy ogromnej armii (mniej kolizji w siatce). Dajemy
    /// szykowi 3/4 armii, reszta rozproszona.
    fn formation_point(&mut self, index: usize, count: usize, center: Vec2, spread: f32) -> Vec2 {
        if index * 4 < count * 3 {
            let cols = 64usize;
            let col = (index % cols) as f32 - (cols as f32 - 1.0) * 0.5;
            let row = (index / cols) as f32;
            center + Vec2::new(col * spread, row * spread)
        } else {
            let a = self.rand_range(0.0, std::f32::consts::TAU);
            let r = spread * 3.0 * self.rand().sqrt();
            center + Vec2::new(a.cos() * r, a.sin() * r)
        }
    }

    /// Buduje początkową armię do jednorazowego wgrania na GPU.
    pub fn build_army(&mut self, arena: Rect) -> Vec<GpuUnit> {
        let cfg = self.config;
        let mut units = Vec::with_capacity(cfg.capacity);

        // poddani gracza: szyk przy punkcie zgrupowania
        for i in 0..cfg.friendly {
            let p = self.formation_point(i, cfg.friendly, Vec2::ZERO, cfg.unit_size * 1.6);
            units.push(GpuUnit::spawn(p, Vec2::ZERO, Team::Friendly, 100.0, i as u32));
        }
        // przeciwnicy: dwa skrzydełka po bokach, żeby ruch formacji był widoczny
        let wings = [
            Vec2::new(arena.center().x - arena.width() * 0.30, arena.center().y),
            Vec2::new(arena.center().x + arena.width() * 0.30, arena.center().y),
        ];
        for i in 0..cfg.enemy {
            let center = wings[i % wings.len()];
            let p = self.formation_point(i, cfg.enemy, center, cfg.unit_size * 1.6);
            units.push(GpuUnit::spawn(p, Vec2::ZERO, Team::Enemy, 100.0, i as u32));
        }
        // reszta bufora: martwa rezerwa do wskrzeszenia klawiszem F
        for i in cfg.reserve_start()..cfg.capacity {
            units.push(GpuUnit {
                position: [0.0; 2],
                velocity: [0.0; 2],
                target: [0.0; 2],
                health: 0.0,
                cooldown: 0.0,
                flags: UnitFlags::dead().with_team(Team::Friendly).0,
                seed: i as u32,
                _pad: [0.0; 2],
            });
        }
        units
    }

    /// Wgrywa armię i ustawia parametry symulacji (jednorazowo, przy starcie).
    ///
    /// Zwraca `false`, gdy gra nie włączyła symulacji GPU — wtedy demo
    /// po prostu nie ma piechoty, ale reszta działa.
    pub fn install(&mut self, ctx: &mut Ctx, arena: Rect) -> bool {
        let Some(sim) = ctx.sim_mut() else { return false };
        *sim.params_mut() = SimParams::for_arena(arena, self.config.capacity, 8.0);
        {
            let style = sim.style_mut();
            style.radius = self.config.unit_size;
            style.alpha = 0.95;
        }
        let army = self.build_army(arena);
        ctx.upload_army(&army);
        true
    }

    /// Dowodzenie: rozkazy gracza + wysyłka parametrów do GPU.
    pub fn update(&mut self, ctx: &mut Ctx, dt: f32) {
        self.time += dt;

        // rozkaz: prawy przycisk = nowy punkt zgrupowania
        if ctx.input.mouse_just_pressed(MouseButton::Right) {
            self.rally = ctx.mouse_world();
        }
        // F = uzupełnienie z rezerwy
        if ctx.input.just_pressed(Key::KeyF) {
            self.reinforce(ctx);
        }

        let Some(sim) = ctx.sim_mut() else { return };
        let cfg = self.config;
        let p = sim.params_mut();
        p.dt = dt;
        p.time = self.time;
        p.player_pos = ctx.camera.position.to_array();
        p.rally = self.rally.to_array();
        p.max_speed = cfg.speed;
        // odpychanie musi być mocne: 40k jednostek inaczej zlewa się
        // w jeden punkt w miejscu zgrupowania
        p.separation = 900.0;
        p.separation_radius = cfg.unit_size * 2.4;
        p.attack_range = cfg.attack_range;
        p.attack_damage = cfg.attack_damage;
        p.attack_cooldown = cfg.attack_cooldown;
        p.unit_size = cfg.unit_size;
    }

    /// Wskrzesza kolejną partię z rezerwy wokół punktu zgrupowania.
    fn reinforce(&mut self, ctx: &mut Ctx) {
        let cfg = self.config;
        let start_of_reserve = cfg.reserve_start();
        if start_of_reserve >= cfg.capacity {
            return;
        }
        let reserve = cfg.capacity - start_of_reserve;
        let batch = reserve / REINFORCE_BATCHES as usize;
        let which = (self.reinforcements % REINFORCE_BATCHES) as usize;
        let from = start_of_reserve + which * batch;
        let to = (from + batch).min(cfg.capacity);
        ctx.reinforce_range(from, to, self.rally, Team::Friendly);
        self.reinforcements += 1;
    }

    /// Rysuje marker punktu zgrupowania (w świecie, pod armią).
    pub fn draw_rally_marker(&self, ctx: &mut Ctx) {
        let pulse = 1.0 + 0.12 * (self.time * 3.0).sin();
        ctx.gfx
            .world_space()
            .layer(-5)
            .blend(BlendMode::Additive)
            .color(Color::from_hex(0x4FC3F7).with_alpha(0.55))
            .draw_rect(Rect::from_center(self.rally, Vec2::splat(22.0 * pulse)));
        ctx.gfx
            .color(Color::from_hex(0x4FC3F7).with_alpha(0.22))
            .draw_rect(Rect::from_center(self.rally, Vec2::splat(70.0 * pulse)));
    }

    /// HUD armii. Statystyki pochodzą z GPU, więc mogą być kilka klatek
    /// stare — to cena asynchronicznego odczytu.
    pub fn draw_hud(&self, ctx: &mut Ctx) {
        let Some(font) = ctx.font else { return };
        let stats: SimStats = ctx.sim_stats();
        ctx.gfx
            .screen_space()
            .layer(101)
            .color(Color::from_hex(0x8FA6C4))
            .draw_text(
                font,
                &format!(
                    "Piechota {} żołnierzy na GPU  ·  {} vs {}  ·  {} strzałów  ·  {} poległo",
                    self.config.capacity,
                    stats.alive_friendly,
                    stats.alive_enemy,
                    stats.shots,
                    stats.kills
                ),
                Vec2::new(24.0, 58.0),
                16.0,
                TextAlign::Left,
            );
        ctx.gfx
            .color(Color::from_hex(0x64789A))
            .draw_text(
                font,
                "RMB — nowy punkt zgrupowania    F — uzupełnienie z rezerwy",
                Vec2::new(24.0, 80.0),
                14.0,
                TextAlign::Left,
            );
    }
}