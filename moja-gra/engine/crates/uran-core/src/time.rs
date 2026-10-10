//! Czas i klatki — wszystko, czego potrzebuje fizyka i animacje.

use std::time::{Duration, Instant};

/// Delta i czas skumulowany.
///
/// Czas jest liczony od `new()` (czyli od startu aplikacji), a nie od
/// unix-epoch, więc liczby są małe i nie tracą precyzji.
#[derive(Debug)]
pub struct Time {
    last: Instant,
    start: Instant,
    delta: Duration,
    elapsed: Duration,
    frame: u64,
    /// Wygładzona wartość FPS (uśredniana krokowo, bez skoków).
    fps: f32,
    /// Maksymalna delta, jaką raportujemy (ochrona przed teleportacją
    /// obiektów po zawieszeniu okna / debug breaku).
    max_delta: Duration,
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

impl Time {
    /// Limit delty: 100 ms (10 FPS) — niżej gra i tak nie ma sensu.
    pub const DEFAULT_MAX_DELTA: Duration = Duration::from_millis(100);

    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last: now,
            start: now,
            delta: Duration::ZERO,
            elapsed: Duration::ZERO,
            frame: 0,
            fps: 0.0,
            max_delta: Self::DEFAULT_MAX_DELTA,
        }
    }

    /// Wywoływane raz na klatkę — aktualizuje wszystkie liczniki.
    pub fn tick(&mut self) {
        let now = Instant::now();
        self.delta = now.duration_since(self.last).min(self.max_delta);
        self.last = now;
        self.elapsed = now.duration_since(self.start);
        self.frame += 1;

        if self.delta > Duration::ZERO {
            let instant_fps = 1.0 / self.delta.as_secs_f32();
            // waga 0.1 zbija drgania, ale reaguje na zmianę obciążenia
            self.fps = if self.frame <= 1 {
                instant_fps
            } else {
                self.fps * 0.9 + instant_fps * 0.1
            };
        }
    }

    /// Ponownie zeruje liczniki (np. po pauzie).
    pub fn reset(&mut self) {
        let now = Instant::now();
        self.last = now;
        self.start = now;
        self.delta = Duration::ZERO;
        self.elapsed = Duration::ZERO;
        self.frame = 0;
        self.fps = 0.0;
    }

    pub fn set_max_delta(&mut self, max: Duration) {
        self.max_delta = max;
    }

    /// Czas od ostatniej klatki (bez limitu — używaj `delta_seconds` w grze).
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// **Najważniejsze**: sekundy od ostatniej klatki.
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    pub fn delta_millis(&self) -> f32 {
        self.delta.as_secs_f32() * 1000.0
    }

    /// Czas od startu aplikacji.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn elapsed_seconds(&self) -> f32 {
        self.elapsed.as_secs_f32()
    }

    /// Numer klatki (od 1).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Wygładzone FPS.
    pub fn fps(&self) -> f32 {
        self.fps
    }

    /// Suwak „co N sekundy" — np. `should_tick(0.5)` na autosave.
    pub fn interval(&self, seconds: f32) -> bool {
        let period = seconds.max(0.0);
        period > 0.0
            && (self.frame == 1
                || (self.elapsed_seconds() / period).floor()
                    > (self.elapsed_seconds() - self.delta_seconds()) / period)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_zero() {
        let t = Time::new();
        assert_eq!(t.frame(), 0);
        assert_eq!(t.delta(), Duration::ZERO);
        assert_eq!(t.elapsed(), Duration::ZERO);
    }

    #[test]
    fn tick_advances_frame_and_time() {
        let mut t = Time::new();
        t.tick();
        t.tick();
        assert_eq!(t.frame(), 2);
        assert!(t.elapsed() > Duration::ZERO);
        assert!(t.elapsed_seconds() > 0.0);
    }

    #[test]
    fn delta_is_clamped() {
        let mut t = Time::new();
        t.set_max_delta(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(5));
        t.tick();
        assert!(
            t.delta() <= Duration::from_millis(1),
            "delta nie może urosnąć powyżej limitu"
        );
    }

    #[test]
    fn interval_triggers() {
        let mut t = Time::new();
        t.tick();
        assert!(t.interval(0.5), "pierwsza klatka zawsze odpala interwał");
    }

    #[test]
    fn reset_clears_counters() {
        let mut t = Time::new();
        t.tick();
        t.reset();
        assert_eq!(t.frame(), 0);
        assert_eq!(t.elapsed(), Duration::ZERO);
    }
}
