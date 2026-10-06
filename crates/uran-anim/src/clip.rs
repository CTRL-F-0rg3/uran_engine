//! Klip animacji: seria klatek odtwarzana z zadaną prędkością.
//!
//! Klip sam nie wie, w której komórce arkusza leży — dostaje gotowy
//! [`SpriteSheet`] i wiersz, a sam odpowiada tylko za *która* klatka jest
//! aktualna i jak przesuwa się w czasie.

use uran_ecs::UvRect;
use uran_math::{Rect, Vec2};

use crate::sheet::SpriteSheet;

/// Jak klip zachowuje się po ostatniej klatce.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Po ostatniej klatce wraca do początku — chód, bieg.
    #[default]
    Repeat,
    /// Zatrzymuje się na ostatniej klatce — cios, śmierć.
    Clamp,
    /// Idzie do przodu i wraca — jednorazowa animacja „w tę i z powrotem".
    PingPong,
}

/// Jedna animacja postaci: wiersz arkusza + ile klatek + jak szybko.
#[derive(Debug, Clone, Copy)]
pub struct AnimationClip {
    /// Wiersz w arkuszu (`0` = przód, `1` = lewo, `2` = prawo, `3` = tył).
    pub row: u16,
    /// Liczba klatek w animacji.
    pub frames: u16,
    /// Tempo: klatek na sekundę.
    pub fps: f32,
    /// Zachowanie po ostatniej klatce.
    pub mode: LoopMode,
}

impl AnimationClip {
    /// Klip z tempem w klatkach na sekundę i domyślnym zapętleniem.
    pub const fn new(row: u16, frames: u16, fps: f32) -> Self {
        Self {
            row,
            frames,
            fps,
            mode: LoopMode::Repeat,
        }
    }

    /// Klip odtwarzany raz, zatrzymujący się na ostatniej klatce.
    pub const fn once(row: u16, frames: u16, fps: f32) -> Self {
        Self {
            row,
            frames,
            fps,
            mode: LoopMode::Clamp,
        }
    }

    pub const fn with_mode(mut self, mode: LoopMode) -> Self {
        self.mode = mode;
        self
    }

    /// Czas trwania jednego przejścia w sekundach (0 dla pustego klipu).
    pub fn duration(&self) -> f32 {
        if self.frames == 0 || self.fps <= 0.0 {
            0.0
        } else {
            self.frames as f32 / self.fps
        }
    }

    /// Ustawia liczbę klatek, przycinając ją do szerokości arkusza.
    ///
    /// Chroni przed najczęstszym błędem przy łączeniu assetów: klip
    /// „chodzenia" z 10 klatkami podstawiony pod arkusz 8-klatkowy
    /// pokazywałby kawałki obcego arkusza zamiast pętli.
    pub fn frames_clamped_to(&mut self, sheet: &SpriteSheet) {
        self.frames = self.frames.min(sheet.cols()).max(1);
        self.row = self.row.min(sheet.rows().saturating_sub(1));
    }
}

/// Stan odtwarzania klipu: numer klatki + czas w klipie.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Playback {
    /// Numer klatki (0-based) w obrębie klipu.
    pub frame: u16,
    /// Czas od początku klipu (sekundy).
    pub elapsed: f32,
    /// Czy klip się zakończył (`Clamp` na ostatniej klatce).
    pub finished: bool,
}

/// Odtwarza klip: przesuwa czas i wystawia bieżącą klatkę.
#[derive(Debug, Clone, Copy)]
pub struct Animator {
    clip: AnimationClip,
    state: Playback,
}

impl Animator {
    /// Animator z klipem, zatrzymany na pierwszej klatce.
    pub fn new(clip: AnimationClip) -> Self {
        Self {
            clip,
            state: Playback::default(),
        }
    }

    pub fn clip(&self) -> AnimationClip {
        self.clip
    }

    /// Ustawia nowy klip i wraca do jego pierwszej klatki.
    ///
    /// `restart = false` nie cofa się do początku, gdy to ten sam klip —
    /// dzięki temu przełączanie `Idle -> Walk -> Idle` przy chodzeniu
    /// w kółko nie „zacina" animacji co krok.
    pub fn play(&mut self, clip: AnimationClip, restart: bool) {
        let same = clip.row == self.clip.row
            && clip.frames == self.clip.frames
            && clip.fps == self.clip.fps
            && clip.mode == self.clip.mode;
        if same && !restart {
            return;
        }
        self.clip = clip;
        self.state = Playback::default();
    }

    /// Podmienia klip bez cofania do początku, jeśli to ta sama animacja.
    pub fn play_or_keep(&mut self, clip: AnimationClip) {
        self.play(clip, false);
    }

    pub fn playback(&self) -> Playback {
        self.state
    }

    pub fn frame(&self) -> u16 {
        self.state.frame
    }

    pub fn elapsed(&self) -> f32 {
        self.state.elapsed
    }

    pub fn finished(&self) -> bool {
        self.state.finished
    }

    /// Postęp klipu w zakresie `0..1` (do debugu i debug draw).
    ///
    /// **Zakończony klip to zawsze `1.0`.** Bez tego klip `Clamp` zatrzymuje
    /// się na klatce `frames - 1`, czyli nigdy nie osiąga `1.0` — a efekt
    /// sterowany postępem (rozjaśnianie, zanikanie) nigdy by nie zniknął
    /// do końca. Klip `Repeat` nigdy nie jest `finished`, więc zachowuje
    /// się jak zwykle.
    ///
    /// Klip bez czasu trwania (`fps <= 0` albo `frames == 0`) ma `0.0`:
    /// nie ma czego liczyć, a `1.0` kłamałoby o zakończeniu.
    pub fn progress(&self) -> f32 {
        let d = self.clip.duration();
        if d <= 0.0 {
            0.0
        } else if self.state.finished {
            1.0
        } else {
            (self.state.elapsed / d).clamp(0.0, 1.0)
        }
    }

    /// Przesuwa animację o `dt` sekund.
    pub fn tick(&mut self, dt: f32) {
        if self.clip.frames == 0 || self.clip.fps <= 0.0 {
            self.state.frame = 0;
            self.state.finished = true;
            return;
        }
        // Ujemny dt (pauza, przewijanie) nie cofa animacji.
        self.state.elapsed += dt.max(0.0);

        let count = self.clip.frames as i64;
        match self.clip.mode {
            LoopMode::Repeat => {
                let total = (self.state.elapsed * self.clip.fps).floor() as i64;
                self.state.frame = total.rem_euclid(count) as u16;
                self.state.finished = false;
            }
            LoopMode::Clamp => {
                let idx = (self.state.elapsed * self.clip.fps).floor() as i64;
                if idx >= count - 1 {
                    self.state.frame = self.clip.frames - 1;
                    self.state.finished = true;
                } else {
                    self.state.frame = idx.max(0) as u16;
                    self.state.finished = false;
                }
            }
            LoopMode::PingPong => {
                // Okres pełnej obwiedni to 2 * (N-1) klatek.
                let span = ((self.clip.frames - 1) * 2).max(1) as i64;
                let idx = (self.state.elapsed * self.clip.fps).floor() as i64 % span;
                let idx = if idx < 0 { idx + span } else { idx };
                let bounce = if idx < count { idx } else { span - idx };
                self.state.frame = bounce as u16;
                self.state.finished = false;
            }
        }
    }

    /// Bieżąca klatka jako UV gotowe do `draw_texture`.
    pub fn current_uv(&self, sheet: &SpriteSheet) -> Option<UvRect> {
        sheet.uv_for(self.state.frame, self.clip.row)
    }

    /// Prostokąt świata dla bieżącej klatki, zakotwiczony środkiem w
    /// `center`.
    ///
    /// Rozmiar bierze z arkusza (piksele klatki = jednostki świata przy
    /// skali `1.0`), `scale` powiększa całego sprite'a, a `y_offset`
    /// podnosi go względem punktu „stóp".
    pub fn draw_rect(&self, sheet: &SpriteSheet, center: Vec2, scale: f32, y_offset: f32) -> Rect {
        let size = sheet.frame_world_size() * scale;
        Rect::from_center(center + Vec2::new(0.0, y_offset), size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(row: u16, frames: u16, fps: f32) -> AnimationClip {
        AnimationClip::new(row, frames, fps)
    }

    /// Tempo w klatkach na sekundę przekłada się na czas trwania.
    #[test]
    fn duration_is_frames_over_fps() {
        assert!((clip(0, 8, 8.0).duration() - 1.0).abs() < 1e-5);
        assert!((clip(0, 10, 10.0).duration() - 1.0).abs() < 1e-5);
        assert_eq!(clip(0, 0, 8.0).duration(), 0.0, "pusty klip ma czas 0");
    }

    /// `Repeat` zawija klatki — po obiegu wracamy do początku, inaczej
    /// animacja chodu zamarzałaby po pierwszym przejściu.
    #[test]
    fn repeat_wraps_frames() {
        let mut a = Animator::new(clip(0, 4, 4.0));
        let mut seen = Vec::new();
        // 4 klatki przy 4 fps => jedna klatka = 0.25 s
        for _ in 0..5 {
            a.tick(0.25);
            seen.push(a.frame());
        }
        assert_eq!(seen, vec![1, 2, 3, 0, 1]);
        assert!(!a.finished());
    }

    /// `Clamp` trzyma ostatnią klatkę i oznacza koniec — potrzebne dla
    /// ataku, który ma się zatrzymać, a nie zapętlać.
    #[test]
    fn clamp_holds_last_frame() {
        let mut a = Animator::new(clip(1, 3, 3.0).with_mode(LoopMode::Clamp));
        a.tick(0.34);
        assert_eq!(a.frame(), 1);
        assert!(!a.finished());
        a.tick(0.34);
        assert_eq!(a.frame(), 2);
        a.tick(1.0);
        assert_eq!(a.frame(), 2, "ostaje na ostatniej klatce");
        assert!(a.finished());
    }

    /// `PingPong` idzie do przodu i wraca: 0,1,2,1,0,1,...
    #[test]
    fn ping_pong_bounces() {
        let mut a = Animator::new(clip(0, 3, 4.0).with_mode(LoopMode::PingPong));
        let mut seen = Vec::new();
        for _ in 0..5 {
            a.tick(0.25);
            seen.push(a.frame());
        }
        assert_eq!(seen, vec![1, 2, 1, 0, 1]);
    }

    /// Klip pusty albo z `fps <= 0` nie dzieli przez zero i kończy się
    /// natychmiast zamiast wisieć na klatce.
    #[test]
    fn degenerate_clips_are_safe() {
        let mut a = Animator::new(clip(0, 0, 8.0));
        a.tick(1.0);
        assert_eq!(a.frame(), 0);
        assert!(a.finished());

        let mut b = Animator::new(clip(0, 4, 0.0));
        b.tick(1.0);
        assert_eq!(b.frame(), 0);
        assert!(b.finished());
        assert_eq!(b.progress(), 0.0);
    }

    /// Klip wybrany ponownie wraca do pierwszej klatki, ale ten sam klip
    /// zachowuje pozycję — inaczej przełączanie idle/walk skakałoby w kółko.
    #[test]
    fn play_restarts_only_on_change() {
        let mut a = Animator::new(clip(0, 8, 8.0));
        a.tick(0.5);
        assert_eq!(a.frame(), 4);

        a.play_or_keep(clip(0, 8, 8.0));
        assert_eq!(a.frame(), 4, "ten sam klip nie cofa się do zera");

        a.play_or_keep(clip(1, 8, 8.0));
        assert_eq!(a.frame(), 0, "inny klip startuje od początku");
        assert_eq!(a.clip().row, 1);

        a.tick(0.5);
        a.play(clip(1, 8, 8.0), true);
        assert_eq!(a.frame(), 0, "wymuszony restart tego samego klipu");
    }

    /// Postęp rośnie od 0 do 1 i nie przekracza 1.
    #[test]
    fn progress_is_bounded() {
        let mut a = Animator::new(clip(0, 4, 4.0));
        assert_eq!(a.progress(), 0.0);
        a.tick(0.5);
        assert!((a.progress() - 0.5).abs() < 1e-5);
        a.tick(5.0);
        assert_eq!(a.progress(), 1.0, "nie wychodzimy poza 1");
    }

    /// Ujemny `dt` nie cofa animacji — zdarza się przy obsłudze pauzy.
    #[test]
    fn negative_dt_does_not_rewind() {
        let mut a = Animator::new(clip(0, 4, 4.0));
        a.tick(0.5);
        let f = a.frame();
        a.tick(-1.0);
        assert!(a.frame() >= f, "cofnięcie czasu nie wraca do początku");
    }

    /// Zakończony klip `Clamp` musi raportować `1.0`, inaczej efekt
    /// sterowany postępem nigdy nie docierałby do końca.
    #[test]
    fn finished_clip_reaches_full_progress() {
        let mut a = Animator::new(clip(0, 4, 4.0).with_mode(LoopMode::Clamp));
        a.tick(0.5);
        assert!(a.progress() < 1.0, "w trakcie nie jest jeszcze koniec");
        a.tick(1.0);
        assert!(a.finished());
        assert_eq!(a.progress(), 1.0, "skończony klip to 100%");
    }

    /// Klip `Repeat` nigdy nie jest `finished`, więc postęp liczymy normalnie,
    /// a przycięcie do `1.0` działa jak wcześniej.
    #[test]
    fn repeating_clip_keeps_winding_progress() {
        let mut a = Animator::new(clip(0, 4, 4.0));
        a.tick(5.0);
        assert!(!a.finished());
        assert!((a.progress() - 1.0).abs() < 1e-5, "postęp przycięty do 1.0");
    }

    /// `frames_clamped_to` chroni przed za długim klipem podstawionym pod
    /// za krótki arkusz — bez niego pokazywałyby się kawałki innego pliku.
    #[test]
    fn frames_are_clamped_to_sheet() {
        let handle = uran_asset::Handle::default();
        let sheet = SpriteSheet::new(handle, Vec2::new(64.0, 128.0), Vec2::new(512.0, 512.0));
        assert_eq!(sheet.cols(), 8);

        let mut c = AnimationClip::new(0, 10, 10.0);
        c.frames_clamped_to(&sheet);
        assert_eq!(c.frames, 8, "klip nie może być dłuższy niż arkusz");
    }
}
