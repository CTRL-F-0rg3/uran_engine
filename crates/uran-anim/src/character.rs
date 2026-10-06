//! Postać: pozycja + ruch + automatyczny dobór animacji.
//!
//! Ten typ jest tym, czego gra potrzebuje w codziennej pętli: system podaje
//! **wektor wejścia** (z klawiatury albo AI), a `Character` sam przesuwa
//! pozycję, pamięta kierunek i wystawia gotową klatkę do narysowania.
//!
//! ```ignore
//! let input = Vec2::new(ctx.input.axis(Key::KeyA, Key::KeyD),
//!                       ctx.input.axis(Key::KeyS, Key::KeyW));
//! player.update(ctx.dt(), input, ctx.input.shift(), bounds, config);
//! player.draw(&mut ctx.gfx, &sheets, config);
//! ```

use uran_ecs::UvRect;
use uran_math::{Rect, Vec2};
use uran_render::Graphics;

use crate::clip::{AnimationClip, Animator};
use crate::facing::Facing;
use crate::sheet::SpriteSheet;

/// Zestaw arkuszy jednej animowanej postaci (po jednym na stan ruchu).
#[derive(Debug, Clone)]
pub struct CharacterSheets {
    /// Stojąc / oddychając.
    pub idle: SpriteSheet,
    /// Chód.
    pub walk: SpriteSheet,
    /// Sprint.
    pub run: SpriteSheet,
}

impl CharacterSheets {
    /// Arkusz dla stanu ruchu.
    pub fn get(&self, state: MotionState) -> &SpriteSheet {
        match state {
            MotionState::Idle => &self.idle,
            MotionState::Walk => &self.walk,
            MotionState::Run => &self.run,
        }
    }
}

/// Stan ruchu postaci — decyduje, który klip gramy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionState {
    /// Postać stoi.
    #[default]
    Idle,
    /// Postać idzie.
    Walk,
    /// Postać biegnie.
    Run,
}

/// Konfiguracja prędkości i animacji — jeden obiekt zamiast rozsianych
/// stałych, żeby dało się ją podmienić w testach.
#[derive(Debug, Clone, Copy)]
pub struct CharacterConfig {
    /// Prędkość chodu (jednostki świata na sekundę).
    pub walk_speed: f32,
    /// Prędkość sprintu (jednostki świata na sekundę).
    pub run_speed: f32,
    /// Tempo animacji chodu (klatki/s).
    pub walk_fps: f32,
    /// Tempo animacji biegu (klatki/s).
    pub run_fps: f32,
    /// Tempo animacji w miejscu (klatki/s).
    pub idle_fps: f32,
    /// Skala sprite'a względem rozmiaru klatki w arkuszu.
    pub sprite_scale: f32,
    /// Przesunięcie sprite'a w pionie względem „stóp" (ujemne = w dół).
    pub sprite_y_offset: f32,
    /// Rysować cień pod postacią.
    pub shadow: bool,
    /// Półwymiar stopy — do trzymania postaci w granicach świata.
    pub footprint: Vec2,
}

impl Default for CharacterConfig {
    fn default() -> Self {
        Self {
            walk_speed: 220.0,
            run_speed: 420.0,
            // Tempo idzie w parze z prędkością: chód 10 klatek na ~1 s,
            // bieg 8 klatek ale szybciej — inaczej stopa „ślizga się"
            // po podłodze albo animacja zwija się w kółko.
            walk_fps: 10.0,
            run_fps: 14.0,
            idle_fps: 6.0,
            // Klatka to 64x128 px i zawiera dużo pustego tła wokół
            // postaci, dlatego skala < 1.
            sprite_scale: 0.9,
            sprite_y_offset: -24.0,
            shadow: true,
            footprint: Vec2::new(20.0, 10.0),
        }
    }
}

/// Postać sterowana wektorem wejścia.
#[derive(Debug, Clone)]
pub struct Character {
    /// Pozycja „stóp" (środek podłogi pod postacią).
    pub position: Vec2,
    /// Prędkość z ostatniej klatki — przydatna do animacji i debugu.
    pub velocity: Vec2,
    /// Kierunek, w którym postać patrzy (nie znika gdy stoi).
    pub facing: Facing,
    /// Aktualny stan ruchu.
    pub state: MotionState,
    /// Odtwarzana animacja.
    pub anim: Animator,
    /// Klatka z ostatniego rysowania — do sprawdzenia, czy UV się zmienia.
    pub drawn_frame: u16,
}

impl Character {
    /// Nowa postać w punkcie `position`, patrząca na gracza.
    pub fn new(position: Vec2, config: CharacterConfig) -> Self {
        let facing = Facing::default();
        Self {
            position,
            velocity: Vec2::ZERO,
            facing,
            state: MotionState::Idle,
            anim: Animator::new(clip_for(MotionState::Idle, facing, config)),
            drawn_frame: u16::MAX,
        }
    }

    /// Przesuwa postać i dobiera animację.
    ///
    /// `input` to surowy wektor ze sterowania (nie musi być znormalizowany —
    /// normalizujemy go tutaj). `bounds` to prostokąt, w którym postać ma
    /// pozostać; pozycja jest do niego przycięta, więc gracz nie wyjdzie
    /// poza mapę.
    pub fn update(
        &mut self,
        dt: f32,
        input: Vec2,
        running: bool,
        bounds: Rect,
        config: CharacterConfig,
    ) {
        let motion = normalize(input);
        let moving = motion.length_squared() > 0.0;

        // Stan zależy od **bieżącego** wejścia, nie od historii — dzięki
        // temu puszczenie klawisza zatrzymuje postać natychmiast.
        self.state = match (moving, running) {
            (false, _) => MotionState::Idle,
            (true, true) => MotionState::Run,
            (true, false) => MotionState::Walk,
        };

        let speed = match self.state {
            MotionState::Idle => 0.0,
            MotionState::Walk => config.walk_speed,
            MotionState::Run => config.run_speed,
        };
        self.velocity = motion * speed;
        self.position += self.velocity * dt.max(0.0);

        self.clamp_to(bounds, config);

        // Kierunek zmieniamy tylko przy realnym ruchu — stojąca postać ma
        // patrzeć tam, gdzie zatrzymała się, a nie w losową stronę.
        if let Some(next) = Facing::from_motion_auto(self.velocity) {
            self.facing = next;
        }

        // Animacja zależy od stanu I kierunku: inny wiersz arkusza.
        let clip = clip_for(self.state, self.facing, config);
        self.anim.play_or_keep(clip);
        self.anim.tick(dt);
    }

    /// Przycina pozycję do granic świata. Robimy to po obu osiach
    /// niezależnie, żeby ślizganie się po ścianie działało poprawnie.
    fn clamp_to(&mut self, bounds: Rect, config: CharacterConfig) {
        let f = config.footprint;
        if self.position.x - f.x < bounds.left() {
            self.position.x = bounds.left() + f.x;
        }
        if self.position.x + f.x > bounds.right() {
            self.position.x = bounds.right() - f.x;
        }
        if self.position.y - f.y < bounds.bottom() {
            self.position.y = bounds.bottom() + f.y;
        }
        if self.position.y + f.y > bounds.top() {
            self.position.y = bounds.top() - f.y;
        }
    }

    /// Pozycja „stóp" — to, co śledzi kamera.
    pub fn feet(&self) -> Vec2 {
        self.position
    }

    /// Środek sprite'a (uwzględnia przesunięcie w pionie).
    pub fn sprite_center(&self, config: CharacterConfig) -> Vec2 {
        self.position + Vec2::new(0.0, config.sprite_y_offset)
    }

    /// UV bieżącej klatki dla arkusza stanu.
    pub fn current_uv(&self, sheets: &CharacterSheets) -> Option<UvRect> {
        self.anim.current_uv(sheets.get(self.state))
    }

    /// Rysuje postać: cień pod stopami, a potem sprite.
    pub fn draw(
        &mut self,
        gfx: &mut Graphics<'_>,
        sheets: &CharacterSheets,
        config: CharacterConfig,
    ) {
        let sheet = sheets.get(self.state);

        if config.shadow {
            let w = sheet.frame_world_size().x * 0.55 * config.sprite_scale;
            let shadow = Rect::from_center(self.position, Vec2::new(w, w * 0.35));
            gfx.color(uran_math::Color::rgba(0.0, 0.0, 0.0, 0.30))
                .draw_rounded_rect(shadow, w * 0.16);
        }

        let center = self.sprite_center(config);
        let rect = self.anim.draw_rect(sheet, center, config.sprite_scale, 0.0);

        // Pędzel trzyma kolor z poprzedniego rysowania (tutaj: czarny,
        // półprzezroczysty cień), a `draw_texture` mnoży teksturę przez ten
        // kolor. Bez resetu sprite wyszedłby półprzezroczysty i przebarwiony —
        // dlatego przed rysowaniem postaci przywracamy biel.
        gfx.color(uran_math::Color::WHITE);
        sheet.draw(gfx, rect, self.anim.frame(), self.anim.clip().row);
        self.drawn_frame = self.anim.frame();
    }
}

/// Klip dla stanu ruchu i kierunku, z tempem z konfiguracji.
///
/// Liczby klatek odpowiadają realnym assetom: `idle` i `run` mają 8 klatek
/// w arkuszu, `walk` 10.
fn clip_for(state: MotionState, facing: Facing, config: CharacterConfig) -> AnimationClip {
    let (frames, fps) = match state {
        MotionState::Idle => (8, config.idle_fps),
        MotionState::Walk => (10, config.walk_fps),
        MotionState::Run => (8, config.run_fps),
    };
    AnimationClip::new(facing.row(), frames, fps)
}

/// Normalizuje wektor wejścia, tłumiąc zbyt krótkie (śmieci z klawiatury).
///
/// W pełni zerowy wektor przechodzi bez zmian — inaczej gra nie mogłaby
/// zatrzymać postaci w miejscu.
fn normalize(v: Vec2) -> Vec2 {
    let len = v.length();
    if len <= 1e-4 {
        v
    } else {
        v / len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::from_xywh(0.0, 0.0, 1000.0, 1000.0)
    }

    /// Postać stoi w miejscu i nie dryfuje — brak wejścia to zero ruchu.
    #[test]
    fn idle_stands_still() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);
        for _ in 0..10 {
            c.update(0.016, Vec2::ZERO, false, bounds(), cfg);
        }
        assert_eq!(c.state, MotionState::Idle);
        assert_eq!(c.velocity, Vec2::ZERO);
        assert!((c.position - Vec2::new(500.0, 500.0)).length() < 1e-4);
    }

    /// Chód przesuwa postać o `speed * dt` w stronę wejścia.
    #[test]
    fn walk_moves_at_configured_speed() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);
        let dt = 0.5;
        c.update(dt, Vec2::new(1.0, 0.0), false, bounds(), cfg);

        assert_eq!(c.state, MotionState::Walk);
        assert!((c.velocity.x - cfg.walk_speed).abs() < 1e-3);
        assert!((c.position.x - (500.0 + cfg.walk_speed * dt)).abs() < 1e-3);
    }

    /// Ruch po skosie nie jest szybszy niż po osi — inaczej gracz uciekałby
    /// „ukośnie" z prędkością sqrt(2) razy większą.
    #[test]
    fn diagonal_is_not_faster() {
        let cfg = CharacterConfig::default();
        let mut straight = Character::new(Vec2::new(500.0, 500.0), cfg);
        let mut diagonal = Character::new(Vec2::new(500.0, 500.0), cfg);

        straight.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        diagonal.update(0.1, Vec2::new(1.0, 1.0), false, bounds(), cfg);

        assert!((diagonal.velocity.length() - cfg.walk_speed).abs() < 1e-3);
        assert!((straight.velocity.length() - diagonal.velocity.length()).abs() < 1e-3);
    }

    /// Sprint jest szybszy od chodu i daje inne tempo animacji.
    #[test]
    fn running_is_faster_and_animates_differently() {
        let cfg = CharacterConfig::default();
        let mut walk = Character::new(Vec2::new(500.0, 500.0), cfg);
        let mut run = Character::new(Vec2::new(500.0, 500.0), cfg);

        walk.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        run.update(0.1, Vec2::new(1.0, 0.0), true, bounds(), cfg);

        assert_eq!(walk.state, MotionState::Walk);
        assert_eq!(run.state, MotionState::Run);
        assert!(run.velocity.length() > walk.velocity.length());
        // ten sam wiersz (ta sama twarz), ale inne tempo => inna animacja
        assert_eq!(walk.anim.clip().row, run.anim.clip().row);
        assert!((run.anim.clip().fps - cfg.run_fps).abs() < 1e-5);
        assert!((walk.anim.clip().fps - cfg.walk_fps).abs() < 1e-5);
    }

    /// Kierunek zmienia się zgodnie z ruchem, a wiersz arkusza za nim.
    #[test]
    fn facing_follows_motion() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);

        c.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Right);
        assert_eq!(c.anim.clip().row, Facing::Right.row());

        c.update(0.1, Vec2::new(0.0, -1.0), false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Down);

        c.update(0.1, Vec2::new(0.0, 1.0), false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Up);

        c.update(0.1, Vec2::new(-1.0, 0.0), false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Left);
    }

    /// Postać stojąca **nie zmienia** kierunku — inaczej po zatrzymaniu
    /// odwróciłaby się w losową stronę.
    #[test]
    fn standing_keeps_last_facing() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);
        c.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Right);

        c.update(0.1, Vec2::ZERO, false, bounds(), cfg);
        assert_eq!(c.facing, Facing::Right, "kierunek przeżywa postój");
        assert_eq!(c.state, MotionState::Idle);
    }

    /// Postać nie wychodzi poza świat — przy każdej krawędzi.
    #[test]
    fn clamps_to_bounds_on_every_side() {
        let cfg = CharacterConfig::default();
        let f = cfg.footprint;
        let b = bounds();

        let mut left = Character::new(b.center(), cfg);
        for _ in 0..200 {
            left.update(0.1, Vec2::new(-1.0, 0.0), true, b, cfg);
        }
        assert!(left.position.x >= b.left() + f.x - 1e-3);

        let mut right = Character::new(b.center(), cfg);
        for _ in 0..200 {
            right.update(0.1, Vec2::new(1.0, 0.0), true, b, cfg);
        }
        assert!(right.position.x <= b.right() - f.x + 1e-3);

        let mut bottom = Character::new(b.center(), cfg);
        for _ in 0..200 {
            bottom.update(0.1, Vec2::new(0.0, -1.0), true, b, cfg);
        }
        assert!(bottom.position.y >= b.bottom() + f.y - 1e-3);

        let mut top = Character::new(b.center(), cfg);
        for _ in 0..200 {
            top.update(0.1, Vec2::new(0.0, 1.0), true, b, cfg);
        }
        assert!(top.position.y <= b.top() - f.y + 1e-3);
    }

    /// Najważniejsza zgodność: klatka klipu **musi mieścić się** w arkuszu,
    /// inaczej animacja pokazuje cudze kadry z sąsiedniego arkusza.
    #[test]
    fn clips_fit_the_sheets() {
        let handle = uran_asset::Handle::default();
        let sheets = CharacterSheets {
            idle: SpriteSheet::new(handle, Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0)),
            walk: SpriteSheet::new(handle, Vec2::new(64.0, 128.0), Vec2::new(640.0, 512.0)),
            run: SpriteSheet::new(handle, Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0)),
        };
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);

        for (input, run) in [
            (Vec2::new(1.0, 0.0), false),
            (Vec2::new(1.0, 0.0), true),
            (Vec2::ZERO, false),
        ] {
            for _ in 0..60 {
                c.update(0.05, input, run, bounds(), cfg);
            }
            let clip = c.anim.clip();
            assert!(
                c.current_uv(&sheets).is_some(),
                "klatka ({}, {}) poza arkuszem {:?}",
                c.anim.frame(),
                clip.row,
                c.state
            );
        }
    }

    /// Klatka animacji naprawdę się zmienia w czasie chodu — inaczej
    /// „animacja" stałaby w miejscu i nie zgadzała się z ruchem.
    #[test]
    fn walk_advances_frames() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);
        let mut frames = Vec::new();
        for _ in 0..30 {
            c.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
            frames.push(c.anim.frame());
        }
        assert!(
            frames.windows(2).any(|w| w[0] != w[1]),
            "klatka nie zmienia się: {frames:?}"
        );
    }

    /// Po wejściu w ruch i zatrzymaniu animacja wraca do idle.
    #[test]
    fn state_transitions_idle_walk_idle() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);

        c.update(0.1, Vec2::ZERO, false, bounds(), cfg);
        assert_eq!(c.state, MotionState::Idle);

        c.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        assert_eq!(c.state, MotionState::Walk);

        c.update(0.1, Vec2::ZERO, false, bounds(), cfg);
        assert_eq!(c.state, MotionState::Idle);
    }

    /// Przesunięcie decyduje, gdzie sprite ląduje względem stóp.
    #[test]
    fn sprite_sits_above_feet() {
        let cfg = CharacterConfig::default();
        let c = Character::new(Vec2::new(500.0, 500.0), cfg);
        let center = c.sprite_center(cfg);
        assert!(
            center.y < c.position.y,
            "sprite jest nad stopami, nie pod nimi"
        );
        assert!(
            (center.x - c.position.x).abs() < 1e-6,
            "x nie jest przesunięty"
        );
    }

    /// Ujemny `dt` nie przesuwa postaci w drugą stronę.
    #[test]
    fn negative_dt_does_not_teleport_backwards() {
        let cfg = CharacterConfig::default();
        let mut c = Character::new(Vec2::new(500.0, 500.0), cfg);
        c.update(0.1, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        let x = c.position.x;
        c.update(-1.0, Vec2::new(1.0, 0.0), false, bounds(), cfg);
        assert!(c.position.x >= x - 1e-3);
    }
}
