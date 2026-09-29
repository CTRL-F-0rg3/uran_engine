//! Punkt styku renderera 2D ze sceną 3D.
//!
//! Trajektoria: `uran-engine` (okno, pętla, `Ctx`) wspóldzieli JEDEN
//! `Device`/`Queue`/`Surface` między rendererem 2D a rendererem 3D.
//! Dwa renderery w dwóch crate'ach, dwa zestawy potoków, dwa shadery —
//! ale jedno urządzenie i jedna klatka.
//!
//! Dlaczego tak, a nie osobny `Surface`: wgpu pozwala mieć tylko jeden
//! `Surface` na okno. Drugi `Surface` na to samo okno albo się nie da
//! utworzyć, albo ostatni `present()` wygrywa i klatki znikają. Osobne
//! okno dla 3D z kolei niepotrzebnie komplikuje pętlę zdarzeń.

use crate::backend::device::GpuContext;

/// Cel rysowania przekazywany scenie 3D.
///
/// Renderer 2D posiada `SurfaceTexture` i nie udostępnia go (jego typ jest
/// wewnętrzny dla wgpu), więc przekazujemy to, czego scena realnie potrzebuje:
/// widok koloru, opcjonalny cel MSAA i konfigurację próbkowania.
pub struct Scene3dTarget<'a> {
    /// Widok tekstury kolorowej (ta sama, do której pisze potem 2D).
    pub color: &'a wgpu::TextureView,
    /// Widok multisamplingowy, gdy `samples > 1` — tu się rozwiązuje.
    pub resolve: Option<&'a wgpu::TextureView>,
    /// Format docelowy (potok 3D musi się zgadzać z formatem powierzchni).
    pub format: wgpu::TextureFormat,
    /// Ile próbek na piksel (1 = brak MSAA).
    pub samples: u32,
    /// Kontekst GPU — do kolejki (np. wgranie geometrii) i limitów.
    pub gpu: &'a GpuContext,
}

/// Scena 3D rysowana we własnym render passie.
///
/// Implementację dostarcza `uran-render3d`; tu jest tylko kontrakt.
pub trait Scene3d {
    /// Rysuje scenę 3D do `target`, w tym własny bufor głębokości.
    ///
    /// Wywoływane RAZ na klatkę, PRZED passem 2D.
    fn draw(&mut self, target: &Scene3dTarget<'_>);

    /// Zgłasza zmianę rozmiaru okna — scena sama odtwarza depth buffer.
    fn resize(&mut self, _width: u32, _height: u32) {}

    /// Czy scena jest aktywna. Gdy `false`, 2D czyści kadr sam i pomija
    /// `draw` (przydatne, gdy nie ma świata 3D).
    fn is_active(&self) -> bool {
        true
    }
}