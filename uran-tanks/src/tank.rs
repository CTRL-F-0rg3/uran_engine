//! Czołg: stan, ruch, celowanie, strzał.
//!
//! Fizyka jest świadomie uproszczona (`uran-physics` 3D jeszcze nie ma —
//! crate jest pusty), ale zachowuje to, co w czołgówce najważniejsze: ciężkie
//! przyspieszanie, ograniczoną prędkość skrętu zależną od biegu (jak w
//! World of Tanks, gdzie na postoju skręcasz szybko, a w ruchu ledwo),
//! odrzut po strzale i przeładowanie.

use uran_math::Vec3;

use crate::geometry::tank_dim;

/// Strona w starciu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Team {
    Player,
    Enemy,
}

/// Parametry czołgu (tier 1 — wszystko w jednym).
#[derive(Debug, Clone, Copy)]
pub struct TankConfig {
    /// Maksymalna prędkość jazdy do przodu (jednostki świata / s).
    pub max_speed: f32,
    /// Prędkość cofania (mniejsza — cofanie jest powolne).
    pub reverse_speed: f32,
    /// Przyspieszenie pod gazem.
    pub accel: f32,
    /// Hamowanie / wytracanie biegu po puszczeniu gazu.
    pub brake: f32,
    /// Maksymalna prędkość obrotu kadłuba (rad/s) na postoju.
    pub turn_rate: f32,
    /// Mnożnik `turn_rate` przy pełnym biegu — czołgi na gąsiennicach
    /// skręcają na postoju, w ruchu mają ogon.
    pub turn_grip: f32,
    /// Czas przeładowania działa (s).
    pub reload: f32,
    /// Obrażenia pocisku.
    pub damage: f32,
    /// Zasięg strzału (tier 1: bez sprawdzania przeszkód).
    pub range: f32,
    /// Impuls odrzutu działa.
    pub recoil: f32,
    /// Maksymalne HP.
    pub max_hp: f32,
}

impl Default for TankConfig {
    fn default() -> Self {
        Self {
            max_speed: 18.0,
            reverse_speed: 8.0,
            accel: 9.0,
            brake: 14.0,
            turn_rate: 1.4,
            // na pełnym biegu skręt spada do 35% — „ogon ciągnie"
            turn_grip: 0.35,
            reload: 1.8,
            damage: 34.0,
            range: 70.0,
            recoil: 12.0,
            max_hp: 220.0,
        }
    }
}

/// Pocisk w locie (widoczny jako iskra).
#[derive(Debug, Clone, Copy)]
pub struct Shell {
    pub pos: Vec3,
    pub vel: Vec3,
    /// Czas życia — po nim pocisk przepada.
    pub ttl: f32,
    /// Czy pocisk należy do gracza (tylko jego rysujemy inaczej).
    pub from_player: bool,
}

/// Czołg: pozycja, kadłub, wieża, HP, przeładowanie.
#[derive(Debug, Clone)]
pub struct Tank {
    pub team: Team,
    pub pos: Vec3,
    /// Obrót kadłuba wokół Y (radiany).
    pub heading: f32,
    /// Prędkość liniowa wzdłuż osi kadłuba (ujemna = cofanie).
    pub speed: f32,
    /// Obrót wieży wokół Y, w świecie.
    pub turret_yaw: f32,
    /// Kąt podniesienia lufy: >0 = w górę.
    pub gun_pitch: f32,
    pub hp: f32,
    /// Czas do gotowości następnego strzału.
    pub reload_left: f32,
    pub config: TankConfig,
}

impl Tank {
    pub fn new(pos: Vec3, team: Team) -> Self {
        let config = TankConfig::default();
        Self {
            team,
            pos,
            heading: 0.0,
            speed: 0.0,
            turret_yaw: 0.0,
            gun_pitch: 0.0,
            hp: config.max_hp,
            reload_left: 0.0,
            config,
        }
    }

    pub fn is_alive(&self) -> bool {
        self.hp > 0.0
    }

    pub fn hp_ratio(&self) -> f32 {
        (self.hp / self.config.max_hp).clamp(0.0, 1.0)
    }

    pub fn is_reloading(&self) -> bool {
        self.reload_left > 0.0
    }

    /// Kierunek jazdy (jednostkowy) w świecie.
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.heading.sin(), 0.0, self.heading.cos())
    }

    /// Środek wieży w świecie.
    pub fn turret_position(&self) -> Vec3 {
        self.pos + Vec3::new(0.0, tank_dim::TURRET_Y, 0.0)
    }

    /// Kierunek, w który patrzy lufa (uwzględnia pochylenie).
    ///
    /// Obrót świata to `turret_yaw`, a wewnątrz niego `gun_pitch`.
    /// Oś lufy w przestrzeni obiektu to `(0, sin(pitch), cos(pitch))`.
    pub fn aim_dir(&self) -> Vec3 {
        let flat = Vec3::new(self.turret_yaw.sin(), 0.0, self.turret_yaw.cos());
        let pitch = self.gun_pitch;
        // skręt w górę: obracamy wektor płaski wokół osi X
        let right = flat.cross(Vec3::Y);
        (flat * pitch.cos() + right * pitch.sin()).normalize_or_zero()
    }

    /// Punkt wylotu pocisku (środek lufy).
    pub fn muzzle(&self) -> Vec3 {
        self.turret_position() + self.aim_dir() * (tank_dim::GUN_LEN - 0.2)
    }

    /// Promień czołgu (do kolizji i kamery).
    pub fn radius(&self) -> f32 {
        tank_dim::HULL_HALF_Z
    }

    /// Krok symulacji: gaz, skręt, przeładowanie, odrzut.
    ///
    /// `throttle` w zakresie `[-1, 1]`, `steer` w `[-1, 1]`.
    pub fn update(&mut self, dt: f32, throttle: f32, steer: f32) {
        if !self.is_alive() {
            self.speed = 0.0;
            return;
        }
        let c = self.config;

        // --- napęd
        let target = if throttle > 0.0 {
            throttle * c.max_speed
        } else {
            throttle * c.reverse_speed
        };
        if throttle.abs() < 0.01 {
            // brak gazu -> wytracanie biegu
            let drop = c.brake * dt;
            self.speed = if self.speed.abs() <= drop {
                0.0
            } else {
                self.speed - self.speed.signum() * drop
            };
        } else if (target - self.speed).abs() > 0.001
            && (target > 0.0) == (self.speed >= 0.0)
        {
            // przyspieszamy w tym samym kierunku co cel
            self.speed += (target - self.speed).signum() * c.accel * dt * throttle.abs();
        } else {
            // gaz przeciwny kierunkowi biegu (np. cofanie) -> hamowanie
            self.speed -= self.speed.signum() * c.brake * dt;
        }
        self.speed = self.speed.clamp(-c.reverse_speed, c.max_speed);

        // --- pozycja
        self.pos += self.forward() * (self.speed * dt);

        // --- obrót kadłuba: gąsiennice „trzymają" tylko słabo w ruchu
        let grip = if self.speed.abs() < 0.5 { 1.0 } else { c.turn_grip };
        self.heading += steer * c.turn_rate * grip * dt;

        // --- przeładowanie
        if self.reload_left > 0.0 {
            self.reload_left = (self.reload_left - dt).max(0.0);
        }
    }

    /// Obrót wieży myszką. Świadomie NIE ograniczamy obrotu względem
    /// kadłuba — pełny model z limitem zasłony to TODO na wyższy tier.
    pub fn aim(&mut self, d_yaw: f32, d_pitch: f32) {
        self.turret_yaw += d_yaw;
        // lufa nie może patrzeć w kosmos ani pod czołg
        self.gun_pitch = (self.gun_pitch + d_pitch).clamp(-0.15, 0.5);
    }

    /// Czy można strzelać.
    pub fn can_fire(&self) -> bool {
        self.is_alive() && !self.is_reloading()
    }

    /// Oddaje strzał: zwraca pocisk albo `None`, gdy nie gotowy.
    ///
    /// Odrzut działa TUTAJ, a nie w `update`: jest zdarzeniem pojedynczym.
    /// Gdyby był w `update`, czołg traciłby prędkość co klatkę i nigdy
    /// nie ruszyłby z miejsca.
    pub fn fire(&mut self) -> Option<Shell> {
        if !self.can_fire() {
            return None;
        }
        self.reload_left = self.config.reload;
        // odrzut w linii jazdy: pchamy czołg do tyłu
        self.speed = (self.speed - self.config.recoil).clamp(
            -self.config.reverse_speed,
            self.config.max_speed,
        );
        Some(Shell {
            pos: self.muzzle(),
            // pocisk leci z prędkością zasięgu; w kroku symulacji
            // `vel * dt` daje przemieszczenie na klatkę
            vel: self.aim_dir() * self.config.range,
            ttl: 1.2,
            from_player: self.team == Team::Player,
        })
    }

    /// Nakłada obrażenia. Zwraca `true`, gdy czołg został zniszczony.
    pub fn take_damage(&mut self, amount: f32) -> bool {
        if !self.is_alive() {
            return false;
        }
        self.hp -= amount;
        if self.hp <= 0.0 {
            self.hp = 0.0;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tank() -> Tank {
        Tank::new(Vec3::ZERO, Team::Player)
    }

    #[test]
    fn new_tank_is_alive_and_ready() {
        let t = tank();
        assert!(t.is_alive());
        assert!(!t.is_reloading(), "nowy czołg nie jest przeładowany");
        assert_eq!(t.hp, t.config.max_hp);
    }

    #[test]
    fn throttle_accelerates_forward() {
        let mut t = tank();
        t.update(0.5, 1.0, 0.0);
        assert!(t.speed > 0.0, "gaz do przodu nie działa: {}", t.speed);
        assert!(t.pos.z > 0.0, "czołg nie pojechał do przodu: {:?}", t.pos);
    }

    #[test]
    fn reverse_is_slower_than_forward() {
        let mut fwd = tank();
        fwd.update(4.0, 1.0, 0.0);
        let mut rev = tank();
        rev.update(4.0, -1.0, 0.0);
        assert!(rev.speed.abs() < fwd.speed.abs(), "cofanie nie powinno być szybsze");
        assert!(rev.pos.z < 0.0, "cofanie nie cofało");
    }

    #[test]
    fn speed_is_capped() {
        let mut t = tank();
        for _ in 0..200 {
            t.update(0.1, 1.0, 0.0);
        }
        assert!(
            t.speed <= t.config.max_speed + 0.01,
            "prędkość przekroczyła limit: {} > {}",
            t.speed,
            t.config.max_speed
        );
    }

    #[test]
    fn turning_is_slower_at_speed() {
        // to jest „chwyt" czołgówki: na postoju skrętasz, w ruchu nie
        let mut parked = tank();
        let mut moving = tank();
        // tylko `moving` rozpędza się do pełnego biegu; `parked` zostaje
        // z prędkością 0, więc ma pełny „chwyt" gąsiennic
        for _ in 0..20 {
            moving.update(0.1, 1.0, 0.0);
        }
        assert!(moving.speed.abs() > 0.5, "czołg nie ruszył: {}", moving.speed);

        let parked_before = parked.heading;
        parked.update(1.0, 0.0, 1.0);
        let moving_before = moving.heading;
        moving.update(1.0, 0.0, 1.0);

        let parked_delta = (parked.heading - parked_before).abs();
        let moving_delta = (moving.heading - moving_before).abs();
        assert!(
            parked_delta > moving_delta * 2.0,
            "na postoju ({parked_delta}) powinno skręcać się znacznie \
             szybciej niż w ruchu ({moving_delta})"
        );
    }

    #[test]
    fn recoil_only_happens_on_shot() {
        // odrzut w `update` co klatkę = czołg nigdy nie ruszy
        let mut t = tank();
        for _ in 0..10 {
            t.update(0.1, 1.0, 0.0);
        }
        let before = t.speed;
        t.update(0.1, 0.0, 0.0);
        // przy puszczeniu gazu prędkość może tylko maleć (wytracanie biegu)
        assert!(
            t.speed <= before,
            "prędkość wzrosła bez gazu: {before} -> {}",
            t.speed
        );

        // a sam odrzut po strzale jest zauważalny
        let mut t2 = tank();
        for _ in 0..10 {
            t2.update(0.1, 1.0, 0.0);
        }
        let cruising = t2.speed;
        t2.reload_left = 0.0;
        t2.fire();
        assert!(
            t2.speed < cruising,
            "strzał nie spowodował odrzutu: {cruising} -> {}",
            t2.speed
        );
    }

    #[test]
    fn firing_starts_reload_and_blocks_second_shot() {
        let mut t = tank();
        assert!(t.fire().is_some());
        assert!(t.is_reloading());
        assert!(t.fire().is_none(), "nie można strzelać w trakcie przeładowania");
    }

    #[test]
    fn reload_expires() {
        let mut t = tank();
        t.fire();
        t.update(t.config.reload + 0.1, 0.0, 0.0);
        assert!(!t.is_reloading(), "przeładowanie się nie skończyło");
        assert!(t.fire().is_some(), "po przeładowaniu można strzelać");
    }

    #[test]
    fn dead_tank_cannot_move_or_fire() {
        let mut t = tank();
        t.take_damage(t.config.max_hp + 1.0);
        assert!(!t.is_alive());
        assert!(t.fire().is_none());
        t.update(1.0, 1.0, 0.0);
        assert_eq!(t.speed, 0.0, "martwy czołg się nie porusza");
    }

    #[test]
    fn damage_never_goes_below_zero() {
        let mut t = tank();
        t.take_damage(10_000.0);
        assert_eq!(t.hp, 0.0, "HP spadło poniżej zera");
        assert!(!t.is_alive());
    }

    #[test]
    fn aim_pitch_is_clamped() {
        let mut t = tank();
        t.aim(0.0, 10.0);
        assert!(t.gun_pitch <= 0.51, "lufa patrzyła w kosmos: {}", t.gun_pitch);
        t.aim(0.0, -10.0);
        assert!(t.gun_pitch >= -0.16, "lufa patrzyła pod ziemię: {}", t.gun_pitch);
    }

    #[test]
    fn muzzle_is_in_front_of_the_tank() {
        let t = tank();
        let m = t.muzzle();
        // lufa musi być przed kadłubem (oś Z dodatnia przy heading=0)
        assert!(m.z > t.pos.z, "lufa jest z tyłu: {m:?}");
    }

    #[test]
    fn aim_dir_is_unit_length() {
        let mut t = tank();
        t.aim(0.7, 0.3);
        assert!(
            (t.aim_dir().length() - 1.0).abs() < 1e-4,
            "kierunek nie jest jednostkowy"
        );
    }

    #[test]
    fn hp_ratio_is_bounded() {
        let mut t = tank();
        assert!((t.hp_ratio() - 1.0).abs() < 1e-5);
        t.take_damage(t.config.max_hp / 2.0);
        assert!((t.hp_ratio() - 0.5).abs() < 1e-4);
    }
}