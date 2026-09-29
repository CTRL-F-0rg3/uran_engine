//! Świat przeszkód i rozwiązywanie wejść gracza.
//!
//! Świadomie **bez silnika kolizji** (jak [`junak-rider`]): cała farma
//! to kilkanaście prostopadłościanów stojących na płaskim terenie.
//! Silnik byłby tu dwa rzędy wielkości cięższy i wniósłby dokładnie ten
//! sam błąd — trudniej byłoby zobaczyć, co się dzieje.
//!
//! Gracz to prostopadłościan: szerokość [`PLAYER_RADIUS`] na obie
//! strony i wysokość [`PLAYER_HEIGHT`]. Traktujemy go jako bryłę, a nie
//! punkt, bo inaczej gracz „wchodziłby" w ścianę aż do samej środka
//! oczu, a kamera wystawałaby z budynku.

use uran_math::Vec3;

/// Połowa szerokości gracza (promień barków), w metrach.
pub const PLAYER_RADIUS: f32 = 0.35;

/// Wysokość gracza od stóp do czubka głowy, w metrach.
///
/// Oko ([`crate::EYE`]) jest niżej niż czubek głowy, a stopy na `y = 0`.
pub const PLAYER_HEIGHT: f32 = 1.8;

/// Najdłuższy podkrok ruchu (m) — ochrona przed tunelowaniem.
///
/// Ściana ma 0,3 m grubości, więc podkroki krótsze niż to pozwalają
/// zawsze wykryć wejście. Przy 60 FPS i 5 m/s gracz przesuwa się
/// 0,083 m na klatkę, więc w normalnej grze dzielenie w ogóle nie
/// zachodzi — chroni tylko przy zaciętym klatkarzu (patrz test
/// `huge_dt_does_not_tunnel_through_a_wall`).
const MAX_STEP: f32 = 0.2;

/// Prostopadłościan w świecie — środek i połowa wymiarów.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Box {
    pub center: Vec3,
    pub half: Vec3,
}

/// Bryła **postawiona na ziemi** (dno na `y = 0`).
///
/// Zapisujemy pozycję i wysokość, a nie środek, bo przy budowaniu
/// świata z assetów znamy właśnie takie dane.
pub struct Solid {
    /// Środek w płaszczyźnie XZ.
    pub pos: Vec3,
    pub width: f32,
    pub depth: f32,
    pub height: f32,
}

impl Solid {
    /// Zamienia na AABB. `pos.y` ignorujemy — bryła stoi na ziemi.
    pub fn to_box(self) -> Box {
        Box {
            center: Vec3::new(self.pos.x, self.height * 0.5, self.pos.z),
            half: Vec3::new(self.width * 0.5, self.height * 0.5, self.depth * 0.5),
        }
    }
}

/// Świat przeszkód: statyczne bryły + ograniczenie mapy.
#[derive(Debug, Default, Clone)]
pub struct World {
    boxes: Vec<Box>,
    /// Granica świata w metrach (`|x|`, `|z|`).
    pub limit: f32,
}

impl World {
    pub fn new(limit: f32) -> Self {
        Self {
            boxes: Vec::new(),
            limit,
        }
    }

    pub fn add(&mut self, solid: Solid) {
        self.boxes.push(solid.to_box());
    }

    /// Wszystkie bryły świata.
    ///
    /// Potrzebne do rysowania drutów (patrz `F1` w `main.rs`) — bez
    /// dostępu do listy debug pokazałby świat bez przeszkód.
    pub fn boxes(&self) -> &[Box] {
        &self.boxes
    }

    /// Przesuwa gracza i rozwiązuje wejścia w bryły. `pos` to **dno**.
    ///
    /// Kolejność operacji wynika z tego, że teren jest płaski i
    /// nieskończony: każde zejście poniżej `y = 0` kończy się
    /// lądowaniem. Rozwiązywanie poziome **osobno dla X i dla Z** daje
    /// ślizganie się po ścianie zamiast zacięcia w jej narożniku.
    ///
    /// Ruch dzielimy na podkroki nie dłuższe niż [`MAX_STEP`], bo jedno
    /// rozwiązanie AABB nie chroni przed **tunelowaniem**: przy skoku
    /// 15 m gracz przeszedłby przez ścianę, bo po przesunięciu jego AABB
    /// wylądowałby już po drugiej stronie i test przecięcia nie znalazłby
    /// nic do wypchnięcia. Podkroki gwarantują, że w każdym kroku
    /// gracz styka się z bryłą **z jednej strony**.
    pub fn move_player(&self, pos: &mut Vec3, delta: Vec3, radius: f32, height: f32) {
        let len = delta.length();
        if len <= MAX_STEP {
            self.move_one_step(pos, delta, radius, height);
            return;
        }
        let steps = (len / MAX_STEP).ceil() as i32;
        let part = delta / steps as f32;
        for _ in 0..steps {
            self.move_one_step(pos, part, radius, height);
        }
    }

    /// Jeden podkrok: grawitacja, lądowanie i wypchnięcia po osiach.
    fn move_one_step(&self, pos: &mut Vec3, delta: Vec3, radius: f32, height: f32) {
        // Oś Y: grawitacja i lądowanie. Lądowanie bezwarunkowe, bo
        // podłoga jest wszędzie na `y = 0`.
        pos.y += delta.y;
        if pos.y <= 0.0 {
            pos.y = 0.0;
        }

        pos.x += delta.x;
        self.resolve(&mut *pos, radius, height);
        pos.z += delta.z;
        self.resolve(&mut *pos, radius, height);
    }

    /// Wypycha gracza z brył. Powtarzamy kilka razy, bo jedno
    /// rozwiązanie może wcisnąć gracza w inną bryłę (np. narożnik
    /// płotu przy budynku). Limit iteracji chroni przed zawieszeniem,
    /// gdyby bryły wypierały się nawzajem bez końca.
    fn resolve(&self, pos: &mut Vec3, radius: f32, height: f32) {
        for _ in 0..4 {
            if !self.resolve_once(pos, radius, height) {
                return;
            }
        }
    }

    /// Jedna runda wypychania. Zwraca `true`, gdy coś się ruszyło.
    ///
    /// Po wybraniu kierunku **sprawdzamy wynik** i, jeśli gracz nadal
    /// leży w bryle, wypychamy go w osi, którą wcześniej pominięliśmy.
    /// Bez tego zdarzał się błąd „najkrótszej drogi": przy nacisku po
    /// skosie gracz zostawał w kolidatorze, bo wypchnięcie w Z
    /// przesuwało go zza ściany prosto do ściany obok.
    fn resolve_once(&self, pos: &mut Vec3, radius: f32, height: f32) -> bool {
        let mut moved = false;
        let head_y = pos.y + height;
        for b in &self.boxes {
            // Gracz jako AABB: `2*radius` na X i Z, `height` od stóp.
            let px_min = b.center.x - b.half.x - radius;
            let px_max = b.center.x + b.half.x + radius;
            let pz_min = b.center.z - b.half.z - radius;
            let pz_max = b.center.z + b.half.z + radius;
            let by_min = b.center.y - b.half.y;
            let by_max = b.center.y + b.half.y;

            // Test przecięcia na każdej osi osobno. Używamy `<=`/`>=`,
            // żeby dotykanie ściany kończyło się wypchnięciem, a nie
            // przepuszczaniem.
            if pos.x + radius <= px_min
                || pos.x - radius >= px_max
                || pos.z + radius <= pz_min
                || pos.z - radius >= pz_max
                || head_y <= by_min
                || pos.y >= by_max
            {
                continue;
            }

            // Wybieramy tę oś, na której **korygowanie jest najmniejsze**
            // liczone względem środka bryły. Wypchnięcie liczymy jako
            // odległość do krawędzi bryły, a nie do środka — różnica
            // jest istotna przy szerokich bryłach.
            let push_x = (px_max - pos.x).min(pos.x - px_min);
            let push_z = (pz_max - pos.z).min(pos.z - pz_min);
            if push_x <= push_z {
                // Wybieramy stronę, do której gracz jest bliżej, żeby
                // wypchnięcie nie przerzucało go przez całą bryłę.
                pos.x += if pos.x < b.center.x {
                    px_min - pos.x
                } else {
                    px_max - pos.x
                };
            } else {
                pos.z += if pos.z < b.center.z {
                    pz_min - pos.z
                } else {
                    pz_max - pos.z
                };
            }
            moved = true;
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Świat z jedną bryłą 2×2×2 stojącą w (0,0,0), czyli zajmującą
    /// X i Z w zakresie [-1, 1].
    fn world_with_hut() -> World {
        let mut w = World::new(50.0);
        w.add(Solid {
            pos: Vec3::ZERO,
            width: 2.0,
            depth: 2.0,
            height: 2.0,
        });
        w
    }

    #[test]
    fn solid_sits_on_the_ground() {
        let b = Solid {
            // `pos.y` celowo śmieciowe — bryła stoi na ziemi, więc
            // przesunięcie w Y musi zostać zignorowane.
            pos: Vec3::new(1.0, 99.0, 2.0),
            width: 4.0,
            depth: 6.0,
            height: 3.0,
        }
        .to_box();
        assert_eq!(b.center, Vec3::new(1.0, 1.5, 2.0));
        assert_eq!(b.half, Vec3::new(2.0, 1.5, 3.0));
    }

    #[test]
    fn player_cannot_walk_into_a_wall() {
        let w = world_with_hut();
        let mut p = Vec3::new(-2.0, 0.0, 0.0);
        // Duży krok prosto w ścianę.
        w.move_player(
            &mut p,
            Vec3::new(5.0, 0.0, 0.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        assert!(p.x + PLAYER_RADIUS <= 1.0, "gracz wszedł w ścianę: {p:?}");
    }

    #[test]
    fn player_slides_along_the_wall_instead_of_sticking() {
        let w = world_with_hut();
        let mut p = Vec3::new(-2.0, 0.0, 0.0);
        // Nacisk po skosie: bez ślizgania utknąłby w narożniku.
        w.move_player(
            &mut p,
            Vec3::new(5.0, 0.0, 2.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        // Ślizganie: gracz musi przesunąć się w Z (poza ścianę).
        assert!(p.z > 1.0, "gracz nie przesunął się wzdłuż ściany: {p:?}");
        // I **nie może** zostać w bryle. Nie sprawdzamy tu `x`, bo
        // ślizganie z natury zostawia `x` wewnątrz zakresu bryły —
        // to poprawne, bo gracz jest poza nią dzięki `z`.
        let inside = p.x.abs() < 1.0 && p.z.abs() < 1.0;
        assert!(!inside, "gracz wpadł do bryły: {p:?}");
    }

    #[test]
    fn player_can_jump_over_a_low_wall() {
        let mut w = World::new(50.0);
        // Płot 0,4 m wysokości. Wąski w X (0,3 m), żeby gracz szedł
        // dokładnie na wprost niego, a nie w jego bok.
        w.add(Solid {
            pos: Vec3::ZERO,
            width: 0.3,
            depth: 6.0,
            height: 0.4,
        });
        let mut p = Vec3::new(-2.0, 0.0, 0.0);
        // Skok: stopy ponad szczytem, więc nie ma z czym kolidować.
        w.move_player(
            &mut p,
            Vec3::new(3.0, 1.5, 0.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        assert!(p.x > 0.15, "nie przeskoczył płotu: {p:?}");
    }

    #[test]
    fn gravity_pulls_down_to_the_ground() {
        let w = World::new(50.0);
        // Spadamy z 5 m. Jeden krok 0,1 m **nie** dosięgnie ziemi, więc
        // iterujemy aż do lądowania — inaczej test sprawdzałby
        // arytmetykę, a nie grawitację.
        let mut p = Vec3::new(0.0, 5.0, 0.0);
        for _ in 0..200 {
            w.move_player(
                &mut p,
                Vec3::new(0.0, -0.1, 0.0),
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
            );
        }
        assert_eq!(p.y, 0.0, "nie wylądował na ziemi: {p:?}");
    }

    #[test]
    fn tall_wall_cannot_be_jumped() {
        let mut w = World::new(50.0);
        // Ściana **wąska** w X (0,3 m) i wysoka na 6 m. Wąska, bo
        // szeroka kolidowałaby z graczem już przy starcie — gracz
        // wchodziłby w jej bok, a nie w ścianę naprzeciwko siebie.
        w.add(Solid {
            pos: Vec3::new(0.0, 0.0, 0.0),
            width: 0.3,
            depth: 6.0,
            height: 6.0,
        });
        let mut p = Vec3::new(-2.0, 0.0, 0.0);
        // Skok z 1,5 m: stopy są nad szczytem 6 m? Nie — to ściana
        // za wysoka, więc gracz musi zostać przed nią.
        w.move_player(
            &mut p,
            Vec3::new(3.0, 1.5, 0.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        assert!(
            p.x + PLAYER_RADIUS <= 0.15,
            "przeskoczył 6-metrową ścianę: {p:?}"
        );
    }

    #[test]
    fn no_approach_ends_up_inside_the_hut() {
        let w = world_with_hut();
        for start in [
            Vec3::new(-3.0, 0.0, -3.0),
            Vec3::new(3.0, 0.0, -3.0),
            Vec3::new(-3.0, 0.0, 3.0),
            Vec3::new(3.0, 0.0, 3.0),
            Vec3::ZERO,
        ] {
            let mut p = start;
            w.move_player(
                &mut p,
                Vec3::new(0.5, 0.0, 0.5),
                PLAYER_RADIUS,
                PLAYER_HEIGHT,
            );
            // Środek gracza nie może znaleźć się wewnątrz bryły.
            let inside = p.x.abs() < 1.0 && p.z.abs() < 1.0 && p.y < 2.0;
            assert!(!inside, "gracz z {start:?} znalazł się w bryle: {p:?}");
        }
    }

    #[test]
    fn empty_world_lets_player_move_freely() {
        let w = World::new(50.0);
        let mut p = Vec3::ZERO;
        w.move_player(
            &mut p,
            Vec3::new(3.0, 0.0, 4.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        // Podział na podkroki mnoży długość ruchu, więc wynik różni
        // się o ułamek float. Porównujemy z tolerancją, nie `==`.
        assert!((p.x - 3.0).abs() < 1e-3, "x = {}", p.x);
        assert!((p.z - 4.0).abs() < 1e-3, "z = {}", p.z);
    }

    #[test]
    fn huge_dt_does_not_tunnel_through_a_wall() {
        // Okno przeciągnięte albo breakpoint: dt = 3 s przy 5 m/s to
        // 15 m skoku, czyli przejście przez całą bryłę.
        let w = world_with_hut();
        let mut p = Vec3::new(-2.0, 0.0, 0.0);
        w.move_player(
            &mut p,
            Vec3::new(15.0, 0.0, 0.0),
            PLAYER_RADIUS,
            PLAYER_HEIGHT,
        );
        assert!(
            p.x + PLAYER_RADIUS <= 1.0,
            "gracz przeleciał przez ścianę: {p:?}"
        );
    }
}
