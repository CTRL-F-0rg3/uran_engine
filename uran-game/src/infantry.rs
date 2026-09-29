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
// `GpuSim`, `SimParams`, `SimStats`, `Team`, `UnitFlags` są w prelude silnika.
use uran_render::{GpuUnit, SimParams, Team, UnitFlags};

/// Łączna liczba żołnierzy obu drużyn.
///
/// To jest właśnie sprawdzian: czy płynność zależy od liczby jednostek,
/// czy od liczby *systemów* na CPU.
pub const ARMY_CAPACITY: usize = 100_000;

/// Zapas „rezerwy" (procent pojemności), którą wskrzesza klawisz F.
const RESERVE_PERCENT: usize = 20;

/// Ziarno losowości jednostki o indeksie `index` w globalnym buforze.
///
/// Używamy indeksu GŁÓWALNEGO (a nie numeru wewnątrz drużyny), bo obie
/// drużyny numerują się od zera — inaczej żołnierz 7 gracza i żołnierz 7
/// przeciwnika dostaliby identyczne ziarno, a armia wyglądałaby sztucznie
/// równo. Mieszamy z „złotą" stałą, żeby zbiory nie pokrywały się.
fn seed_of(index: usize) -> u32 {
    (index as u32) ^ 0x9E37_79B9
}

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
            // zasięg musi obejmować kilka rzędów szyku, inaczej jednostki
            // w środku kolumny nigdy nie widzą wroga i armia nie walczy
            attack_range: 14.0,
            attack_damage: 2.0,
            attack_cooldown: 0.45,
        }
    }
}

impl InfantryConfig {
    /// Pierwszy wolny indeks poza drużynami (początek rezerwy).
    pub fn reserve_start(&self) -> usize {
        self.friendly + self.enemy
    }

    /// Ta sama konfiguracja, ale o innej pojemności.
    ///
    /// Podział na drużyny i rezerwę liczymy OD NOWA — inaczej zmiana samego
    /// `capacity` zostawiłaby `friendly + enemy > capacity` i armia
    /// wyszłaby poza zaalokowany bufor.
    pub fn with_capacity(&self, capacity: usize) -> Self {
        let reserve = capacity * RESERVE_PERCENT / 100;
        let per_team = (capacity - reserve) / 2;
        Self {
            capacity,
            friendly: per_team,
            enemy: per_team,
            ..*self
        }
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
    ///
    /// CAŁY szyk musi zmieścić się w prostokącie `bounds` — inaczej
    /// skrajne kolumny wystawałyby poza mapę. Dlatego liczbę kolumn
    /// ograniczamy tak, aby ostatnia kolumna mieściła się w szerokości,
    /// a rozstaw dobieramy z obu osi naraz (węższa wygrywa).
    fn formation_point(
        &mut self,
        index: usize,
        count: usize,
        center: Vec2,
        bounds: Rect,
        min_spread: f32,
    ) -> Vec2 {
        if index * 4 < count * 3 {
            // ile kolumn zmieści się w poziomie przy minimalnym rozstawie
            let cols_by_width = ((bounds.width() / min_spread).floor() as usize).max(1);
            // ile kolumn zmieści się w pionie przy minimalnym rozstawie
            let rows_by_height = ((bounds.height() / min_spread).floor() as usize).max(1);
            // ile kolumn potrzebujemy, żeby zmieścić tyle a tyle rzędów
            let cols_needed = count.div_ceil(rows_by_height).max(1);
            let cols = cols_needed.min(cols_by_width).max(1);
            let rows = count.div_ceil(cols).max(1);

            // rozstaw liczony osobno dla każdej osi; bierzemy mniejszy,
            // żeby blok na pewno zmieścił się w OBU wymiarach
            let col_spread = if cols > 1 {
                bounds.width() / (cols as f32 - 1.0)
            } else {
                0.0
            };
            let row_spread = if rows > 1 {
                bounds.height() / (rows as f32 - 1.0)
            } else {
                0.0
            };
            let spread = col_spread.min(row_spread);

            // pozycja w szyku wyśrodkowana względem środka (o -0.5 kolumny/rzędu)
            let col = (index % cols) as f32 - (cols as f32 - 1.0) * 0.5;
            let row = (index / cols) as f32 - (rows as f32 - 1.0) * 0.5;
            let p = center + Vec2::new(col * spread, row * spread);
            // ostatnia linia: szyk mógł mieć mniej rzędów niż wyliczyliśmy,
            // więc przesuwamy z powrotem do środka, żeby nie zostawić
            // pustego przesunięcia na dole bloku
            let rows_actual = count.div_ceil(cols).max(1) as f32;
            let shift = (rows_actual - rows as f32) * 0.5 * spread;
            Vec2::new(p.x, p.y + shift)
        } else {
            // rozproszona bryła: elipsa dopasowana do prostokąta szyku
            let a = self.rand_range(0.0, std::f32::consts::TAU);
            let r = self.rand().sqrt();
            center
                + Vec2::new(
                    a.cos() * r * bounds.width() * 0.5,
                    a.sin() * r * bounds.height() * 0.5,
                )
        }
    }

    /// Buduje początkową armię do jednorazowego wgrania na GPU.
    ///
    /// Zwraca dokładnie `capacity` wpisów, w kolejności:
    /// `0..friendly` gracze, `friendly..friendly+enemy` przeciwnicy,
    /// dalej martwa rezerwka. Dzięki stałemu podziałowi indeksów
    /// `reinforce_range` wystarczy do wskrzeszania kolejnych partii.
    pub fn build_army(&mut self, arena: Rect) -> Vec<GpuUnit> {
        let cfg = self.config;
        let mut units = Vec::with_capacity(cfg.capacity);
        let min_spread = cfg.unit_size * 1.6;

        // poddani gracza: szyk przy punkcie zgrupowania (środek areny).
        // Margines 0.04 zostawiamy na promień żołnierza, żeby skrajne
        // jednostki nie wychodziły poza krawędź areny.
        let friendly_bounds = Rect::from_center(
            Vec2::ZERO,
            Vec2::new(arena.width() * 0.30, arena.height() * 0.56),
        );
        for i in 0..cfg.friendly {
            let p = self.formation_point(i, cfg.friendly, Vec2::ZERO, friendly_bounds, min_spread);
            units.push(GpuUnit::spawn(
                p,
                Vec2::ZERO,
                Team::Friendly,
                100.0,
                seed_of(i),
            ));
        }

        // przeciwnicy: dwa skrzydełka po bokach, żeby ruch formacji był widoczny.
        // Środek skrzydła trzymamy w połowie odstępu między skrzydłami, a sam
        // prostokąt ograniczamy do połowy tego odstępu — inaczej skrzydełka
        // nachodziłyby na siebie i wychodziłyby poza arenę.
        let wing_span = arena.width() * 0.30;
        let wing_size = Vec2::new(wing_span, arena.height() * 0.46);
        let wings = [
            Vec2::new(arena.center().x - wing_span * 0.5, arena.center().y),
            Vec2::new(arena.center().x + wing_span * 0.5, arena.center().y),
        ];
        for i in 0..cfg.enemy {
            let center = wings[i % wings.len()];
            let bounds = Rect::from_center(center, wing_size);
            let p = self.formation_point(i, cfg.enemy, center, bounds, min_spread);
            // ziarno z GLOBALNEGO indeksu slotu — dzięki temu przeciwnik
            // nie powtarza wzoru jasności gracza
            units.push(GpuUnit::spawn(
                p,
                Vec2::ZERO,
                Team::Enemy,
                100.0,
                seed_of(cfg.friendly + i),
            ));
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
        // Najpierw budujemy armię na CPU — `ctx.sim_mut()` pożycza cały
        // renderer, więc w trakcie tego pożyczania nie wolno sięgać po `ctx`.
        let army = self.build_army(arena);
        {
            let Some(sim) = ctx.sim_mut() else {
                return false;
            };
            *sim.params_mut() = SimParams::for_arena(arena, self.config.capacity, 8.0);
            let style = sim.style_mut();
            style.radius = self.config.unit_size;
            style.alpha = 0.95;
        }
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

        // Wartości z kontekstu czytamy PRZED pożyczeniem symulatora —
        // `sim_mut()` pożycza cały renderer na czas wywołania.
        let player_pos = ctx.camera.position;
        let Some(sim) = ctx.sim_mut() else { return };
        let cfg = self.config;
        let rally = self.rally;
        let p = sim.params_mut();
        p.dt = dt;
        p.time = self.time;
        p.player_pos = player_pos.to_array();
        p.rally = rally.to_array();
        p.max_speed = cfg.speed;
        // Waga odpychania po normalizacji kierunku (patrz `think` w sim.wgsl).
        // Za mało = jednostki wchodzą na siebie i armia zlewa się w jeden
        // punkt; za dużo = szyk się rozrywa na całą mapę. 0.5 = odpychanie
        // waży tyle co marsz, co utrzymuje formację w ruchu.
        p.separation = 0.5;
        p.separation_radius = cfg.unit_size * 2.2;
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
    ///
    /// `font` podajemy jawnie (a nie przez `Ctx::font`), bo to ta sama
    /// czcionka, którą gra już trzyma w swoim stanie.
    pub fn draw_hud(&self, ctx: &mut Ctx, font: Font, _size: Vec2) {
        let stats: SimStats = ctx.sim_stats();
        let alive = stats.total_alive();
        let samples = stats.samples;
        // przed pierwszym odczytem liczników nie pokazujemy zer
        let line = if samples == 0 {
            "Piechota — czekam na pierwsze statystyki z GPU…".to_string()
        } else {
            format!(
                "Piechota {} żołnierzy na GPU  ·  {} vs {} ({} żywych)  ·  {} strzałów  ·  {} poległo",
                self.config.capacity,
                stats.alive_friendly,
                stats.alive_enemy,
                alive,
                stats.shots,
                stats.kills,
            )
        };

        ctx.gfx
            .screen_space()
            .layer(101)
            .color(Color::from_hex(0x8FA6C4))
            .draw_text(font, &line, Vec2::new(24.0, 58.0), 16.0, TextAlign::Left);
        ctx.gfx.color(Color::from_hex(0x64789A)).draw_text(
            font,
            "RMB — nowy punkt zgrupowania    F — uzupełnienie z rezerwy",
            Vec2::new(24.0, 80.0),
            14.0,
            TextAlign::Left,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arena testowa — proporcje zbliżone do demo.
    fn arena() -> Rect {
        Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0))
    }

    /// Mała pojemność, żeby testy chodziły szybko — podział na drużyny
    /// przeliczamy od nowa, bo inaczej `friendly + enemy` przekroczyłoby
    /// pojemność (oba pola są niezależne).
    fn small_config() -> InfantryConfig {
        InfantryConfig::default().with_capacity(4_000)
    }

    #[test]
    fn config_splits_teams_and_reserve() {
        let c = InfantryConfig::default();
        assert_eq!(c.capacity, ARMY_CAPACITY);
        assert!(c.friendly > 0 && c.enemy > 0);
        assert_eq!(c.friendly, c.enemy, "demo startuje symetrycznie");
        // rezerwa to dokładnie RESERVE_PERCENT pojemności
        let reserve = c.capacity - c.reserve_start();
        assert_eq!(reserve, c.capacity * RESERVE_PERCENT / 100);
        // i nic nie wychodzi poza pojemność
        assert!(c.reserve_start() < c.capacity);
    }

    #[test]
    fn army_has_exactly_capacity_slots() {
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(arena());
        assert_eq!(army.len(), inf.config().capacity);
    }

    #[test]
    fn team_ranges_do_not_overlap() {
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(arena());
        let c = *inf.config();
        // podział indeksów: gracze, przeciwnicy, rezerwa
        for (i, u) in army.iter().enumerate() {
            let expected = if i < c.friendly {
                Team::Friendly
            } else if i < c.reserve_start() {
                Team::Enemy
            } else {
                // rezerwa nie jest jeszcze „w drużynie", ale musi mieć
                // przypisaną drużynę, żeby wskrzeszenie F działało
                Team::Friendly
            };
            assert_eq!(u.team(), expected, "zły slot {i}");
        }
    }

    #[test]
    fn reserve_starts_dead_and_active_units_are_alive() {
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(arena());
        let c = *inf.config();
        for (i, u) in army.iter().enumerate() {
            if i < c.reserve_start() {
                assert!(u.is_alive(), "slot {i} powinien startować żywy");
                assert_eq!(u.health, 100.0);
            } else {
                assert!(!u.is_alive(), "rezerwa w slocie {i} musi być martwa");
            }
        }
    }

    #[test]
    fn formation_stays_inside_the_arena() {
        let a = arena();
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(a);
        // wszystkie startujące jednostki muszą leżeć w arenie —
        // inaczej kamera ich nie pokaże, a shader zaciśnie je przy krawędzi
        for (i, u) in army.iter().enumerate().take(inf.config().reserve_start()) {
            let p = u.pos();
            assert!(
                p.x >= a.min.x && p.x <= a.max.x && p.y >= a.min.y && p.y <= a.max.y,
                "slot {i} poza areną: {p:?}"
            );
        }
    }

    #[test]
    fn formation_is_deterministic() {
        // ten sam stan początkowy musi dać identyczną armię, inaczej
        // porównywanie zrzutów ekranu nic nie znaczy
        let mut a = Infantry::new(small_config());
        let mut b = Infantry::new(small_config());
        assert_eq!(a.build_army(arena()), b.build_army(arena()));
    }

    #[test]
    fn units_are_not_stacked_on_one_spot() {
        // szyk ma rozłożyć jednostki, nie kłaść je w jednym punkcie
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(arena());
        let friendly: Vec<Vec2> = army[..inf.config().friendly]
            .iter()
            .map(|u| u.pos())
            .collect();
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        for p in &friendly {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
        }
        assert!(
            max_x - min_x > 100.0,
            "formacja nie rozłożyła się w poziomie (szerokość {})",
            max_x - min_x
        );
    }

    #[test]
    fn reinforce_batch_stays_within_reserve() {
        // partie uzupełnienia nigdy nie wychodzą poza rezerwę
        let c = small_config();
        let reserve = c.capacity - c.reserve_start();
        let batch = reserve / REINFORCE_BATCHES as usize;
        for which in 0..REINFORCE_BATCHES as usize {
            let from = c.reserve_start() + which * batch;
            let to = (from + batch).min(c.capacity);
            assert!(from >= c.reserve_start());
            assert!(to <= c.capacity);
            assert!(to > from, "partia {which} jest pusta");
        }
    }

    #[test]
    fn with_capacity_recomputes_the_split() {
        // Zmiana samego `capacity` przy niezależnych polach `friendly`/`enemy`
        // dałaby armię dłuższą niż bufor — dlatego mamy `with_capacity`.
        let c = InfantryConfig::default().with_capacity(10_000);
        assert_eq!(c.capacity, 10_000);
        assert_eq!(
            c.reserve_start() + (c.capacity - c.reserve_start()),
            c.capacity
        );
        assert!(c.friendly + c.enemy + (c.capacity - c.reserve_start()) == c.capacity);
        assert!(c.reserve_start() <= c.capacity);
    }

    #[test]
    fn every_slot_has_a_seed_for_shading() {
        // ziarno steruje wariacją jasności w shaderze; sloty dwóch
        // drużyn muszą się różnić, inaczej armia wygląda płasko
        let mut inf = Infantry::new(small_config());
        let army = inf.build_army(arena());
        let c = *inf.config();
        let f = &army[0..c.friendly];
        let e = &army[c.friendly..c.reserve_start()];
        assert_ne!(f[0].seed, e[0].seed, "obie drużyny mają to samo ziarno");
        // i w obrębie drużyny ziarna też muszą się różnić
        assert_ne!(f[0].seed, f[1].seed);
    }
}
