use uran_math::Color;

#[derive(Debug, Clone)]
pub struct WindowDescriptor {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
    pub clear_color: Color,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: "Uran Engine".to_string(),
            width: 1280,
            height: 720,
            resizable: true,
            clear_color: Color::BLACK, // Domyślnie czarne tło
        }
    }
}

/// Punkt wejścia do konfiguracji okna.
/// Przykład użycia: windowed(800, 600).title("Gra").resizable(true).background(0x000000)
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

    /// Ustawia kolor tła (clear color) na podstawie wartości hex (np. 0x000000)
    pub fn background(mut self, hex_color: u32) -> Self {
        self.clear_color = Color::from_hex(hex_color);
        self
    }
}