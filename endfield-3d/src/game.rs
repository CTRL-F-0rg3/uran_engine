//! Rozgrywka: ruch, kamera, przeciwnicy, pociski.
//!
//! # Co tu jest naprawdę
//!
//! To nie jest szkielet na przyszłość. Każdy element jest potrzebny,
//! żeby zrobić się wiarygodny zrzut ekranu, na którym widać
//! materiał: gracz musi się poruszać (inaczej nie ma ruchu kamery),
//! przeciwnik musi się zbliżać (inaczej nie ma walki w kadrze), a
//! pociski muszą lecieć (inaczej nie ma czego śledzić).
//!
//! # Dlaczego to zrobione tak, a nie inaczej
//!
//! Cały świat gry to CPU i wczytanie z `main`. Nie ma ECS, nie ma
//! wielowątkowości. Przy ~20 bytach i jednej postaci to najprostsza
//! struktura, która działa — a dopiero potem warto ją zamieniać.

use uran_math::Vec3;

/// Jak długo trwa atak (sekundy). Po tym wraca do chodu.
pub const ATTACK_TIME: f32 = 0.45;

/// Jak długo trwa odrzut po trafieniu.
pub const HITSTUN_TIME: f32 = 0.30;

/// Czas życia pocisku w sekundach. Po tym zniknie, nawet jeśli nie
/// trafi w cel — inaczej pociski rosłyby w nieskończoność.
pub const BOLT_LIFE: f32 = 1.6;

/// Prędkość pocisku (m/s). Szybki, ale nie natychmiastowy: przy
/// 34 m/s gracz zdąży zobaczyć tor, a pocisk nie przelatuje przez
/// przeciwnika w jednej klatce.
pub const BOLT_SPEED: f32 = 34.0;

/// Ile pocisków gracz może wystrzelić bez przerwy.
pub const MAX_BOLTS: usize = 24;

/// Pojemność magazynka gracza.
pub const MAX_AMMO: usize = 6;

/// Ile przeciwników żyje naraz.
pub const MAX_DRONES: usize = 6;

/// Iskra w miejscu trafienia — krótka, jasna bryła.
///
/// Bez niej trafienie to tylko zniknięcie pocisku, a gracz nie ma
/// jak zobaczyć, że trafił. Krótki czas życia (~0.2 s) sprawia, że
/// przy ciągłym ogniu iskry nakładają się w jeden błysk, tak jak
/// w prawdziwej broni.
#[derive(Clone, Copy)]
pub struct Impact {
    pub pos: Vec3,
    pub life: f32,
}

impl Impact {
    fn new(pos: Vec3) -> Self {
        Self { pos, life: 0.22 }
    }

    /// 0..1 — jak jasna jest iskra teraz.
    pub fn fade(&self) -> f32 {
        (self.life / 0.22).clamp(0.0, 1.0)
    }
}

/// Stan pojedynczego pocisku.
#[derive(Clone, Copy)]
pub struct Bolt {
    /// Środek pocisku.
    pub pos: Vec3,
    /// Kierunek lotu, znormalizowany.
    pub dir: Vec3,
    /// Ile jeszcze ma żyć.
    pub life: f32,
    /// Czy pocisk właśnie trafił (do efektu).
    pub hit: bool,
}

impl Bolt {
    fn new(from: Vec3, dir: Vec3) -> Self {
        Self {
            pos: from,
            dir,
            life: BOLT_LIFE,
            hit: false,
        }
    }
}

/// Stan przeciwnika.
#[derive(Clone, Copy)]
pub struct Drone {
    /// Środek bryły.
    pub pos: Vec3,
    /// Obrót wokół osi Y — pancerz się kręci, więc sylwetka żyje.
    pub yaw: f32,
    /// 0..1 — maleje od trafienia, zerowy = zniszczony.
    pub health: f32,
    /// Animacja „dychania": dryfuje w pionie, nie lata sztywno.
    pub bob: f32,
    /// Czas do następnego strzału.
    pub fire_in: f32,
    /// Czas do końca białego rozbłysku po trafieniu.
    ///
    /// Bez tego trafienie daje tylko zniknięcie pocisku — gracz nie
    /// ma jak zobaczyć, że strzał się powiódł.
    pub flash: f32,
}

impl Drone {
    fn new(pos: Vec3, phase: f32) -> Self {
        Self {
            pos,
            yaw: 0.0,
            health: 1.0,
            // Faza startuje rozstrzelona po wszystkich przeciwnikach,
            // żeby nie unosili się identycznie — synchronizacja od razu
            // po wczytaniu wygląda jak błąd, nie jak wzór.
            bob: phase,
            fire_in: 1.0 + phase,
            flash: 0.0,
        }
    }
}

/// Stan postaci.
pub struct Player {
    /// Pozycja stóp.
    pub pos: Vec3,
    /// Kierunek patrzenia (radiany wokół Y).
    pub yaw: f32,
    /// Prędkość w chodzie (m/s).
    pub speed: f32,
    /// Czy biegnie (Shift).
    pub sprinting: bool,
    /// Czy jest w powietrzu.
    pub airborne: bool,
    /// Wysokość skoku.
    pub vy: f32,
    /// Faza chodu — napędza nogi.
    pub gait: f32,
    /// Czas do końca ataku.
    pub attack: f32,
    /// Czas do końca odrzutu.
    pub stun: f32,
    /// Ile pocisków zostało do wystrzałenia.
    pub ammo: usize,
    /// Czas od ostatniego trafienia — napędza odrzut kamery.
    pub shake: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            pos: Vec3::new(0.0, 0.0, 9.0),
            yaw: std::f32::consts::PI,
            speed: 0.0,
            sprinting: false,
            airborne: false,
            vy: 0.0,
            gait: 0.0,
            attack: 0.0,
            stun: 0.0,
            ammo: MAX_AMMO,
            shake: 0.0,
        }
    }
}

impl Player {
    /// Wysokość bioder — punkt, z którego wychodzą pociski i ręce.
    ///
    /// Nie środek postaci: to on daje pociskom „ramiona" i sprawia,
    /// że atak z gracza wygląda na rzucany z barku.
    pub fn chest(&self) -> Vec3 {
        self.pos + Vec3::new(0.0, 1.15, 0.0)
    }

    /// Dokąd patrzy gracz (jednostkowy wektor poziomy).
    pub fn facing(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos())
    }
}

/// Cały stan rozgrywki.
pub struct Game {
    pub player: Player,
    pub drones: Vec<Drone>,
    pub bolts: Vec<Bolt>,
    /// Iskry w miejscach trafień.
    pub impacts: Vec<Impact>,
    /// Czas gry w sekundach — napędza animacje i mruganie świateł.
    pub time: f32,
    /// Ile przeciwników zostało.
    pub kills: u32,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    pub fn new() -> Self {
        let mut game = Self {
            player: Player::default(),
            drones: Vec::new(),
            bolts: Vec::new(),
            impacts: Vec::new(),
            time: 0.0,
            kills: 0,
        };
        game.spawn_wave();
        game
    }

    /// Rozmieszcza przeciwników wokół platformy.
    ///
    /// Pierścień, nie chaos: czytelny układ od razu pokazuje, że scena
    /// jest zaprojektowana, a nie zrzucona. Środkowe miejsce zostaje
    /// puste, bo stoi tam reaktor.
    fn spawn_wave(&mut self) {
        // 6 przeciwników na pierścieniu, co 60°, pomijając reaktor.
        for i in 0..MAX_DRONES {
            let a = i as f32 / MAX_DRONES as f32 * std::f32::consts::TAU;
            let r = 13.0;
            let pos = Vec3::new(a.cos() * r, 1.25, a.sin() * r);
            // Faza rozstrzelona złotą proporcją — „irracjonalny" podział
            // wygląda bardziej naturalnie niż równy.
            self.drones.push(Drone::new(pos, i as f32 * 0.618));
        }
    }
}

/// Wejście z klawiatury: stan przycisków i myszy na jedną klatkę.
///
/// Oddzielne od `Game`, żeby można było testować logikę ruchu bez
/// okna i bez GPU — dokładnie po to powstał ten typ.
#[derive(Debug, Clone, Copy, Default)]
pub struct Input {
    /// Wektor ruchu w przestrzeni świata: X = w prawo, Z = do przodu.
    pub move_dir: [f32; 2],
    /// Kamerę obracamy myszą — względem ekranu, nie świata.
    /// Klawisze akcji, odczytane jako „w tej klatce wciśnięte".
    pub fire: bool,
    pub sprint: bool,
    pub jump: bool,
}

impl Input {
    /// Czy gracz w ogóle się porusza — brak ruchu nie powinien
    /// przyspieszać chodu.
    pub fn moving(&self) -> bool {
        self.move_dir[0].abs() + self.move_dir[1].abs() > 0.01
    }
}

impl Game {
    /// Przesuwa grę o `dt` sekund.
    ///
    /// Kolejność operacji wynika z zależności, nie z przyzwyczajenia:
    /// ruch → pozycja → pociski → przeciwnicy. Gdyby pociski leciały
    /// przed ruchem gracza, odrzut po trafieniu przesunąłby strzał
    /// o krok.
    pub fn update(&mut self, dt: f32, input: &Input, cam_yaw: f32) {
        let dt = dt.clamp(0.0, 0.05);
        self.time += dt;

        self.update_player(dt, input, cam_yaw);
        self.update_bolts(dt);
        self.update_drones(dt);
    }

    /// Ruch postaci wraz z chodem, skokiem i atakiem.
    fn update_player(&mut self, dt: f32, input: &Input, cam_yaw: f32) {
        let p = &mut self.player;

        // --- odrzut po trafieniu: najpierw, bo zabiera kontrolę ---
        if p.stun > 0.0 {
            p.stun -= dt;
            p.speed = 0.0;
        }
        if p.attack > 0.0 {
            p.attack -= dt;
        }
        if p.shake > 0.0 {
            p.shake -= dt;
        }

        // --- wektor ruchu w przestrzeni świata ---
        //
        // Obracamy wektor z klawiatury o kąt kamery. Dzięki temu
        // „W" zawsze znaczy „od kamery", a gracz nie musi pamiętać,
        // w którą stronę zwrócona jest postać.
        let (s, c) = cam_yaw.sin_cos();
        let (mx, mz) = (input.move_dir[0], input.move_dir[1]);
        let wish = Vec3::new(mx * c + mz * s, 0.0, -mx * s + mz * c);

        p.sprinting = input.sprint && input.moving() && p.stun <= 0.0;
        // 5.5 m/s to bieg, 2.6 to chód — różnica jest duża, bo to ona
        // decyduje, czy postać „ucieka" czy idzie.
        let target = if p.sprinting { 5.5 } else { 2.6 };
        // Przyspieszanie zamiast natychmiastowej prędkości: skok
        // z 0 na 5.5 m/s w jednej klatce wygląda jak teleport.
        p.speed += (target - p.speed) * (12.0 * dt).min(1.0);
        if !input.moving() {
            p.speed *= 0.82f32.powf(dt * 60.0);
        }

        if p.stun <= 0.0 {
            p.pos += wish * (p.speed * dt);
        }

        // --- obrót postaci ---
        //
        // Postać obraca się W STRONĘ ruchu, a nie kamery: to znany
        // problem „chodzenia bokiem" w kamerze trzeciej osoby.
        let facing_target = if input.fire {
            p.yaw
        } else if input.moving() {
            wish.yaw()
        } else {
            p.yaw
        };
        // Najkrótsza droga: różnica 179° i −179° to jedno samo,
        // a naiwna interpolacja wykonałaby obrót o 358° przez całą
        // scenę.
        let diff = wrap_angle(facing_target - p.yaw);
        p.yaw += diff * (14.0 * dt).min(1.0);

        // --- skok i grawitacja ---
        //
        // Oba w jednym miejscu, bo rozdzielenie prowadzi do
        // klasycznego błędu: postać wylatuje w górę i już nie wraca.
        if input.jump && !p.airborne && p.stun <= 0.0 {
            p.vy = 4.6;
            p.airborne = true;
        }
        if p.airborne {
            p.vy -= 19.6 * dt;
            p.pos.y += p.vy * dt;
            if p.pos.y <= 0.0 {
                p.pos.y = 0.0;
                p.vy = 0.0;
                p.airborne = false;
            }
        }

        // --- chód ---
        //
        // Faza chodu towarzyszy ODLEGŁOŚCI, nie czasowi. Dzięki temu
        // stopa nie ślizga się po podłodze: krótszy krok = wolniejszy
        // marsz, co jest dokładnie tym, co robią nogi.
        p.gait += p.speed * dt * 2.6;
        if p.gait > std::f32::consts::TAU {
            p.gait -= std::f32::consts::TAU;
        }

        // --- ostrzał ---
        if input.fire && p.attack <= 0.0 && p.ammo > 0 && p.stun <= 0.0 {
            p.attack = ATTACK_TIME;
            p.ammo -= 1;
            // Pocisk wychodzi z piersi i leci lekko w górę: przy
            // celowaniu w przeciwnika stojącego poziomo trafia w
            // tułów, a nie w stopy.
            let dir = (p.facing() + Vec3::new(0.0, 0.05, 0.0)).normalize_or_zero();
            self.bolts.push(Bolt::new(p.chest(), dir));
        }
        // Regeneracja amunicji: demo musi dać się grać w nieskończoność,
        // a bez tego kończyłoby się po sześciu strzałach.
        if p.ammo < MAX_AMMO {
            p.ammo = (p.ammo as f32 + dt * 1.6) as usize;
            if p.ammo > MAX_AMMO {
                p.ammo = MAX_AMMO;
            }
        }

        // --- granice platformy ---
        //
        // Gracz nie wychodzi poza podłogę. Bez tego spadałby w mgłę
        // i demo wyglądałoby jak awaria, a nie jak zamknięta arena.
        let lim = crate::world::ARENA_HALF - 1.5;
        p.pos.x = p.pos.x.clamp(-lim, lim);
        p.pos.z = p.pos.z.clamp(-lim, lim);
    }

    /// Aktualny stan animacji postaci.
    pub fn stance(&self) -> crate::character::Stance {
        let p = &self.player;
        if p.attack > 0.0 {
            return crate::character::Stance::Attack;
        }
        if p.airborne {
            return crate::character::Stance::Jump;
        }
        if p.speed > 3.6 {
            return crate::character::Stance::Run;
        }
        if p.speed > 0.25 {
            return crate::character::Stance::Walk;
        }
        crate::character::Stance::Idle
    }

    /// Ruch pocisków i sprawdzanie trafień.
    fn update_bolts(&mut self, dt: f32) {
        // Zbieramy trafienia w osobną listę, a nie modyfikujemy
        // `drones` w pętli po `bolts`: po pierwszym trafieniu `break`
        // kończy tylko tę gałąź, a `drones` i tak musi zostać
        // niezmienione do końca klatki.
        let mut landed: Vec<(usize, Vec3)> = Vec::new();
        for bolt in &mut self.bolts {
            bolt.life -= dt;
            bolt.pos += bolt.dir * (BOLT_SPEED * dt);
            for (i, drone) in self.drones.iter().enumerate() {
                // Test kulisty: prostokątna bryła przeciwnika dałaby
                // trafienia w narożnik, których gracz nie widzi.
                if (bolt.pos - drone.pos).length() < 0.85 {
                    bolt.hit = true;
                    bolt.life = -1.0;
                    landed.push((i, bolt.pos));
                    break;
                }
            }
        }
        for (i, at) in landed {
            self.drones[i].health -= 0.34;
            self.drones[i].flash = 0.18;
            self.player.shake = 0.12;
            self.impacts.push(Impact::new(at));
        }
        self.bolts.retain(|b| b.life > 0.0);
        // Iskry znikają same — bez tego scena zapychałaby się punktami
        // światła w miejscu każdego trafienia.
        for im in &mut self.impacts {
            im.life -= dt;
        }
        self.impacts.retain(|i| i.life > 0.0);
        // Usuwamy zniszczonych i naliczamy punkty.
        let before = self.drones.len();
        self.drones.retain(|d| d.health > 0.0);
        self.kills += (before - self.drones.len()) as u32;
        // Gdy arena jest pusta, dorzucamy kolejną falę — inaczej demo
        // cichłoby po dwudziestu sekundach.
        if self.drones.is_empty() {
            self.spawn_wave();
        }
    }

    /// Ruch przeciwników: podejście do gracza i ostrzał.
    fn update_drones(&mut self, dt: f32) {
        let target = self.player.pos;
        let time = self.time;
        // Kolizja pocisku z graczem liczona tu, po ruchu przeciwników,
        // żeby trafienie zależało od ich aktualnego położenia, a nie
        // pozycji z początku klatki.
        let mut hits: Vec<Vec3> = Vec::new();

        for drone in &mut self.drones {
            drone.bob += dt * 1.4;
            if drone.flash > 0.0 {
                drone.flash -= dt;
            }
            let to_player = target - drone.pos;
            let dist = to_player.length();
            // `dist` może być zerowy, gdy przeciwnik stoi dokładnie na
            // graczu — wtedy `normalize_or_zero` daje zero i nic się
            // nie ruszy, zamiast wyrzucić NaN do pozycji.
            let dir = to_player.normalize_or_zero();
            // Trzymają dystans 9 m: bliżej wyglądają jak wrogowie
            // napierający, dalej — jak patrolujące maszyny.
            let want = 9.0;
            if dist > want + 1.0 {
                drone.pos += dir * (2.4 * dt);
            } else if dist < want - 1.0 {
                drone.pos -= dir * (1.6 * dt);
            }
            // Nie wchodzą w reaktor.
            let flat = Vec3::new(drone.pos.x, 0.0, drone.pos.z).length();
            if flat < 3.2 && flat > 1e-4 {
                let push = Vec3::new(drone.pos.x, 0.0, drone.pos.z).normalize_or_zero() * 3.2;
                drone.pos.x = push.x;
                drone.pos.z = push.z;
            }
            // Obrót pancerza: zawsze w stronę gracza, żeby było widać,
            // gdzie jest przód.
            drone.yaw = dir.yaw();
            // Delikatne kołysanie w pionie.
            drone.pos.y = 1.25 + (time * 1.4 + drone.bob).sin() * 0.12;

            // Ostrzał: rzadki, bo częsty strzał zalewa ekran pociskami.
            drone.fire_in -= dt;
            if drone.fire_in <= 0.0 {
                drone.fire_in = 2.6 + drone.bob.sin().abs() * 1.4;
                // Celujemy w pierś, nie w stopy — wygląda groźniej.
                let shot = (self.player.chest() - drone.pos).normalize_or_zero();
                if self.bolts.len() < MAX_BOLTS {
                    self.bolts.push(Bolt::new(drone.pos, shot));
                }
            }
        }

        // Pociski wroga mogą trafić gracza. Sprawdzamy to po ruchu
        // przeciwników, ale przed usunięciem pocisków.
        for bolt in &mut self.bolts {
            if bolt.life > 0.0 && (bolt.pos - self.player.chest()).length() < 0.6 {
                bolt.life = -1.0;
                hits.push(bolt.pos);
            }
        }
        for _ in hits {
            self.player.stun = HITSTUN_TIME;
            self.player.shake = 0.25;
        }
        self.bolts.retain(|b| b.life > 0.0);
    }
}

/// Kąt obrotu wokół osi Y dla wektora (kierunek `facing`).
///
/// Używamy `atan2(x, z)`, a nie `atan2(z, x)`: w tej konwencji kąt 0
/// oznacza „w stronę +Z", więc wektor zbudowany z `yaw` wraca do
/// `yaw` po odwróceniu. Odwrócona kolejność argumentów daje obrót
/// o 90° i cała scena staje się bokiem.
trait Yaw {
    fn yaw(&self) -> f32;
}

impl Yaw for Vec3 {
    fn yaw(&self) -> f32 {
        self.x.atan2(self.z)
    }
}

/// Skraca różnicę kątów do przedziału `(-π, π]`.
///
/// Bez tego obrót o 1° przy `yaw` przekraczającym `π` wykonałby
/// pełny obrót dookoła — i postać wyglądałaby, jak wiruje w miejscu.
pub fn wrap_angle(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut x = a;
    while x > PI {
        x -= TAU;
    }
    while x < -PI {
        x += TAU;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gracz nie może wyjść poza platformę.
    ///
    /// To najtańszy test w pliku, a chroni najdroższy błąd: postać
    /// spadająca w mgłę wygląda jak awaria silnika, a jest błędem
    /// jednego nierówności.
    #[test]
    fn player_stays_on_the_platform() {
        let mut g = Game::new();
        let push = Input {
            move_dir: [1.0, 1.0],
            ..Default::default()
        };
        // 20 sekund w jedną stronę — więcej niż wystarczająco, żeby
        // przekroczyć krawędź kilka razy.
        for _ in 0..1200 {
            g.update(1.0 / 60.0, &push, 0.0);
        }
        let lim = crate::world::ARENA_HALF - 1.5;
        assert!(
            g.player.pos.x.abs() <= lim + 0.01 && g.player.pos.z.abs() <= lim + 0.01,
            "gracz wyszedł poza platformę: {:?} (limit {lim})",
            g.player.pos
        );
    }

    /// `dt` z ułamka sekundy nie może „przeskoczyć" przez całą
    /// symulację.
    ///
    /// Test celowo podaje 5 sekund w jednym kroku (zaciśnięta pętla
    /// albo debugger). Klamrowanie `dt` w `update` powinno ograniczyć
    /// to do jednej klatki, inaczej gracz teleportowałby się przez
    /// całą scenę.
    #[test]
    fn huge_timestep_does_not_teleport_the_player() {
        let mut g = Game::new();
        let start = g.player.pos;
        g.update(
            5.0,
            &Input {
                move_dir: [1.0, 0.0],
                ..Default::default()
            },
            0.0,
        );
        let moved = (g.player.pos - start).length();
        assert!(
            moved < 1.0,
            "jedna klatka przesunęła gracza o {moved} m — dt nie jest klamrowane"
        );
    }

    /// Pociski znikają, a nie mnożą się w nieskończoność.
    ///
    /// Bez `retain` po `life` pociski nigdy nie znikają, a scena
    /// zapycha się bryłami. Test sprawdza najprostszy przypadek: brak
    /// celów, sam upływ czasu.
    #[test]
    fn bolts_expire_on_their_own() {
        let mut g = Game::new();
        // Wystrzelić w stronę bez przeciwników i poczekać.
        g.player.pos = Vec3::new(0.0, 0.0, -20.0);
        g.player.yaw = 0.0;
        // Wszystkich dronów odsuwamy daleko: ich pociski też wędrują
        // w `bolts`, więc bez tego „pusty" stan nigdy nie nastąpi.
        for d in &mut g.drones {
            d.pos = Vec3::new(500.0, 0.0, 500.0);
            d.fire_in = 1e9;
        }
        for _ in 0..5 {
            g.update(
                0.05,
                &Input {
                    fire: true,
                    ..Default::default()
                },
                0.0,
            );
        }
        assert!(
            !g.bolts.is_empty(),
            "gracz nie wystrzelił, test jest bez sensu"
        );
        for _ in 0..(BOLT_LIFE / 0.05) as u32 + 4 {
            g.update(0.05, &Input::default(), 0.0);
        }
        assert!(
            g.bolts.is_empty(),
            "po {:.0} s zostało {} pocisków, a powinno zniknąć",
            BOLT_LIFE,
            g.bolts.len()
        );
    }

    /// Skok kończy się lądowaniem, nie wiecznym unoszeniem.
    ///
    /// Rozdzielenie grawitacji i skoku to klasyczny błąd: postać
    /// wylatuje i już nie wraca, a demo wygląda wtedy, jakby było
    /// celowo „nierealne".
    #[test]
    fn jump_always_comes_back_down() {
        let mut g = Game::new();
        g.update(
            1.0 / 60.0,
            &Input {
                jump: true,
                ..Default::default()
            },
            0.0,
        );
        assert!(g.player.airborne, "skok nie wystartował");
        assert!(g.player.pos.y > 0.0, "postać nie uniosła się");
        for _ in 0..300 {
            g.update(1.0 / 60.0, &Input::default(), 0.0);
        }
        assert!(!g.player.airborne, "postać wciąż jest w powietrzu");
        assert!(
            g.player.pos.y.abs() < 0.01,
            "postać wylądowała na wysokości {}",
            g.player.pos.y
        );
    }

    /// Arena nigdy nie zostaje pusta na dłużej niż jedna klatka.
    ///
    /// Bez respawnu demo cichłoby po dwudziestu sekundach i zrzut
    /// ekranu byłby pustą platformą.
    #[test]
    fn killing_every_drone_respawns_a_wave() {
        let mut g = Game::new();
        g.drones.clear();
        g.update(1.0 / 60.0, &Input::default(), 0.0);
        assert_eq!(
            g.drones.len(),
            MAX_DRONES,
            "pusta arena nie dostała nowej fali"
        );
    }

    /// `wrap_angle` skraca różnicę do najkrótszej drogi.
    ///
    /// Bez tego obrót o 1° przy `yaw` tuż nad `π` wykonałby pełen obrót
    /// dookoła — postać wyglądałaby, jak wiruje w miejscu.
    #[test]
    fn angle_wrapping_picks_the_short_way_round() {
        use std::f32::consts::{PI, TAU};
        // Wynik ZAWSZE mieści się w `(-π, π]` — to jedyna gwarancja,
        // której potrzebujemy przy obliczaniu `yaw + różnica`.
        for a in [
            0.0,
            0.5,
            PI - 0.001,
            PI,
            PI + 0.1,
            2.0 * PI - 0.1,
            2.0 * PI,
            7.0,
            -7.0,
        ] {
            let w = wrap_angle(a);
            assert!(
                w > -PI - 1e-4 && w <= PI + 1e-4,
                "wrap_angle({a}) = {w}, poza zakresem (−π, π]"
            );
        }
        // 360° + 0.5 to to samo co 0.5, a 358° to −2°.
        assert!((wrap_angle(TAU + 0.5) - 0.5).abs() < 1e-5);
        assert!((wrap_angle(2.0 * PI - 0.1) + 0.1).abs() < 1e-5);
        // 181° musi dać −179°, a nie +181° (to jest właśnie „najkrótsza
        // droga": 2° zamiast obrotu o 358°).
        assert!((wrap_angle(PI + 0.1) - (-PI + 0.1)).abs() < 1e-5);
    }
}
