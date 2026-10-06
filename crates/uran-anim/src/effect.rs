//! Efekt jednorazowy: animacja, która gra się **raz** i znika.
//!
//! To brakujące ogniwo między `Character` a zaklęciami. Postać ma cztery
//! wiersze arkusza (cztery kierunki) i zapętla się w nieskończoność, a
//! efekt ma jeden blok klatek (pojawienie → rozbłysk → zanik) i kończy się.
//!
//! ```ignore
//! let mut effect = SpriteEffect::new(sheet, EffectConfig::default());
//! if input.cast_pressed() {
//!     effect.restart();
//! }
//! effect.update(ctx.dt());
//! if effect.is_playing() {
//!     effect.draw(&mut ctx.gfx, player_position);
//! }
//! ```
//!
//! ## Dlaczego własny typ, a nie `Animator`
//!
//! `Animator` zna czas i numer klatki, ale nie odpowiada na pytanie „czy ta
//! animacja jeszcze trwa?". Jednorazowy efekt musi wiedzieć, kiedy **zniknąć**,
//! bo zapętlona tarcza wygląda jak zawieszenie gry, a nie magia.

use uran_ecs::{BlendMode, UvRect};
use uran_math::{Color, Rect, Vec2};
use uran_render::Graphics;

use crate::clip::{AnimationClip, Animator, LoopMode};
use crate::sheet::SpriteSheet;

/// Jak wygląda i jak długo trwa efekt.
#[derive(Debug, Clone, Copy)]
pub struct EffectConfig {
    /// Tempo w klatkach na sekundę.
    pub fps: f32,
    /// Skala klatki względem rozmiaru w arkuszu.
    pub scale: f32,
    /// Przesunięcie w pionie względem punktu, w którym go przywołujemy.
    pub y_offset: f32,
    /// Warstwa rysowania — efekt musi leżeć nad postacią.
    pub layer: i32,
    /// Tryb mieszania; świecące VFX dobrze wyglądają addytywnie.
    pub additive: bool,
}

impl Default for EffectConfig {
    /// Umiarkowane tempo i skala `1.0` — czyli klatka jest tyle, ile ma
    /// pikseli w pliku, co najczęściej jest właśnie pożądanym rozmiarem.
    fn default() -> Self {
        Self {
            fps: 20.0,
            scale: 1.0,
            y_offset: 0.0,
            layer: 20,
            additive: true,
        }
    }
}

/// Jednorazowa animacja sprite'owa odtwarzana po kolei przez cały arkusz.
///
/// Klatki idą **wierszami** ([`SpriteSheet::uv_for_flat`]), a efekt zatrzymuje
/// się na ostatniej z nich zamiast wracać do początku.
#[derive(Debug, Clone)]
pub struct SpriteEffect {
    sheet: SpriteSheet,
    anim: Animator,
    config: EffectConfig,
    /// Czy animacja jeszcze trwa (po zakończeniu `update` przestaje).
    playing: bool,
}

impl SpriteEffect {
    /// Efekt grający **wszystkie** klatki arkuszu od początku do końca.
    ///
    /// Nowy efekt jest zatrzymany (`playing == false`) — żeby grafika nie
    /// migała na starcie gry, zanim ktokolwiek rzuci zaklęcie.
    pub fn new(sheet: SpriteSheet, config: EffectConfig) -> Self {
        // `row` jest tu bez znaczenia: klatki idą liniowo, a nie po wierszach
        // kierunku. Zostawiamy 0, żeby `play_or_keep` nie resetował klipu
        // przy każdym wywołaniu.
        let clip = AnimationClip {
            row: 0,
            frames: sheet.frame_count().min(u16::MAX as u32) as u16,
            fps: config.fps,
            mode: LoopMode::Clamp,
        };
        let mut anim = Animator::new(clip);
        anim.play(clip, true);
        Self {
            sheet,
            anim,
            config,
            playing: false,
        }
    }

    /// Uruchamia efekt od pierwszej klatki.
    ///
    /// Powtórne `restart()` w trakcie trwania **cofa** animację do początku —
    /// rzucenie zaklęcia drugi raz musi wyglądać jak nowe, nie jak skok
    /// w środek trwającego efektu.
    pub fn restart(&mut self) {
        let clip = self.anim.clip();
        self.anim.play(clip, true);
        self.playing = true;
    }

    /// Przesuwa efekt o `dt` i sam zdejmuje go ze sceny po ostatniej klatce.
    pub fn update(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        self.anim.tick(dt);
        if self.anim.finished() {
            self.playing = false;
        }
    }

    /// Czy efekt trwa i ma być rysowany.
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Czy skończył się i zniknął (`!is_playing()` po pełnym przebiegu).
    pub fn is_finished(&self) -> bool {
        !self.playing && self.anim.finished()
    }

    /// Postęp `0..1` — do debugu i do rozjaśniania efektu w trakcie.
    pub fn progress(&self) -> f32 {
        self.anim.progress()
    }

    /// Aktualny numer klatki (po kolei przez cały arkusz).
    pub fn frame(&self) -> u16 {
        self.anim.frame()
    }

    pub fn sheet(&self) -> &SpriteSheet {
        &self.sheet
    }

    pub fn config(&self) -> EffectConfig {
        self.config
    }

    /// UV bieżącej klatki albo `None`, gdy nie mamy już czego rysować.
    pub fn current_uv(&self) -> Option<UvRect> {
        if !self.playing {
            return None;
        }
        self.sheet.uv_for_flat(self.anim.frame())
    }

    /// Prostokąt świata dla bieżącej klatki, zakotwiczony w `center`.
    pub fn frame_rect(&self, center: Vec2) -> Rect {
        let size = self.sheet.frame_world_size() * self.config.scale;
        Rect::from_center(center + Vec2::new(0.0, self.config.y_offset), size)
    }

    /// Rysuje bieżącą klatkę nad postacią (nic nie robi, gdy efekt nie gra).
    ///
    /// Kolor jest ustawiany na biel **za każdym razem**: pędzel trzyma stan
    /// z poprzedniego rysowania, a `draw_texture` mnoży teksturę przez ten
    /// kolor — bez resetu efekt wyszedłby przebarwiony resztą HUD-u.
    pub fn draw(&self, gfx: &mut Graphics<'_>, center: Vec2) {
        let Some(uv) = self.current_uv() else {
            return;
        };
        if self.config.additive {
            gfx.blend(BlendMode::Additive);
        }
        gfx.layer(self.config.layer)
            .color(Color::WHITE)
            .draw_texture(self.sheet.texture(), self.frame_rect(center), uv);
        if self.config.additive {
            gfx.blend(BlendMode::Alpha);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uran_asset::{Handle, Image};

    /// Arkusz efektu 5x4 (jak arkusze VFX Pipoya) i pusty uchwyt —
    /// do liczenia UV nie potrzebujemy prawdziwej tekstury.
    fn effect_sheet() -> SpriteSheet {
        let handle = Handle::<Image>::default();
        SpriteSheet::new(handle, Vec2::new(192.0, 192.0), Vec2::new(960.0, 768.0))
    }

    fn effect() -> SpriteEffect {
        let mut config = EffectConfig::default();
        config.fps = 10.0; // 20 klatek = 2 s przebiegu
        SpriteEffect::new(effect_sheet(), config)
    }

    /// Świeżo utworzony efekt **nie gra** — inaczej zaklęcie migoczełoby
    /// na starcie gry, zanim gracz zdążył cokolwiek rzucić.
    #[test]
    fn new_effect_does_not_play_on_its_own() {
        let e = effect();
        assert!(!e.is_playing());
        assert!(
            e.current_uv().is_none(),
            "nie rysujemy nierozpoczętego efektu"
        );
    }

    /// `restart` odpala efekt, a po pełnym przebiegu sam go wyłącza.
    ///
    /// Nie polegamy na konkretnych czasach z dokładnością do 1e-6: liczby
    /// zmiennoprzecinkowe kumulują błąd przy `dt`, a `floor()` potrafi
    /// przesunąć granicę klatki o jedną. Zamiast tego tykamy krokami
    /// i sprawdzamy, **kiedy** efekt przestaje grać.
    #[test]
    fn runs_once_then_stops() {
        let mut e = effect();
        e.restart();
        assert!(e.is_playing());

        // 10 klatek przy 10 fps to połowa arkusza — efekt jeszcze trwa.
        for _ in 0..10 {
            e.update(0.1);
        }
        assert!(e.is_playing(), "efekt nie może zniknąć przed końcem");

        // Po pełnym przebiegu znika sam — gra nie musi go wyłączać ręcznie.
        for _ in 0..15 {
            e.update(0.1);
        }
        assert!(!e.is_playing());
        assert!(e.is_finished());
        assert!(e.current_uv().is_none(), "po skończeniu nie rysujemy nic");
    }

    /// Powtórne rzucenie wraca do pierwszej klatki, zamiast kontynuować
    /// efekt od miejsca, w którym był.
    #[test]
    fn restart_goes_back_to_first_frame() {
        let mut e = effect();
        e.restart();
        e.update(1.0);
        let mid = e.frame();
        assert!(mid > 0, "w połowie przebiegu jesteśmy już dalej");

        e.restart();
        assert_eq!(e.frame(), 0, "nowe rzucenie zaczyna od nowa");
        assert!(e.is_playing());
    }

    /// Klatki idą kolejno przez **cały** arkusz, wierszami — nie zapętlają
    /// się na końcu pierwszego wiersza.
    #[test]
    fn frames_walk_through_the_whole_sheet() {
        let mut e = effect();
        e.restart();
        let mut seen = Vec::new();
        for _ in 0..20 {
            seen.push(e.frame());
            e.update(0.1);
        }
        assert_eq!(
            seen,
            (0..20).collect::<Vec<_>>(),
            "klatki muszą iść po kolei"
        );
    }

    /// Numer klatki zawsze wskazuje prawdziwą klatkę arkusza — inaczej
    /// efekt pokazywałby puste kawałki obcego arkusza.
    ///
    /// Pętla kończy się razem z efektem: po ostatniej klatce `current_uv`
    /// celowo zwraca `None`, bo nie ma już czego rysować.
    #[test]
    fn current_frame_is_always_inside_the_sheet() {
        let mut e = effect();
        e.restart();
        for _ in 0..40 {
            if !e.is_playing() {
                break;
            }
            assert!(
                e.current_uv().is_some(),
                "klatka {} poza arkuszem",
                e.frame()
            );
            e.update(0.05);
        }
        assert!(e.is_finished(), "przebiegliśmy cały arkusz i skończyliśmy");
    }

    /// `update` na zatrzymanym efekcie nic nie robi — inaczej gra startowałaby
    /// z efektem przesuniętym o jeden krok.
    #[test]
    fn idle_effect_does_not_advance() {
        let mut e = effect();
        e.update(10.0);
        assert_eq!(e.frame(), 0);
        assert!(!e.is_playing());
    }

    /// Efekt z zerową liczbą klatek nie zapętla się i nie zawiesza pętli.
    #[test]
    fn empty_sheet_is_safe() {
        let handle = Handle::<Image>::default();
        let sheet = SpriteSheet::new(handle, Vec2::new(192.0, 192.0), Vec2::splat(0.0));
        let mut e = SpriteEffect::new(sheet, EffectConfig::default());
        e.restart();
        e.update(1.0);
        assert!(!e.is_playing());
        assert_eq!(e.frame(), 0);
    }

    /// Skala z konfiguracji przeskalowuje całą klatkę względem arkusza.
    #[test]
    fn scale_resizes_the_frame_rect() {
        let mut e = effect();
        e.config.scale = 2.0;
        e.restart();
        let rect = e.frame_rect(Vec2::ZERO);
        assert_eq!(rect.size(), Vec2::new(384.0, 384.0));
        assert_eq!(
            rect.center(),
            Vec2::ZERO,
            "klatka jest zakotwiczona w punkcie"
        );
    }
}
