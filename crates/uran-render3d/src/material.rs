//! Materiały PBR na GPU: tekstury, parametry i ich wiązanie z shaderem.
//!
//! ## Po co to istnieje
//!
//! Parser `.mtl` (moduł `import`) umie przeczytać `map_Kd`, `map_Bump`
//! i `map_Pr`/`map_Pm`, ale wczytany plik to dopiero opis. Ten moduł
//! zamienia go w zasoby, które wgpu potrafi związać z potokiem.
//!
//! ## Trzy tekstury, nie jedna
//!
//! Blender eksportuje „wypalony" materiał jako trzy mapy:
//!
//! | Tekstura | Kanały | Format wgpu | Dlaczego nie sRGB |
//! |----------|--------|-------------|-------------------|
//! | Base Color | RGB | `Rgba8UnormSrgb` | kolor — musi być zdekodowany przez GPU |
//! | Normal | RGB | `Rgba8Unorm` | wektor, nie kolor |
//! | ORM | R=AO, G=rough, B=metal | `Rgba8Unorm` | wartości fizyczne |
//!
//! Wpisanie mapy normalnych w formacie sRGB jest **klasycznym błędem**:
//! GPU zdekodowałby ją jak kolor, wygładził nierówności i normalne
//! pojechałyby w złą stronę. Dlatego format dobieramy z roli tekstury,
//! a nie z roli pliku.

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::import::Material;

/// Parametry materiału przekazywane do shadera (4 × `vec4` = 64 B).
///
/// Wszystko w `vec4`, bo WGSL wyrównuje `vec3` do 16 B — struktura
/// z samymi `f32` i `vec3` dałaby inny rozmiar po obu stronach i
/// walidacja wgpu odrzuciłaby bufor.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct MaterialUniform {
    /// `x` = chropowatość bazowa, `y` = metaliczność bazowa,
    /// `z` = mnożnik UV, `w` = siła normal mappingu.
    pub params: [f32; 4],
    /// `x` = albedo mapa?, `y` = normal mapa?, `z` = ORM mapa?,
    /// `w` = odwrócenie skaliowej osi V (V do góry / do dołu).
    pub flags: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<MaterialUniform>() == 32);

/// Materiał gotowy do rysowania.
pub struct GpuMaterial {
    bind_group: wgpu::BindGroup,
    uniform: wgpu::Buffer,
    // Trzymamy tekstury przy sobie: wgpu nie ma współdzielenia
    // zasobów, więc zwolnienie `wgpu::Texture` gwarantowałoby, że
    // bind group wskaże na nic.
    _albedo: wgpu::Texture,
    _normal: wgpu::Texture,
    _orm: wgpu::Texture,
}

/// Zestaw domyślnych tekstur dla materiałów bez plików.
///
/// Trzymamy je w [`MaterialBank`], bo wgpu nie pozwala współdzielić
/// jednej tekstury między bind grupy — każdy materiał dostaje swój
/// bind group, ale fizycznie dzieli te same obrazy.
pub struct MaterialBank {
    // `TextureView` NIE implementuje `Clone` w wgpu 0.19, a ten sam
    // widok musi trafić do bind group każdego materiału. Dlatego
    // trzymamy go za `Arc` zamiast kopiować.
    pub white: std::sync::Arc<wgpu::TextureView>,
    pub flat_normal: std::sync::Arc<wgpu::TextureView>,
    pub orm_default: std::sync::Arc<wgpu::TextureView>,
    pub sampler: wgpu::Sampler,
    pub layout: std::sync::Arc<wgpu::BindGroupLayout>,
    /// Materiały w kolejności ich identyfikatorów.
    pub materials: Vec<GpuMaterial>,
    /// Identyfikatory materiałów, których nie wolno usunąć.
    pub fallback: Option<MaterialId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialId(pub u32);

/// Tekstura 1×1 w zadanym kolorze (uzywana jako "brak tekstury").
fn solid(device: &wgpu::Device, queue: &wgpu::Queue, rgba: [u8; 4], srgb: bool) -> wgpu::Texture {
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Uran 3D tekstura domyslna"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &rgba,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    tex
}

/// Wczytuje PNG i tworzy z niego teksturę.
///
/// Zwraca `None`, gdy pliku nie ma lub jest uszkodzony — brakująca
/// tekstura nie może wywrócić importu modelu.
fn load_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &std::path::Path,
    srgb: bool,
) -> Option<wgpu::Texture> {
    let img = uran_asset::Image::load_png(path).ok()?;
    let (w, h) = (img.width, img.height);
    if w == 0 || h == 0 {
        return None;
    }
    let format = if srgb {
        wgpu::TextureFormat::Rgba8UnormSrgb
    } else {
        wgpu::TextureFormat::Rgba8Unorm
    };
    let size = wgpu::Extent3d {
        width: w,
        height: h,
        depth_or_array_layers: 1,
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Uran 3D tekstura materiału"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &img.data,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        size,
    );
    Some(tex)
}

impl MaterialBank {
    /// Tworzy bank z zestawem tekstur domyślnych i materiałem zapasowym.
    ///
    /// Materiał zapasowy MUSI istnieć od razu: `DrawCmd` wskazuje
    /// materiał, a `Option<MaterialId>` w komendzie zamieniałby każde
    /// rysowanie w gałąź. Lepiej jeden wspólny materiał „biały bez map".
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran 3D Layout Materiału"),
            entries: &[
                // 0: albedo (kolor → sRGB, GPU zdekoduje do liniowego)
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 1: normalna (wektor → liniowy!)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // 2: ORM (AO / roughness / metaliczność → liniowy)
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran 3D Sampler"),
            // Powtarzamy poza zakresem 0..1 — modele z Blendera często
            // mają UV wysuwające poza kwadrat, a bez tego krawędź
            // tekstury ciemnieje w miejscu, gdzie zaczyna się tiling.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Zastępcze: biały albedo, płaska normalna (0,5,0,5,1) i ORM
        // oznaczające „brak AO, chropowatość 1 (matowe), metal 0".
        let white = solid(device, queue, [255, 255, 255, 255], true);
        let flat_normal = solid(device, queue, [128, 128, 255, 255], false);
        let orm_default = solid(device, queue, [255, 255, 0, 255], false);

        let mut bank = Self {
            white: std::sync::Arc::new(white.create_view(&Default::default())),
            flat_normal: std::sync::Arc::new(flat_normal.create_view(&Default::default())),
            orm_default: std::sync::Arc::new(orm_default.create_view(&Default::default())),
            sampler,
            layout: std::sync::Arc::new(layout),
            materials: Vec::new(),
            fallback: None,
        };
        // Materiał 0 zawsze istnieje i zawsze jest biały bez map
        let id = bank.add(device, queue, &Material::fallback("default"));
        bank.fallback = Some(id);
        bank
    }

    /// Dodaje materiał i zwraca jego identyfikator.
    pub fn add(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, m: &Material) -> MaterialId {
        // Wszystkie ścieżki rozpatrujemy w stosunku do katalogu `.mtl`
        // (import już je złączył), a brak pliku nie jest błędem.
        let albedo_tex = m
            .albedo_map
            .as_deref()
            .and_then(|p| load_texture(device, queue, p, true));
        let normal_tex = m
            .normal_map
            .as_deref()
            .and_then(|p| load_texture(device, queue, p, false));
        // ORM: Blender w `.mtl` zapisuje je osobno (`map_Pr`, `map_Pm`),
        // a w glTF jako jedną paczkę. Bierzemy `albedo_map` tylko gdy
        // wskazuje na plik ORM — inaczej zostawiamy wartości stałe.
        let orm_tex: Option<wgpu::Texture> = None;

        let albedo_view = albedo_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.white));
        let normal_view = normal_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.flat_normal));
        let orm_view = orm_tex
            .as_ref()
            .map(|t| std::sync::Arc::new(t.create_view(&Default::default())))
            .unwrap_or_else(|| std::sync::Arc::clone(&self.orm_default));

        let uniform = MaterialUniform {
            params: [
                m.roughness.clamp(0.02, 1.0),
                m.metallic.clamp(0.0, 1.0),
                1.0, // mnożnik UV
                if normal_tex.is_some() { 1.0 } else { 0.0 },
            ],
            flags: [
                if albedo_tex.is_some() { 1.0 } else { 0.0 },
                if normal_tex.is_some() { 1.0 } else { 0.0 },
                if orm_tex.is_some() { 1.0 } else { 0.0 },
                // V do góry: OpenGL/Blender ma V rosnące w górę, a
                // wgpu zapisuje tekstury od góry. Bez odwrócenia obraz
                // byłby do góry nogami.
                -1.0,
            ],
        };
        let ubuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uran 3D Uniform Materiału"),
            contents: bytemuck::bytes_of(&uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran 3D Bind Materiału"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&normal_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&orm_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: ubuf.as_entire_binding(),
                },
            ],
        });

        let id = MaterialId(self.materials.len() as u32);
        self.materials.push(GpuMaterial {
            bind_group,
            uniform: ubuf,
            // Tekstury trzymamy przy sobie: wgpu nie współdzieli
            // zasobów, więc ich zwolnienie zostawiłoby bind group
            // wskazujący na nic.
            _albedo: albedo_tex.unwrap_or_else(|| self.white_placeholder(device, queue)),
            _normal: normal_tex.unwrap_or_else(|| self.normal_placeholder(device, queue)),
            _orm: orm_tex.unwrap_or_else(|| self.orm_placeholder(device, queue)),
        });
        id
    }

    fn white_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [255, 255, 255, 255], true)
    }
    fn normal_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [128, 128, 255, 255], false)
    }
    fn orm_placeholder(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
        solid(device, queue, [255, 255, 0, 255], false)
    }

    /// Bind grupy materiału, albo `None` gdy identyfikator jest zły.
    pub fn bind_group(&self, id: MaterialId) -> Option<&wgpu::BindGroup> {
        self.materials.get(id.0 as usize).map(|m| &m.bind_group)
    }
}
