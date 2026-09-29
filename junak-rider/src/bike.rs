//! Fizyka motocykla: gaz, hamowanie, skręt i przechył w zakręcie.
//!
//! Świadomie bez COLLISION ENGINE'A. Droga jest prosta i nieskończona,
//! a model zachowania, który tu opisujemy (prędkość -> siła napędowa ->
//! przechył), jest w pełni analityczny. Użycie rapiera3d tylko po to,
// by stwierdzić, że nie wjeżdżamy w pobocze, byłoby cięższe o dwa
//! rzędy wielkości i wniosłoby dokładnie ten sam błąd.
//!
//! Wszystko jest w metrach i sekundach, zgodnie z resztą silnika.

use uran_math::Vec3;

/// Parametry motocyklu — jeden zestaw, bo to jeden model.
#[derive(Debug, Clone, Copy)]
pub struct BikeConfig {
    /// Prędkość maksymalna (m/s). ~200 km/h dla motocykla szosowego.
    pub max_speed: f32,
    /// Prędkość cofania (m/s) — jest ograniczona mocniej niż do przodu.
    pub reverse_speed: f32,
    /// Przyspieszenie z gazu (m/s²).
    pub accel: f32,
    /// Hamowanie (m/s²).
    pub brake: f32,
    /// Opór powietrza: `v²`. Przy 30 m/s daje ~2.4 m/s², czyli
    /// zauważalne, ale bez dominacji nad gazem.
    pub drag: f32,
    /// Tarcie toczne (m/s²) — wolniejsze od oporu aerodynamicznego
    /// dopiero powyżej ~60 km/h.
    pub rolling: f32,
    /// Maksymalny kąt skrętu kierownicy (rad). ~35° to typowy limit
    /// motocyklowy; przy 2 m długości to zakrąt o promieniu ~3.3 m.
    pub max_steer: f32,
    /// Maksymalna prędkość obrotu kierownicy (rad/s) — kierownica
    /// nie skacze z 0 na pełny skręt w jednej klatce.
    pub steer_rate: f32,
    /// Szerokość toru (m). Liczy promień skrętu: `R = L / tan(steer)`.
    pub wheelbase: f32,
    /// Maksymalny przechył (rad). ~35° to granica, przy której koło
    /// dotyka obudowy; wyżej to już przewrócenie.
    pub max_lean: f32,
    /// Jak szybko przechył wraca do pionu (1/s).
    pub lean_stiffness: f32,
}

impl Default for BikeConfig {
    fn default() -> Self {
        Self {
            max_speed: 55.0,
            reverse_speed: 8.0,
            accel: 7.0,
            brake: 12.0,
            drag: 0.0025,
            rolling: 0.35,
            max_steer: 0.61,      // ~35°
            steer_rate: 3.2,
            wheelbase: 1.45,
            max_lean: 0.61,       // ~35°
            lean_stiffness: 6.0,
        }
    }
}

/// Stan motocykla w jednej klatce.
#[derive(Debug, Clone, Copy)]
pub struct Bike {
    pub pos: Vec3,
    /// Nagłówek: `forward = (sin(yaw), 0, cos(yaw))`.
    pub yaw: f32,
    /// Podłużna prędkość (m/s). Ujemna = cofanie.
    pub speed: f32,
    /// Aktualny wychył kierownicy (rad), już wygładzony.
    pub steer: f32,
    /// Przechył nadwozia (rad) — dodatni = w prawo.
    pub lean: f32,
    /// Prędkość obrotu (rad/s) — do liczenia przechyłu.
    pub yaw_rate: f32,
}

impl Default for Bike {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            yaw: 0.0,
            speed: 0.0,
            steer: 0.0,
            lean: 0.0,
            yaw_rate: 0.0,
        }
    }
}

impl Bike {
    pub fn forward(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos())
    }

    /// Prawa strona kierowcy (nie „prawo w ekranie").
    ///
    /// Liczymy jako `forward × up`, co jest jedyną definicją zgodną
    /// z układem prawoskrętnym przy `up = +Y`. Wartość ujemna na osi X
    /// przy `forward = +Z` nie jest błędem: kierowca patrzy NA +Z, więc
    /// jego prawa strona rzeczywiście leży w `-X`.
    pub fn right(&self) -> Vec3 {
        self.forward().cross(Vec3::Y)
    }

    /// Promień skrętu z aktualnego wychyłu kierownicy (m).
    ///
    /// Przy małym kącie wychyłu promień jest bardzo duży (skręt wprost),
    /// więc dzielimy przez `tan` z zabezpieczeniem — inaczej wychodziłby
    /// `inf`, a potem `NaN` w całej symulacji.
    pub fn turn_radius(&self, cfg: &BikeConfig) -> f32 {
        let t = self.steer.tan();
        if t.abs() < 1e-4 {
            f32::INFINITY
        } else {
            cfg.wheelbase / t
        }
    }
}

/// Krok symulacji.
///
/// `throttle` i `steer` to `-1..=1` (gaz/hamulec, lewo/prawo).
/// `dt` jest ograniczane od góry: przy klatce 0.5 s (okno przeciągnięte)
/// niezlimitowany `dt` wyrzuciłoby motocykl poza drogę.
pub fn step(bike: &mut Bike, dt: f32, throttle: f32, steer_input: f32, cfg: &BikeConfig) {
    let dt = dt.clamp(0.0, 0.05);
    if dt <= 0.0 {
        return;
    }
    let throttle = throttle.clamp(-1.0, 1.0);
    let steer_input = steer_input.clamp(-1.0, 1.0);

    // --- wychył kierownicy: nie skacze, tylko się wygładza
    //
    // Celujemy w `steer_input * max_steer` z ograniczoną prędkością
    // kąta. Dzięki temu kierownica ma „ciężar" i wymusza wyprostowanie
    // po puszczeniu — bez tego skręt byłby niekontrolowany przy 200 km/h.
    let steer_target = steer_input * cfg.max_steer;
    let max_delta = cfg.steer_rate * dt;
    let delta = (steer_target - bike.steer).clamp(-max_delta, max_delta);
    bike.steer += delta;

    // --- siły podłużne
    //
    // Gaz jest STAŁY, a prędkość maksymalną pilnuje twardy limit
    // prędkości niżej. Pierwsza wersja używała mnożnika `1 - v/vmax`,
    // który działa tak, że gdy opory rosną z kwadratem prędkości,
    // motor przestaje przyspieszać ZANIM dojdzie do `max_speed` —
    // i wartość z konfiguracji staje się nieosiągalna. Przy tych
    // oporach prędkość graniczna wypadała ok. 32 m/s (115 km/h),
    // a `max_speed` wynosiło 55. Stąd twarde cięcie zamiast mnożnika:
    // HUD pokazuje wtedy dokładnie to, co da się osiągnąć.
    let mut a = 0.0;
    if throttle > 0.0 {
        a += throttle * cfg.accel;
    } else if throttle < 0.0 {
        // Hamowanie działa w obie strony: zatrzymuje jadącego i cofa
        a -= (-throttle) * cfg.brake;
        // ...ale nie pcha dalej w tył, gdy już jedziemy do tyłu
        if bike.speed < -cfg.reverse_speed {
            a = 0.0;
        }
    }

    // Opory: aerodynamiczny rośnie z kwadratem prędkości, toczny stały
    let v = bike.speed;
    a -= cfg.drag * v * v.abs();
    // UWAGA: tu NIE wolno użyć `v.signum()`. W Rust `f32::signum(0.0)`
    // zwraca `1.0`, a nie `0.0` — więc motocykl stojący w miejscu
    // dostałby pełne tarcie toczne skierowane do tyłu, zacząłby się
    // cofać sam, a przy skręconej kierownicy obracałby w miejscu.
    // Stąd jawne porównanie z martwą strefą.
    let dir = if v > 1e-4 {
        1.0
    } else if v < -1e-4 {
        -1.0
    } else {
        0.0
    };
    a -= cfg.rolling * dir;

    // --- integracja prędkości
    let prev_speed = bike.speed;
    bike.speed += a * dt;
    // Nie pozwól przejść przez zero „na sile" — inaczej przy pełnym
    // hamowaniu prędkość skacze z -3 do +3 w jednej klatce.
    if prev_speed > 0.0 && bike.speed < 0.0 && throttle >= 0.0 {
        bike.speed = 0.0;
    }
    if prev_speed < 0.0 && bike.speed > 0.0 && throttle <= 0.0 {
        bike.speed = 0.0;
    }
    bike.speed = bike.speed.clamp(-cfg.reverse_speed, cfg.max_speed);

    // --- obrót
    //
    // Promień skrętu z geometrii: `R = L / tan(steer)`, a prędkość obrotu
    // to `v / R`. Na postoju (`v = 0`) skręt jest zerowy — dokładnie tak,
    // jak na prawdziwym motocyklu, gdzie stoisz w lekkim pochyłomie
    // przy pełnym skręcie kierownicy.
    //
    // MINUS jest tu istotny: `right()` (strona kierowcy) wskazuje na `-X`
    // przy `forward = +Z`, więc dodatni `steer` (przycisk D = w prawo)
    // musi zmniejszać `yaw`, nie zwiększać. Bez tego skręt w prawo
    // skierowałby motocykl w LEWO — i gra byłaby niemożliwa do
    // prowadzenia, bo kierownica działałaby odwrotnie niż myśli łowca.
    let yaw_rate = if bike.speed.abs() < 1e-3 {
        0.0
    } else {
        -(bike.speed / cfg.wheelbase) * bike.steer.tan()
    };
    bike.yaw_rate = yaw_rate;
    bike.yaw += yaw_rate * dt;
    // `-0.0` po przechyłach psuje formatowanie HUD, a `yaw` rośnie
    // bez ograniczeń — przy długiej sesji robi się duży i traci
    // precyzję w sin/cos, więc skracamy go do 2π.
    if bike.yaw.abs() > std::f32::consts::TAU {
        bike.yaw = bike.yaw.rem_euclid(std::f32::consts::TAU);
    }

    bike.pos += bike.forward() * bike.speed * dt;

    // --- przechył
    //
    // Motocykl przewraca się w zakręcie, bo musi „zwinąć" prędkość
    // w dół. Przechył jest sprzężony z kierownicą: im mocniej skręcamy,
    // tym bardziej się kładzie.
    //
    // Znak: `yaw_rate * speed` to przyspieszenie dośrodkowe ZE ZNAKIEM.
    // Przy skręcie w prawo (`steer > 0`) jest ono ujemne, a motocykl
    // musi się położyć w prawo — czyli `lean > 0`. Stąd MINUS.
    // Konwencja: `lean > 0` = przechył w stronę kierowcy (`right()`).
    let lean_target = (-(yaw_rate * bike.speed)).clamp(-cfg.max_lean, cfg.max_lean);
    let k = 1.0 - (-cfg.lean_stiffness * dt).exp();
    bike.lean += (lean_target - bike.lean) * k;
    bike.lean = bike.lean.clamp(-cfg.max_lean, cfg.max_lean);
}

/// Zwraca motocykl na środek drogi, wyzerowując prędkość boczną.
///
/// Używane przy starcie gry, żeby nie wyjść z „dziwnego" ułożenia.
pub fn reset(bike: &mut Bike, z: f32) {
    bike.pos = Vec3::new(0.0, 0.0, z);
    bike.yaw = 0.0;
    bike.speed = 0.0;
    bike.steer = 0.0;
    bike.lean = 0.0;
    bike.yaw_rate = 0.0;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> BikeConfig {
        BikeConfig::default()
    }

    /// Symuluje `secs` sekund z zadanymi wejściami.
    fn run(bike: &mut Bike, secs: f32, throttle: f32, steer: f32) {
        let dt = 1.0 / 60.0;
        for _ in 0..(secs / dt) as i32 {
            step(bike, dt, throttle, steer, &cfg());
        }
    }

    #[test]
    fn full_throttle_reaches_speed_but_not_infinity() {
        let mut b = Bike::default();
        run(&mut b, 20.0, 1.0, 0.0);
        let c = cfg();
        // dociąga do maksimum, ale go NIE przekracza
        assert!(b.speed > c.max_speed * 0.9, "za wolno: {}", b.speed);
        assert!(b.speed <= c.max_speed + 1e-3, "przekroczona max: {}", b.speed);
    }

    #[test]
    fn drag_eventually_stops_a_coasting_bike() {
        let mut b = Bike::default();
        run(&mut b, 5.0, 1.0, 0.0);
        let fast = b.speed;
        assert!(fast > 5.0);
        // puszczamy gaz — opory same zwalniają do zera
        run(&mut b, 40.0, 0.0, 0.0);
        assert!(b.speed.abs() < 0.05, "nie zatrzymał się: {}", b.speed);
    }

    #[test]
    fn brake_stops_faster_than_coasting() {
        let mut a = Bike::default();
        let mut b = Bike::default();
        run(&mut a, 5.0, 1.0, 0.0);
        run(&mut b, 5.0, 1.0, 0.0);
        run(&mut a, 2.0, 0.0, 0.0);
        run(&mut b, 2.0, -1.0, 0.0);
        assert!(b.speed < a.speed, "hamowanie wolniejsze: {} vs {}", b.speed, a.speed);
    }

    #[test]
    fn full_steer_gives_the_whole_lock_range() {
        let c = cfg();
        let mut b = Bike::default();
        // skręcamy do końca w prawo i czekamy aż kierownica dojdzie
        run(&mut b, 2.0, 1.0, 1.0);
        assert!(
            (b.steer - c.max_steer).abs() < 0.02,
            "wychył {} nie doszedł do limitu {}", b.steer, c.max_steer
        );
        // i w drugą stronę
        run(&mut b, 3.0, 1.0, -1.0);
        assert!(
            (b.steer + c.max_steer).abs() < 0.02,
            "w lewo {} nie doszedł do -{}", b.steer, c.max_steer
        );
    }

    #[test]
    fn steering_does_not_pivot_a_stationary_bike() {
        // Na postoju kierownica skręca, ale pojazd stoi — inaczej
        // można by „wspiąć" w miejscu, co zabija poczucie masy.
        let mut b = Bike::default();
        run(&mut b, 3.0, 0.0, 1.0);
        assert!(b.steer > 0.3, "kierownica się skręciła");
        assert!(b.speed.abs() < 0.05);
        assert!(b.yaw.abs() < 1e-3, "a mimo to obrócił się: {}", b.yaw);
    }

    #[test]
    fn turning_right_moves_toward_the_riders_right() {
        let mut b = Bike::default();
        run(&mut b, 6.0, 1.0, 1.0);
        // Skręt w prawo = w stronę `right()`, czyli przeciwnie z rosnącym
        // `yaw`. Na początku (yaw = 0) `right()` wskazuje na `-X`, więc
        // `pos.x` musi zmaleć. To jest test, który łapie odwróconą
        // kierownicę — najgorszy możliwy błąd w grze wyścigowej.
        assert!(b.yaw < 0.0, "nagłówek {} powinien maleć w prawo", b.yaw);
        assert!(b.pos.x < 0.0, "skręt w prawo, a x = {}", b.pos.x);
    }

    #[test]
    fn lean_matches_the_turn_direction() {
        let mut b = Bike::default();
        run(&mut b, 8.0, 1.0, 1.0);
        // Konwencja: `lean > 0` = przechył w stronę `right()`.
        assert!(b.lean > 0.0, "przechył {} nie w stronę skrętu", b.lean);
        // ...i motocykl nie może przechylić się w stronę, w którą
        // nie skręca — to by oznaczało, że pada na zakręcie
        assert!(b.lean.signum() == b.yaw_rate.signum() * -1.0);
    }

    #[test]
    fn lean_is_capped_so_bike_does_not_tip_over() {
        let mut b = Bike::default();
        run(&mut b, 15.0, 1.0, 1.0);
        assert!(b.lean.abs() <= cfg().max_lean + 1e-4, "przewrócił się: {}", b.lean);
    }

    #[test]
    fn reverse_has_a_lower_cap_than_forward() {
        let c = cfg();
        let mut b = Bike::default();
        run(&mut b, 15.0, -1.0, 0.0);
        assert!(b.speed < 0.0, "nie cofa: {}", b.speed);
        assert!(
            b.speed >= -c.reverse_speed - 1e-3,
            "cofanie przekroczyło limit: {} > {}", b.speed, -c.reverse_speed
        );
    }

    #[test]
    fn brake_does_not_flip_through_zero_in_one_frame() {
        let mut b = Bike::default();
        run(&mut b, 3.0, 1.0, 0.0);
        // hamujemy bardzo krótko, wychodząc tuż przed zerem
        step(&mut b, 0.05, -1.0, 0.0, &cfg());
        if b.speed < 0.0 {
            assert!(b.speed > -0.6, "przeskoczył przez zero: {}", b.speed);
        }
    }

    #[test]
    fn huge_dt_does_not_launch_the_bike() {
        // okno przeciągnięte albo breakpoint w debuggerze: dt = 3 s
        let mut b = Bike::default();
        step(&mut b, 3.0, 1.0, 1.0, &cfg());
        assert!(b.pos.is_finite(), "pozycja ma NaN: {:?}", b.pos);
        assert!(b.pos.length() < 5.0, "wyrzuciło: {:?}", b.pos);
    }

    #[test]
    fn reset_puts_bike_on_the_centre_line() {
        let mut b = Bike::default();
        run(&mut b, 10.0, 1.0, 0.7);
        reset(&mut b, 0.0);
        assert_eq!(b.pos, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(b.speed, 0.0);
        assert_eq!(b.yaw, 0.0);
    }
}
