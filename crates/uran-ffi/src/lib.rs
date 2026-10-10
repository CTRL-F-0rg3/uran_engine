//! Interfejs FFI — delegowanie części kodu do C# / C++.
//!
//! Silnik jest w Rust, ale fragment logiki można napisać w innym języku i
//! podpiąć przez ABI C. Moduł daje:
//!
//! * typy zgodne z ABI C ([`CVec2`], [`CRect`], [`CColor`]),
//! * kontrakt wtyczki ([`UranPlugin`]) — tabelę wskaźników do funkcji,
//! * hosta ([`PluginHost`]), który bezpiecznie woła te funkcje z Rusta,
//! * gotowe funkcje eksportowane (`uran_vec2_add`, …).
//!
//! Wtyczkę piszesz w C/C++ (nagłówek `uran_ffi.h`) albo w C# (P/Invoke).
//! Wtyczka eksportuje jeden symbol [`PLUGIN_ENTRY`] (`uran_plugin_get`),
//! który zwraca `*const UranPlugin`.

use std::os::raw::{c_char, c_void};

use uran_math::{Color, Rect, Vec2};

/// Wektor 2D w układzie C (zgodny z ABI).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CVec2 {
    pub x: f32,
    pub y: f32,
}

impl CVec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn from_glam(v: Vec2) -> Self {
        Self { x: v.x, y: v.y }
    }

    pub fn to_glam(self) -> Vec2 {
        Vec2::new(self.x, self.y)
    }
}

/// Prostokąt w układzie C (`x, y` = róg min, `w, h` = rozmiar).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl CRect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn from_glam(r: Rect) -> Self {
        Self {
            x: r.min.x,
            y: r.min.y,
            w: r.width(),
            h: r.height(),
        }
    }

    pub fn to_glam(self) -> Rect {
        Rect::from_xywh(self.x, self.y, self.w, self.h)
    }
}

/// Kolor RGBA w układzie C (kanały 0..1).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl CColor {
    pub fn from_glam(c: Color) -> Self {
        Self {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a,
        }
    }

    pub fn to_glam(self) -> Color {
        Color::rgba(self.r, self.g, self.b, self.a)
    }
}

/// Kontrakt wtyczki — tabela wskaźników do funkcji napisanych w C#/C++.
///
/// Wtyczka eksportuje jeden symbol [`PLUGIN_ENTRY`], który zwraca
/// `*const UranPlugin`.
#[repr(C)]
pub struct UranPlugin {
    /// Nazwa wtyczki (UTF-8, zakończona NUL, statyczna).
    pub name: unsafe extern "C" fn() -> *const c_char,
    /// Tworzy stan wtyczki; zwraca uchwyt (może być null).
    pub create: unsafe extern "C" fn() -> *mut c_void,
    /// Niszczy stan wtyczki.
    pub destroy: unsafe extern "C" fn(state: *mut c_void),
    /// Wywoływane raz przy starcie.
    pub init: unsafe extern "C" fn(state: *mut c_void),
    /// Wywoływane co klatkę (`dt` w sekundach).
    pub update: unsafe extern "C" fn(state: *mut c_void, dt: f32),
}

/// Symbol, który wtyczka musi eksportować.
pub const PLUGIN_ENTRY: &str = "uran_plugin_get";

/// Host wtyczki po stronie Rusta — bezpiecznie woła funkcje z tabeli.
pub struct PluginHost {
    vtable: &'static UranPlugin,
    state: *mut c_void,
}

impl PluginHost {
    /// Tworzy hosta z tabeli `UranPlugin` i wywołuje `create`.
    pub fn new(vtable: &'static UranPlugin) -> Self {
        let state = unsafe { (vtable.create)() };
        Self { vtable, state }
    }

    /// Nazwa wtyczki (pusta, gdy wskaźnik jest zerowy lub nie-UTF-8).
    pub fn name(&self) -> &str {
        let ptr = unsafe { (self.vtable.name)() };
        if ptr.is_null() {
            return "";
        }
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_str()
            .unwrap_or("")
    }

    /// Wywołuje `init` wtyczki.
    pub fn init(&self) {
        unsafe { (self.vtable.init)(self.state) };
    }

    /// Wywołuje `update` wtyczki.
    pub fn update(&self, dt: f32) {
        unsafe { (self.vtable.update)(self.state, dt) };
    }

    /// Surowy stan wtyczki (dla zaawansowanych).
    pub fn state(&self) -> *mut c_void {
        self.state
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        unsafe { (self.vtable.destroy)(self.state) };
    }
}

// --- Eksportowane funkcje matematyczne (przykład powierzchni FFI) ---

#[no_mangle]
pub extern "C" fn uran_vec2_add(a: CVec2, b: CVec2) -> CVec2 {
    CVec2::from_glam(a.to_glam() + b.to_glam())
}

#[no_mangle]
pub extern "C" fn uran_vec2_length(v: CVec2) -> f32 {
    v.to_glam().length()
}

#[no_mangle]
pub extern "C" fn uran_vec2_normalized(v: CVec2) -> CVec2 {
    CVec2::from_glam(v.to_glam().normalize_or_zero())
}

#[no_mangle]
pub extern "C" fn uran_rect_contains(r: CRect, p: CVec2) -> i32 {
    r.to_glam().contains(p.to_glam()) as i32
}

#[no_mangle]
pub extern "C" fn uran_color_from_hex(hex: u32) -> CColor {
    CColor::from_glam(Color::from_hex(hex))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI32, Ordering};

    static COUNTER: AtomicI32 = AtomicI32::new(0);

    unsafe extern "C" fn pname() -> *const c_char {
        b"demo-plugin\0".as_ptr() as *const c_char
    }
    unsafe extern "C" fn pcreate() -> *mut c_void {
        &COUNTER as *const AtomicI32 as *mut c_void
    }
    unsafe extern "C" fn pdestroy(_state: *mut c_void) {}
    unsafe extern "C" fn pinit(state: *mut c_void) {
        let c = &*(state as *const AtomicI32);
        c.store(10, Ordering::SeqCst);
    }
    unsafe extern "C" fn pupdate(state: *mut c_void, dt: f32) {
        let c = &*(state as *const AtomicI32);
        c.fetch_add(dt as i32, Ordering::SeqCst);
    }

    static PLUGIN: UranPlugin = UranPlugin {
        name: pname,
        create: pcreate,
        destroy: pdestroy,
        init: pinit,
        update: pupdate,
    };

    /// Host wywołuje kod „obcy" (init + update) przez tabelę wskaźników.
    #[test]
    fn plugin_host_calls_foreign_code() {
        COUNTER.store(0, Ordering::SeqCst);
        let host = PluginHost::new(&PLUGIN);
        assert_eq!(host.name(), "demo-plugin");
        host.init();
        host.update(3.0);
        assert_eq!(COUNTER.load(Ordering::SeqCst), 13);
    }

    /// Eksportowane funkcje liczą poprawnie (można je wołać z C#/C++).
    #[test]
    fn exported_math_functions_work() {
        assert_eq!(
            uran_vec2_add(CVec2::new(1.0, 2.0), CVec2::new(3.0, 4.0)),
            CVec2::new(4.0, 6.0)
        );
        assert!((uran_vec2_length(CVec2::new(3.0, 4.0)) - 5.0).abs() < 1e-5);
        assert_eq!(
            uran_rect_contains(CRect::new(0.0, 0.0, 10.0, 10.0), CVec2::new(5.0, 5.0)),
            1
        );
        assert_eq!(
            uran_rect_contains(CRect::new(0.0, 0.0, 10.0, 10.0), CVec2::new(11.0, 5.0)),
            0
        );
        assert_eq!(uran_color_from_hex(0xFF0000), CColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
    }
}


