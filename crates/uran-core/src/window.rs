use uran_math::Color;

/// Opis okna aplikacji (patrz też `windowed()` jako punkt wejścia).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
    pub clear_color: Color,
    /// Synchronizacja klatek z monitorem.
    pub vsync: bool,
    /// Liczba próbek MSAA (1 = wyłączone, 4 = ładniejsze krawędzie).
    pub samples: u32,
    /// Kursor systemowy nad oknem.
    pub cursor_visible: bool,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: "Uran Engine".to_string(),
            width: 1280,
            height: 720,
            resizable: true,
            clear_color: Color::BLACK, // Domyślnie czarne tło
            vsync: true,
            samples: 1,
            cursor_visible: true,
        }
    }
}

/// Punkt wejścia do konfiguracji okna.
/// Przykład użycia: `windowed(800, 600).title("Gra").resizable(true).background(0x000000)`
pub fn windowed(width: u32, height: u32) -> WindowDescriptor {
    WindowDescriptor {
        width,
        height,
        ..Default::default()
    }
}

impl WindowDescriptor {
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Ustawia kolor tła (clear color) na podstawie wartości hex (np. 0x000000).
    pub fn background(mut self, hex_color: u32) -> Self {
        self.clear_color = Color::from_hex(hex_color);
        self
    }

    /// Kolor tła z pełnym kanałem alfa.
    pub fn background_rgba(mut self, color: Color) -> Self {
        self.clear_color = color;
        self
    }

    /// Synchronizacja z monitorem (domyślnie włączona = 60 FPS).
    pub fn vsync(mut self, vsync: bool) -> Self {
        self.vsync = vsync;
        self
    }

    /// Antyaliasing: 1 (domyślnie), 2 lub 4.
    pub fn samples(mut self, samples: u32) -> Self {
        self.samples = samples.clamp(1, 4);
        self
    }

    pub fn cursor_visible(mut self, visible: bool) -> Self {
        self.cursor_visible = visible;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_builders() {
        let d = WindowDescriptor::default();
        assert_eq!((d.width, d.height), (1280, 720));
        assert!(d.vsync);
        assert_eq!(d.samples, 1);

        let custom = windowed(800, 600)
            .title("Test")
            .resizable(false)
            .background(0xFF0000)
            .samples(4)
            .vsync(false)
            .cursor_visible(false);
        assert_eq!(custom.title, "Test");
        assert!(!custom.resizable);
        assert_eq!(custom.clear_color, Color::RED);
        assert_eq!(custom.samples, 4);
        assert!(!custom.vsync);
        assert!(!custom.cursor_visible);
    }

    #[test]
    fn samples_are_clamped() {
        assert_eq!(windowed(100, 100).samples(99).samples, 4);
        assert_eq!(windowed(100, 100).samples(0).samples, 1);
    }
}
