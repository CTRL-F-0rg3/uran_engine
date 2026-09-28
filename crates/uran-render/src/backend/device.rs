//! Kontekst GPU: instancja, adapter, urządzenie i konfiguracja powierzchni.

use std::fmt;
use std::sync::Arc;

use winit::dpi::PhysicalSize;
use winit::window::Window;

/// Błąd inicjalizacji renderera.
#[derive(Debug)]
pub enum RenderError {
    Surface(wgpu::CreateSurfaceError),
    Adapter(String),
    Device(wgpu::RequestDeviceError),
    NoSurfaceFormat,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Surface(e) => write!(f, "nie udało się utworzyć powierzchni: {e}"),
            Self::Adapter(name) => write!(f, "brak adaptera GPU: {name}"),
            Self::Device(e) => write!(f, "nie udało się utworzyć urządzenia: {e}"),
            Self::NoSurfaceFormat => write!(f, "adapter nie udostępnia żadnego formatu powierzchni"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<wgpu::CreateSurfaceError> for RenderError {
    fn from(e: wgpu::CreateSurfaceError) -> Self {
        Self::Surface(e)
    }
}

impl From<wgpu::RequestDeviceError> for RenderError {
    fn from(e: wgpu::RequestDeviceError) -> Self {
        Self::Device(e)
    }
}

/// Rozmiar push constants potrzebny przez potok siatek
/// (mat3x3<f32> = 48 bajtów + vec4 = 64).
pub const PUSH_CONSTANT_SIZE: u32 = 64;

/// Wszystkie zasoby GPU wymagane do renderowania 2D.
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: PhysicalSize<u32>,
    /// Informacja o adapterze (do logów startowych).
    pub info: wgpu::AdapterInfo,
}

impl GpuContext {
    pub async fn new(
        window: Arc<Window>,
        vsync: bool,
        _samples: u32,
    ) -> Result<Self, RenderError> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        let surface = instance.create_surface(window)?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| RenderError::Adapter("żaden adapter nie pasuje do tej powierzchni".into()))?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("Uran Device"),
                    required_features: wgpu::Features::PUSH_CONSTANTS,
                    required_limits: wgpu::Limits {
                        max_push_constant_size: PUSH_CONSTANT_SIZE,
                        ..wgpu::Limits::downlevel_defaults()
                    },
                },
                None,
            )
            .await?;

        let caps = surface.get_capabilities(&adapter);
        // sRGB daje poprawne mieszanie alfa; format bez sufiksu to plan B
        let format = caps
            .formats
            .iter()
            .copied()
            .find(wgpu::TextureFormat::is_srgb)
            .unwrap_or(caps.formats[0]);

        let present_mode = if vsync {
            wgpu::PresentMode::AutoVsync
        } else {
            match caps.present_modes.contains(&wgpu::PresentMode::AutoNoVsync) {
                true => wgpu::PresentMode::AutoNoVsync,
                false => wgpu::PresentMode::Immediate,
            }
        };

        let config = wgpu::SurfaceConfiguration {
            // COPY_SRC pozwala robić zrzuty ekranu (czytanie klatki z GPU)
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        Ok(Self {
            info: adapter.get_info(),
            instance,
            surface,
            adapter,
            device,
            queue,
            config,
            size,
        })
    }

    /// Rozmiar okna w fizycznych pikselach.
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Czy powierzchnia jest formatu sRGB (wpływa na format tekstur).
    pub fn is_srgb_surface(&self) -> bool {
        self.config.format.is_srgb()
    }

    /// Ponowna konfiguracja po zmianie rozmiaru okna.
    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.size = size;
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Limit rozmiaru tekstury (2D).
    pub fn max_texture_size(&self) -> u32 {
        self.device.limits().max_texture_dimension_2d
    }
}
