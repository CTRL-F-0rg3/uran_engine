//! Boss: zdrowie, fazy i zestaw ataków.
//!
//! Świadomie **bez silnika fizyki**: ataki są analityczne, a cała
//! walka mieści się w kilkunastu wzorcach. Podejście „maszyna stanów"
//! zamiast AI opartego na heurystyce daje **czytelność** — gracz musi
//! rozpoznać, że nadciąga „tsunami", bo to jedyna metoda, żeby się
//! ratować.
//!
//! # Fazy
//!
//! Boss ma trzy fazy, progi zależą od procenta zdrowia. Każda faza
//! **dodaje** ataki i skraca przerwy, ale nie usuwa poprzednich —
//! gracz musi pamiętać wszystko, czego się nauczył. Zmiana fazy daje
//! krótką przerwę i wyraźny sygnał dźwiękowy/wizualny, żeby gracz
//! zdążył zareagować.

use uran_math::{Rect, Vec2};

use crate::projectile::{BULLET_LIFE, Owner, Projectile};

/// Aktywny atak bossa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attack {
    /// Nic — boss się przygotowuje.
    Idle,
    /// Salwa pocisków wachlarzem w stronę gracza.
    Fan,
    /// Deszcz pocisków z góry w losowych miejscach.
    Rain,
    /// Fala uderzająca po ziemi — trzeba przeskoczyć.
    GroundSlam,
    /// Skok do gracza ze zderzeniem.
    Charge,
    /// Spiralny strzał — pociski rozchodzą się z dwóch ramion.
    Spiral,
}

impl Attack {
    /// Czy atak trwa wystarczająco długo, by go zobaczyć.
    pub fn is_telegraphed(self) -> bool {
        matches!(self, Attack::GroundSlam | Attack::Charge)
    }
}

/// Prosta maszyna losująca (xorshift32) — bez zewnętrznej zależności.
#[derive(Debug, Clone, Copy)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        // `0` to punkt stały xorshifta, więc wpychamy na niego 1.
        Self(if seed == 0 { 0x1234_5678 } else { seed })
    }

    /// Liczba z przedziału `[0, 1)`.
    pub fn f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x & 0x00FF_FFFF) as f32 / 0x00FF_FFFF as f32
    }

    /// Liczba z przedziału `[min, max)`.
    pub fn range(&mut self, min: f32, max: f32) -> f32 {
        min + self.f32() * (max - min)
    }

    /// Liczba całkowita z przedziału `[0, n)`.
    pub fn below(&mut self, n: u32) -> u32 {
        (self.f32() * n as f32) as u32
    }
}

/// Faza walki (1..=3).
pub const PHASE_COUNT: u8 = 3;

/// Parametry bossa.
#[derive(Debug, Clone, Copy)]
pub struct BossConfig {
    pub max_hp: f32,
    /// Proporcja zdrowia, poniżej której wchodzimy w fazę 2 i 3.
    pub phase2_at: f32,
    pub phase3_at: f32,
    /// Czas „namysłu" między atakami w fazie 1 (s).
    pub think_time: [f32; PHASE_COUNT as usize],
    /// Szerokość i wysokość ciała bossa.
    pub size: Vec2,
}

impl Default for BossConfig {
    fn default() -> Self {
        Self {
            max_hp: 1200.0,
            phase2_at: 0.66,
            phase3_at: 0.33,
            // Faza 1 daje graczowi czas na naukę, faza 3 prawie go
            // nie zostawia. To kluczowa krzywa trudności.
            think_time: [2.6, 1.9, 1.25],
            size: Vec2::new(150.0, 190.0),
        }
    }
}

/// Stan bossa w klatce.
#[derive(Debug, Clone, Copy)]
pub struct Boss {
    /// Środek ciała.
    pub pos: Vec2,
    /// Środek ciała w poprzedniej klatce — do testów „wejścia".
    pub prev_pos: Vec2,
    pub hp: f32,
    /// 1..=3 — zmienia się przy progu zdrowia.
    pub phase: u8,
    /// Aktualny atak.
    pub attack: Attack,
    /// Ile sekund do następnego ataku.
    pub timer: f32,
    /// Ile seksekund trwa bieżący atak.
    pub attack_time: f32,
    /// Wychył: ile zostało do wychyłu (s). Zerowe = brak wychyłu.
    pub windup: f32,
    /// Czy boss zmierza do gracza (charge).
    pub charging: bool,
    /// Ziarno losowania — powtarzalne walki ułatwiają testy.
    pub rng: Rng,
    /// Czy boss martwy.
    pub dead: bool,
    /// Animacja śmierci (s).
    pub death_time: f32,
}

impl Boss {
    /// Czy boss właśnie się wychyla przed atakiem.
    ///
    /// Osobny bool trzymany w strukturze rozjeżdżałby się z
    /// `windup` w dwa miejsca; stan czytamy z jednego źródła.
    pub fn winding(&self) -> bool {
        self.windup > 0.0
    }

    pub fn new(pos: Vec2, cfg: &BossConfig) -> Self {
        Self {
            pos,
            prev_pos: pos,
            hp: cfg.max_hp,
            phase: 1,
            attack: Attack::Idle,
            timer: 1.5,
            attack_time: 0.0,
            windup: 0.0,
            charging: false,
            rng: Rng::new(0xC0FFEE),
            dead: false,
            death_time: 0.0,
        }
    }

    /// Prostokąt ciała bossa.
    pub fn rect(&self, cfg: &BossConfig) -> Rect {
        Rect::from_center(self.pos, cfg.size)
    }

    /// Zadaje bossowi obrażenia. Zdrowie nie spada poniżej zera.
    pub fn take_damage(&mut self, amount: f32, cfg: &BossConfig) {
        if self.dead {
            return;
        }
        self.hp = (self.hp - amount).max(0.0);
        if self.hp <= 0.0 {
            self.hp = 0.0;
            self.dead = true;
            self.attack = Attack::Idle;
            self.charging = false;
            self.windup = 0.0;
            self.death_time = 0.0;
            let _ = cfg;
        }
    }

    /// Proporcja zdrowia 0..1.
    pub fn hp_ratio(&self, cfg: &BossConfig) -> f32 {
        (self.hp / cfg.max_hp).clamp(0.0, 1.0)
    }

    /// Czy boss jest martwy.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Wybiera następny atak — dobór zależy od fazy.
    ///
    /// Lista dozwolonych ataków **rośnie** z fazą, a losowanie
    /// ważyone (`Fan` jest częstszy w fazie 1, `Spiral` pojawia się
    /// dopiero w fazie 3). Dzięki temu gracz poznaje ataki stopniowo
    /// zamiast uczyć się pięciu wzorców naraz.
    fn choose_attack(&mut self, phase: u8) -> Attack {
        // Wagi: (atak, waga). Suma wag w danej fazie bywa różna —
        // to zamierzone, względne wagi wystarczą.
        let pool: &[(Attack, u32)] = match phase {
            1 => &[(Attack::Fan, 5), (Attack::GroundSlam, 3), (Attack::Rain, 2)],
            2 => &[
                (Attack::Fan, 4),
                (Attack::GroundSlam, 3),
                (Attack::Rain, 3),
                (Attack::Charge, 3),
            ],
            _ => &[
                (Attack::Fan, 3),
                (Attack::GroundSlam, 2),
                (Attack::Rain, 2),
                (Attack::Charge, 3),
                (Attack::Spiral, 2),
            ],
        };
        let total: u32 = pool.iter().map(|(_, w)| w).sum();
        let mut pick = self.rng.below(total);
        for (a, w) in pool {
            if pick < *w {
                return *a;
            }
            pick -= *w;
        }
        pool[0].0
    }

    /// Krok symulacji bossa. Zwraca pociski, które boss wystrzelił.
    ///
    /// Podejście „funkcja zwraca pociski" zamiast „boss trzyma listę"
    /// jest celowe: [`Boss`] zostaje czystym typem skopiowalnym
    /// (`Copy`), a stan świata — pociski — żyje w `Game`. Dzięki temu
    /// testy bossa nie potrzebują świata ani renderer.
    pub fn step(&mut self, dt: f32, player_pos: Vec2, cfg: &BossConfig, out: &mut Vec<Projectile>) {
        if self.dead {
            self.death_time += dt;
            return;
        }
        self.prev_pos = self.pos;
        let dt = dt.clamp(0.0, 0.05);

        // --- zmiana fazy po przekroczeniu progu zdrowia ---
        let ratio = self.hp_ratio(cfg);
        let new_phase = if ratio <= cfg.phase3_at {
            3
        } else if ratio <= cfg.phase2_at {
            2
        } else {
            1
        };
        if new_phase > self.phase {
            self.phase = new_phase;
            // Krótka przerwa na zmianę fazy: natychmiastowy atak po
            // progu zdrowia byłby nieczytelny, bo gracz nie zdąży
            // zobaczyć, co się stało.
            self.attack = Attack::Idle;
            self.timer = 0.9;
            self.charging = false;
            self.windup = 0.0;
            return;
        }

        // --- wychył (telegraph) ---
        //
        // Przed atakami, które trzeba **zobaczyć** (`GroundSlam`,
        // `Charge`) boss stoi nieruchomo i „nabiera". Bez tego
        // gracz nie ma szans odgadnąć, że nadciąga fala.
        if self.windup > 0.0 {
            self.windup -= dt;
            if self.windup <= 0.0 {
                self.fire(self.attack, player_pos, cfg, out);
                self.attack_time = 0.45;
            }
            return;
        }

        // --- trwający atak ---
        if self.attack != Attack::Idle && self.attack_time > 0.0 {
            self.attack_time -= dt;
            // Niektóre ataki „strzelają" w trakcie, a nie jednym
            // strzałem — stąd osobny strzał w każdej klatce.
            match self.attack {
                Attack::Spiral | Attack::Rain => {
                    self.fire(self.attack, player_pos, cfg, out);
                }
                _ => {}
            }
            if self.attack_time <= 0.0 {
                self.attack = Attack::Idle;
                self.timer = cfg.think_time[(self.phase - 1) as usize];
            }
            return;
        }

        // --- charge: ruch w stronę gracza ---
        if self.charging {
            let dir = (player_pos - self.pos).normalize_or_zero();
            self.pos += dir * CHARGE_SPEED * dt;
            if self.attack_time <= 0.0 {
                self.charging = false;
                self.attack = Attack::Idle;
                self.timer = cfg.think_time[(self.phase - 1) as usize];
            }
            return;
        }

        // --- odliczanie do następnego ataku ---
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.attack = self.choose_attack(self.phase);
            if self.attack.is_telegraphed() {
                // Atak z wychyłem: zapowiadamy i czekamy.
                self.windup = WINDUP_TIME;
            } else if self.attack == Attack::Charge {
                self.charging = true;
                self.attack_time = 0.9;
            } else {
                self.fire(self.attack, player_pos, cfg, out);
                self.attack_time = 0.45;
            }
        }
    }
}

/// Prędkość bossa w ataku `Charge`.
pub const CHARGE_SPEED: f32 = 330.0;
/// Czas wychyłu przed atakiem (s) — tyle gracz ma na reakcję.
pub const WINDUP_TIME: f32 = 0.65;

/// Buduje pocisk bossa wychodzący z jego środka.
fn spawn(out: &mut Vec<Projectile>, from: Vec2, dir: Vec2, speed: f32, damage: f32, size: f32) {
    out.push(Projectile {
        pos: from,
        vel: dir.normalize_or_zero() * speed,
        owner: Owner::Boss,
        damage,
        size: Vec2::splat(size),
        life: BULLET_LIFE,
        piercing: false,
    });
}

impl Boss {
    /// Wystrzała pociski zależnie od ataku.
    ///
    /// Rozmieszczenie pocisków **jest** rozgrywką: gracz musi znaleźć
    /// lukę, więc kąty i prędkości są dobrane tak, by przesunięcie o
    /// kafel wystarczało — za wolne pociski dają przewagę, za szybkie
    /// zabijają bez szansy na reakcję.
    fn fire(
        &mut self,
        attack: Attack,
        player_pos: Vec2,
        _cfg: &BossConfig,
        out: &mut Vec<Projectile>,
    ) {
        // Punkt startowy: środek bossa, przesunięty lekko w stronę
        // gracza, żeby pocisk nie wychodził z wnętrza ciała.
        let origin = self.pos;
        match attack {
            Attack::Idle | Attack::Charge => {}

            Attack::Fan => {
                // Wachlarz 5 pocisków skierowanych na gracza, rozłożonych
                // na 50°. Węższy wachlarz = łatwiej przeczytać.
                let aim = (player_pos - origin).normalize_or_zero();
                let base = aim.x.atan2(aim.y);
                for i in 0..5 {
                    let a = base + (i as f32 - 2.0) * 0.22;
                    spawn(out, origin, Vec2::new(a.cos(), a.sin()), 210.0, 8.0, 11.0);
                }
            }

            Attack::Rain => {
                // Deszcz: pociski padają w losowych miejscach w okolicy
                // gracza. Generator jest seeded, więc deszcz jest
                // powtarzalny — gracz może się nauczyć wzorca.
                for _ in 0..3 {
                    let x = player_pos.x + self.rng.range(-260.0, 260.0);
                    let y = player_pos.y + self.rng.range(220.0, 380.0);
                    spawn(out, Vec2::new(x, y), Vec2::new(0.0, -1.0), 300.0, 9.0, 10.0);
                }
            }

            Attack::GroundSlam => {
                // Fala po ziemi: pociski jadą w obie strony od bossa,
                // po **ziemi** (wysyłamy je lekko w dół, bo gracz
                // musi przeskoczyć). To jedyny atak wymuszający skok.
                for s in [-1.0f32, 1.0] {
                    spawn(
                        out,
                        origin,
                        Vec2::new(s, -0.16).normalize_or_zero(),
                        250.0,
                        10.0,
                        16.0,
                    );
                    spawn(
                        out,
                        origin,
                        Vec2::new(s, -0.22).normalize_or_zero(),
                        250.0,
                        10.0,
                        16.0,
                    );
                }
            }

            Attack::Spiral => {
                // Dwa ramiona wirujące w przeciwnych fazach. Dzięki
                // temu w fuzie fazy 3 tworzą się luki, a w szczycie
                // gęsty mur — gracz musi znaleźć rytm, nie szczelinę.
                let t = self.death_time.max(self.attack_time);
                let base = (self.rng.f32() * 6.0) as f32;
                for k in 0..2 {
                    let a = base + t * 5.0 + k as f32 * std::f32::consts::PI;
                    spawn(out, origin, Vec2::new(a.cos(), a.sin()), 190.0, 7.0, 9.0);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn boss() -> (Boss, BossConfig) {
        (
            Boss::new(Vec2::new(900.0, 200.0), &BossConfig::default()),
            BossConfig::default(),
        )
    }

    /// Przechodzi `n` klatek i zbiera pociski.
    fn run(b: &mut Boss, n: usize, player: Vec2, cfg: &BossConfig) -> Vec<Projectile> {
        let mut out = Vec::new();
        for _ in 0..n {
            b.step(DT, player, cfg, &mut out);
        }
        out
    }

    #[test]
    fn fresh_boss_starts_in_phase_one_with_full_hp() {
        let (b, cfg) = boss();
        assert_eq!(b.phase, 1);
        assert_eq!(b.hp, cfg.max_hp);
        assert!(!b.is_dead());
        assert_eq!(b.hp_ratio(&cfg), 1.0);
    }

    #[test]
    fn boss_stays_quiet_during_the_first_think_time() {
        let (mut b, cfg) = boss();
        assert!(run(&mut b, 60, Vec2::ZERO, &cfg).is_empty());
    }

    #[test]
    fn boss_eventually_attacks_with_boss_owned_bullets() {
        let (mut b, cfg) = boss();
        let shots = run(&mut b, 60 * 5, Vec2::new(0.0, 100.0), &cfg);
        assert!(!shots.is_empty(), "boss nigdy nie zaatakował");
        assert!(
            shots.iter().all(|s| s.owner == Owner::Boss),
            "boss strzela pociskami wroga"
        );
    }

    #[test]
    fn damage_lowers_hp_and_changes_phase() {
        let (mut b, cfg) = boss();
        // Zdrowie spada do **dokładnie** 50%: poniżej progu fazy 2
        // (0,66), powyżej progu fazy 3 (0,33). Procenty mnożymy
        // przez `max_hp`, bo `take_damage` liczy w punktach.
        b.take_damage(cfg.max_hp * 0.5, &cfg);
        let ratio = b.hp_ratio(&cfg);
        assert!(ratio < cfg.phase2_at, "test nie doszedł do progu: {ratio}");
        assert!(ratio > cfg.phase3_at, "test spadł do fazy 3: {ratio}");
        b.step(DT, Vec2::ZERO, &cfg, &mut Vec::new());
        assert_eq!(b.phase, 2, "boss nie wszedł w fazę 2");
    }

    #[test]
    fn phase_change_gives_the_player_a_moment_to_react() {
        let (mut b, cfg) = boss();
        b.take_damage(cfg.max_hp * 0.4, &cfg);
        b.step(DT, Vec2::ZERO, &cfg, &mut Vec::new());
        assert!(run(&mut b, 5, Vec2::ZERO, &cfg).is_empty());
    }

    #[test]
    fn killing_the_boss_stops_all_attacks() {
        let (mut b, cfg) = boss();
        b.take_damage(cfg.max_hp, &cfg);
        assert!(b.is_dead());
        assert!(run(&mut b, 120, Vec2::ZERO, &cfg).is_empty());
    }

    #[test]
    fn hp_never_goes_below_zero() {
        let (mut b, cfg) = boss();
        b.take_damage(cfg.max_hp * 10.0, &cfg);
        assert_eq!(b.hp, 0.0);
        b.take_damage(100.0, &cfg);
        assert_eq!(b.hp, 0.0);
    }

    #[test]
    fn phase_three_adds_spiral() {
        let (mut b, cfg) = boss();
        b.take_damage(cfg.max_hp * 0.7, &cfg);
        b.step(DT, Vec2::ZERO, &cfg, &mut Vec::new());
        assert_eq!(b.phase, 3);
        // Spiral ma wagę 2 w fazie 3, więc w 40 s musi się pojawić choć
        // raz — inaczej faza 3 nie różniłaby się od fazy 2.
        let mut saw_spiral = false;
        for _ in 0..(60.0 * 40.0) as usize {
            b.step(DT, Vec2::new(0.0, 100.0), &cfg, &mut Vec::new());
            if b.attack == Attack::Spiral {
                saw_spiral = true;
                break;
            }
        }
        assert!(saw_spiral, "faza 3 nigdy nie użyła spirali");
    }

    #[test]
    fn telegraphed_attacks_have_a_windup() {
        assert!(Attack::GroundSlam.is_telegraphed());
        assert!(Attack::Charge.is_telegraphed());
        // Salwy nie wymagają wychyłu — gracz reaguje na same pociski.
        assert!(!Attack::Fan.is_telegraphed());
        assert!(!Attack::Rain.is_telegraphed());
    }

    #[test]
    fn charge_moves_the_boss_towards_the_player() {
        let (mut b, cfg) = boss();
        let start = b.pos;
        b.charging = true;
        b.attack_time = 0.9;
        for _ in 0..10 {
            b.step(DT, start + Vec2::new(-500.0, 0.0), &cfg, &mut Vec::new());
        }
        assert!(
            b.pos.x < start.x,
            "boss nie ruszył: {} -> {}",
            start.x,
            b.pos.x
        );
    }

    #[test]
    fn huge_dt_does_not_break_the_boss() {
        let (mut b, cfg) = boss();
        let mut out = Vec::new();
        for _ in 0..10 {
            b.step(3.0, Vec2::ZERO, &cfg, &mut out);
        }
        assert!(b.pos.is_finite(), "pozycja ma NaN: {:?}", b.pos);
        assert!(out.len() < 1000, "boss wystrzelił {} pocisków", out.len());
    }
}
