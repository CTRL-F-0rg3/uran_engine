/// Kolor RGBA, kanały zmiennoprzecinkowe w zakresie `0..=1`.
///
/// Wartości trzymane są w **przestrzeni sRGB** (tak jak `Color::from_hex`).
/// Renderer konwertuje je do liniowego światła (`Color::to_linear`) zanim
/// trafią na GPU, a powierzchnia w formatach `*Srgb` zamienia je z powrotem —
/// dzięki temu mieszanie alfy wygląda poprawnie.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    pub const RED: Self = Self::rgb(1.0, 0.0, 0.0);
    pub const GREEN: Self = Self::rgb(0.0, 1.0, 0.0);
    pub const BLUE: Self = Self::rgb(0.0, 0.0, 1.0);
    pub const YELLOW: Self = Self::rgb(1.0, 1.0, 0.0);
    pub const CYAN: Self = Self::rgb(0.0, 1.0, 1.0);
    pub const MAGENTA: Self = Self::rgb(1.0, 0.0, 1.0);
    pub const ORANGE: Self = Self::rgb(1.0, 0.55, 0.0);
    pub const GRAY: Self = Self::rgb(0.5, 0.5, 0.5);
    pub const NONE: Self = Self::TRANSPARENT;

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// Kanały 0..=255 (np. z katalogu palet).
    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }

    pub const fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self::rgba(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            a as f32 / 255.0,
        )
    }

    /// Tworzy kolor z wartości hex (np. 0x000000 dla czarnego, 0xFF0000 dla czerwonego).
    pub const fn from_hex(hex: u32) -> Self {
        Self::rgba8(
            ((hex >> 16) & 0xFF) as u8,
            ((hex >> 8) & 0xFF) as u8,
            (hex & 0xFF) as u8,
            0xFF,
        )
    }

    /// Jak [`Color::from_hex`], ale z alpha w najniższym bajcie (0xRRGGBBAA).
    pub const fn from_hex_rgba(hex: u32) -> Self {
        Self::rgba8(
            ((hex >> 24) & 0xFF) as u8,
            ((hex >> 16) & 0xFF) as u8,
            ((hex >> 8) & 0xFF) as u8,
            (hex & 0xFF) as u8,
        )
    }

    pub const fn to_rgba8(self) -> [u8; 4] {
        [
            (self.r.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.g.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.b.clamp(0.0, 1.0) * 255.0).round() as u8,
            (self.a.clamp(0.0, 1.0) * 255.0).round() as u8,
        ]
    }

    pub const fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Self { a: alpha, ..self }
    }

    /// Przyciemnienie (mnożnik 0..=1 na kanały RGB).
    pub fn multiply(self, factor: f32) -> Self {
        Self {
            r: self.r * factor,
            g: self.g * factor,
            b: self.b * factor,
            a: self.a,
        }
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// sRGB -> liniowe światło (to, czego oczekuje GPU).
    pub fn to_linear(self) -> Self {
        fn c(v: f32) -> f32 {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        Self {
            r: c(self.r),
            g: c(self.g),
            b: c(self.b),
            a: self.a,
        }
    }

    /// Liniowe światło -> sRGB.
    pub fn from_linear(self) -> Self {
        fn c(v: f32) -> f32 {
            if v <= 0.0031308 {
                v * 12.92
            } else {
                1.055 * v.powf(1.0 / 2.4) - 0.055
            }
        }
        Self {
            r: c(self.r),
            g: c(self.g),
            b: c(self.b),
            a: self.a,
        }
    }

    /// Czarno-biała wersja koloru (przydatne do tintowania sprite'ów).
    pub fn grayscale(self) -> Self {
        let luma = 0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b;
        Self::rgba(luma, luma, luma, self.a)
    }
}

impl From<[u8; 4]> for Color {
    fn from(v: [u8; 4]) -> Self {
        Self::rgba8(v[0], v[1], v[2], v[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        assert_eq!(Color::from_hex(0xFF0000), Color::rgb(1.0, 0.0, 0.0));
        assert_eq!(Color::from_hex(0x00FF00), Color::rgb(0.0, 1.0, 0.0));
        assert_eq!(Color::from_hex(0x0000FF), Color::rgb(0.0, 0.0, 1.0));
        assert_eq!(Color::from_hex(0xFFFFFF).a, 1.0);
        assert_eq!(
            Color::from_hex(0x123456).to_rgba8(),
            [0x12, 0x34, 0x56, 0xFF]
        );
    }

    #[test]
    fn hex_with_alpha() {
        let c = Color::from_hex_rgba(0xFF000080);
        assert_eq!(c.r, 1.0);
        assert!((c.a - 0.502).abs() < 0.01);
    }

    #[test]
    fn linear_conversion_roundtrip() {
        for c in [
            Color::WHITE,
            Color::BLACK,
            Color::RED,
            Color::from_hex(0x3366CC),
        ] {
            let back = c.to_linear().from_linear();
            assert!((back.r - c.r).abs() < 1e-5);
            assert!((back.g - c.g).abs() < 1e-5);
            assert!((back.b - c.b).abs() < 1e-5);
            assert_eq!(back.a, c.a);
        }
        assert_eq!(Color::WHITE.to_linear().r, 1.0);
        assert!((Color::from_hex(0x808080).to_linear().r - 0.2158).abs() < 0.001);
    }

    #[test]
    fn lerp_and_alpha() {
        let a = Color::BLACK.with_alpha(0.25);
        assert_eq!(a.a, 0.25);
        // lerp interpoluje również kanał alpha (0.25 -> 1.0).
        assert_eq!(a.lerp(Color::WHITE, 0.5), Color::rgba(0.5, 0.5, 0.5, 0.625));
        assert_eq!(a.lerp(Color::WHITE, 2.0), Color::WHITE);
        assert_eq!(Color::WHITE.multiply(0.0), Color::BLACK);
    }
}
