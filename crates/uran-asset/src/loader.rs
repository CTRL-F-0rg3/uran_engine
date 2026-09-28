//! Ładowanie i dekodowanie assetów: obrazy (PNG) i czcionki (TTF/OTF).

use std::fmt;
use std::path::Path;

/// Błąd podczas wczytywania assetu.
#[derive(Debug)]
pub enum AssetError {
    Io(std::io::Error),
    Decode(String),
    Invalid(String),
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "błąd I/O: {e}"),
            Self::Decode(e) => write!(f, "błąd dekodowania: {e}"),
            Self::Invalid(e) => write!(f, "nieprawidłowy asset: {e}"),
        }
    }
}

impl std::error::Error for AssetError {}

impl From<std::io::Error> for AssetError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Obraz RGBA8 w pamięci CPU (4 bajty na piksel, wiersz po wierszu od góry).
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Image {
    /// Pusty obraz wypełniony zerami (czyli **przezroczysty** — wygodny
    /// punkt wyjścia dla grafik proceduralnych).
    pub fn new(width: u32, height: u32) -> Self {
        let len = (width as usize) * (height as usize) * 4;
        Self { width, height, data: vec![0; len] }
    }

    /// Buduje obraz z surowych danych RGBA8.
    pub fn from_rgba(width: u32, height: u32, data: Vec<u8>) -> Result<Self, AssetError> {
        let expected = (width as usize) * (height as usize) * 4;
        if data.len() != expected {
            return Err(AssetError::Invalid(format!(
                "oczekiwano {expected} bajtów dla {width}x{height}, a jest {}",
                data.len()
            )));
        }
        Ok(Self { width, height, data })
    }

    /// Pusty obraz wypełniony jednym kolorem.
    pub fn filled(width: u32, height: u32, color: [u8; 4]) -> Self {
        let mut img = Self::new(width, height);
        for px in img.pixels_mut() {
            *px = color;
        }
        img
    }

    /// Wczytuje obraz PNG z dysku.
    pub fn load_png(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        let decoded = image::open(path.as_ref())
            .map_err(|e| AssetError::Decode(format!("{}: {e}", path.as_ref().display())))?;
        Self::from_dynamic(decoded)
    }

    /// Zapisuje obraz do pliku PNG (przydatne do zrzutów ekranu i testów).
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), AssetError> {
        let buffer = image::RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or_else(|| AssetError::Invalid("uszkodzony bufor obrazu".into()))?;
        buffer
            .save(path.as_ref())
            .map_err(|e| AssetError::Decode(format!("{}: {e}", path.as_ref().display())))
    }

    fn from_dynamic(img: image::DynamicImage) -> Result<Self, AssetError> {
        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();
        Ok(Self { width, height, data: rgba.into_raw() })
    }

    pub const fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Zwraca piksel (wychodzi poza obraz → `[0, 0, 0, 0]`).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.width || y >= self.height {
            return [0, 0, 0, 0];
        }
        let i = ((y * self.width + x) * 4) as usize;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let i = ((y * self.width + x) * 4) as usize;
        self.data[i..i + 4].copy_from_slice(&color);
    }

    pub fn pixels_mut(&mut self) -> impl Iterator<Item = &mut [u8; 4]> {
        self.data
            .chunks_exact_mut(4)
            .map(|c| <&mut [u8; 4]>::try_from(c).unwrap())
    }

    /// Przycina prostokąt (używane np. do wycinania sprite'ów z atlasu).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Result<Self, AssetError> {
        if x + w > self.width || y + h > self.height {
            return Err(AssetError::Invalid("crop wychodzi poza obraz".into()));
        }
        let mut out = Self::new(w, h);
        for row in 0..h {
            let src_start = (((y + row) * self.width) + x) as usize * 4;
            let dst_start = (row * w) as usize * 4;
            out.data[dst_start..dst_start + w as usize * 4]
                .copy_from_slice(&self.data[src_start..src_start + w as usize * 4]);
        }
        Ok(out)
    }

    /// Skaluje metodą nearest-neighbour (styl pixel-art).
    pub fn scale_nearest(&self, width: u32, height: u32) -> Self {
        let mut out = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let sx = (x as u64 * self.width as u64 / width.max(1) as u64) as u32;
                let sy = (y as u64 * self.height as u64 / height.max(1) as u64) as u32;
                out.set_pixel(x, y, self.pixel(sx, sy));
            }
        }
        out
    }

    /// Rysuje wypełniony prostokąt (współrzędne od lewego górnego rogu).
    pub fn fill_rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: [u8; 4]) {
        for py in y..y + h {
            for px in x..x + w {
                if px >= 0 && py >= 0 {
                    self.set_pixel(px as u32, py as u32, color);
                }
            }
        }
    }

    /// Rysuje wypełniony okrąg (środek w pikselach, promień w pikselach).
    pub fn fill_circle(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
        let r2 = (radius * radius) as i64;
        for py in (cy - radius)..=(cy + radius) {
            for px in (cx - radius)..=(cx + radius) {
                let dx = (px - cx) as i64;
                let dy = (py - cy) as i64;
                if dx * dx + dy * dy <= r2 && px >= 0 && py >= 0 {
                    self.set_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

/// Surowe bajty czcionki (TTF/OTF) — interpretuje je dopiero renderer
/// (font atlas powstaje dopiero, gdy ktoś potrzebuje narysować tekst).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontData {
    pub data: Vec<u8>,
}

impl FontData {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Wczytuje czcionkę z dysku.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        Ok(Self { data: std::fs::read(path.as_ref())? })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_basics() {
        let mut img = Image::new(4, 3);
        assert_eq!(img.size(), (4, 3));
        assert_eq!(img.data.len(), 4 * 3 * 4);
        img.set_pixel(1, 1, [255, 0, 0, 255]);
        assert_eq!(img.pixel(1, 1), [255, 0, 0, 255]);
        assert_eq!(img.pixel(9, 9), [0, 0, 0, 0]); // poza obrazem
        img.set_pixel(9, 9, [1, 2, 3, 4]); // nie panikuje
    }

    #[test]
    fn from_rgba_validates_length() {
        assert!(Image::from_rgba(2, 2, vec![0; 16]).is_ok());
        assert!(Image::from_rgba(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn crop_and_scale() {
        let mut img = Image::new(4, 4);
        img.fill_rect(1, 1, 2, 2, [10, 20, 30, 255]);
        let cropped = img.crop(1, 1, 2, 2).unwrap();
        assert_eq!(cropped.size(), (2, 2));
        assert_eq!(cropped.pixel(0, 0), [10, 20, 30, 255]);
        assert_eq!(cropped.pixel(1, 1), [10, 20, 30, 255]);
        assert!(img.crop(3, 3, 4, 4).is_err());

        let scaled = img.scale_nearest(8, 8);
        assert_eq!(scaled.size(), (8, 8));
        assert_eq!(scaled.pixel(2, 2), [10, 20, 30, 255]);
    }

    #[test]
    fn shapes() {
        let mut img = Image::new(8, 8);
        img.fill_rect(0, 0, 2, 2, [255, 255, 255, 255]);
        assert_eq!(img.pixel(1, 1), [255, 255, 255, 255]);
        // domyślnie obraz jest PRZEZROCZYSTY (alfa 0)
        assert_eq!(img.pixel(4, 4), [0, 0, 0, 0]);

        let mut c = Image::new(9, 9);
        c.fill_circle(4, 4, 3, [255, 0, 0, 255]);
        assert_eq!(c.pixel(4, 4), [255, 0, 0, 255]);
        assert_eq!(c.pixel(0, 0), [0, 0, 0, 0]); // poza okręgiem
    }

    #[test]
    fn filled_is_opaque() {
        let img = Image::filled(2, 2, [1, 2, 3, 4]);
        assert_eq!(img.pixel(0, 0), [1, 2, 3, 4]);
        assert_eq!(img.pixel(1, 1), [1, 2, 3, 4]);
    }

    #[test]
    fn png_roundtrip() {
        let mut img = Image::new(3, 3);
        img.fill_circle(1, 1, 1, [1, 2, 3, 255]);
        let path = std::env::temp_dir().join("uran_test_image.png");
        img.save_png(&path).unwrap();
        let loaded = Image::load_png(&path).unwrap();
        assert_eq!(loaded.size(), (3, 3));
        assert_eq!(loaded.pixel(1, 1), [1, 2, 3, 255]);
        let _ = std::fs::remove_file(path);
    }
}
