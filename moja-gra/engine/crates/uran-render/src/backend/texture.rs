//! Rejestr tekstur GPU: wgrywa obrazy i trzyma bind grupy.
//!
//! Tekstury są wgrywane **leniwie** (przy pierwszym użyciu) i trzymane do
//! końca życia procesu, dopóki ich nie zwolni się jawnie. To dokładnie ten
//! problem, który w prototypie powodował lawinę alokacji: tam bufor
//! wierzchołków powstawał od nowa dla każdej encji w każdej klatce.

use std::collections::HashMap;

use uran_asset::{AssetServer, Image};

use crate::batch::TextureKey;

/// Tekstura wraz z bind grupą (sampler + texture_view).
pub struct GpuTexture {
    pub view: wgpu::TextureView,
    pub bind_group: wgpu::BindGroup,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

impl GpuTexture {
    #[allow(clippy::too_many_arguments)]
    fn create(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        data: &[u8],
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uran Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1, // bez mipmap — w 2D rzadko się przydają, a kosztują pamięć
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let block = format.block_size(None).unwrap_or(4);
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                // `write_texture` wymaga wiersza wyrównanego do 256 bajtów
                bytes_per_row: Some(width * block),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran Texture BindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
            ],
        });
        Self {
            view,
            bind_group,
            size: (width, height),
            format,
        }
    }

    fn from_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        image: &Image,
        srgb: bool,
    ) -> Self {
        // tekstury w przestrzeni sRGB konwertują próbkowanie do liniowego
        // światła, więc tinty i kolory wierzchołków też muszą być liniowe
        let format = if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        };
        Self::create(
            device,
            queue,
            layout,
            sampler,
            image.width,
            image.height,
            format,
            &image.data,
        )
    }

    /// Tekstura jednokanałowa R8 (atlas glifów: kanał alfa = pokrycie).
    /// Tekstura maski: biały RGB + pokrycie w kanale alfa.
    ///
    /// Robimy RGBA, a nie R8, bo shader mnoży pełny próbkowany RGB przez
    /// kolor obiektu. Przy R8 próbka to (pokrycie, 0, 0, 1), więc każdy
    /// glif byłby zabarwiony składową R (czerwony na czarnym).
    pub fn font_atlas(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        size: u32,
        coverage: &[u8],
    ) -> Self {
        let mut rgba = Vec::with_capacity(coverage.len() * 4);
        for &c in coverage {
            rgba.extend_from_slice(&[255, 255, 255, c]);
        }
        Self::create(
            device,
            queue,
            layout,
            sampler,
            size,
            size,
            wgpu::TextureFormat::Rgba8Unorm,
            &rgba,
        )
    }

    /// Tekstura 1x1 (biała albo jednokolorowa).
    pub fn solid_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        rgba: [u8; 4],
    ) -> Self {
        Self::create(
            device,
            queue,
            layout,
            sampler,
            1,
            1,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            &rgba,
        )
    }
}

/// Cache tekstur: klucz -> bind grupa.
///
/// `wgpu::Device` w wgpu 0.19 nie implementuje `Clone`, więc `device`
/// podajemy przy każdym wywołaniu zamiast trzymać w strukturze.
pub struct TextureRegistry {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    textures: HashMap<TextureKey, GpuTexture>,
    /// 1x1 biała — używana, gdy materiał nie ma tekstury.
    white: GpuTexture,
    srgb_surface: bool,
    /// Ile tekstur wgrano w bieżącej klatce (statystyki).
    pub uploaded_this_frame: usize,
    pub total_uploaded: usize,
}

impl TextureRegistry {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: wgpu::BindGroupLayout,
        sampler: wgpu::Sampler,
        srgb_surface: bool,
    ) -> Self {
        let white = GpuTexture::solid_rgba(device, queue, &layout, &sampler, [255, 255, 255, 255]);
        Self {
            layout,
            sampler,
            textures: HashMap::new(),
            white,
            srgb_surface,
            uploaded_this_frame: 0,
            total_uploaded: 0,
        }
    }

    /// Wgrywa teksturę dla klucza, jeśli jeszcze jej nie ma (faza przygotowania,
    /// przed rozpoczęciem render passu). Potem bind grupa jest dostępna
    /// niemutowalnie przez [`TextureRegistry::get_bound`].
    pub fn ensure_bound(
        &mut self,
        device: &wgpu::Device,
        key: TextureKey,
        assets: &AssetServer,
        queue: &wgpu::Queue,
    ) {
        if self.textures.contains_key(&key) {
            return;
        }
        if let TextureKey::Image(id) = key {
            let handle: uran_asset::Handle<Image> = id.into();
            if let Some(image) = assets.image(handle) {
                let texture = GpuTexture::from_image(
                    device,
                    queue,
                    &self.layout,
                    &self.sampler,
                    image,
                    self.srgb_surface,
                );
                self.textures.insert(key, texture);
                self.uploaded_this_frame += 1;
                self.total_uploaded += 1;
            }
        }
    }

    /// Bind grupy dla klucza (faza rysowania, tylko do odczytu).
    /// Gdy klucza nie ma (niewczytany asset), zwraca białą teksturę 1x1.
    pub fn get_bound(&self, key: TextureKey) -> &wgpu::BindGroup {
        match self.textures.get(&key) {
            Some(texture) => &texture.bind_group,
            None => &self.white.bind_group,
        }
    }

    /// Wgrywa (lub aktualizuje) atlas glifów.
    pub fn upload_atlas(
        &mut self,
        device: &wgpu::Device,
        key: TextureKey,
        size: u32,
        data: &[u8],
        queue: &wgpu::Queue,
    ) {
        let existed = self.textures.contains_key(&key);
        let texture =
            GpuTexture::font_atlas(device, queue, &self.layout, &self.sampler, size, data);
        self.textures.insert(key, texture);
        if !existed {
            self.total_uploaded += 1;
        }
        self.uploaded_this_frame += 1;
    }

    /// Czy klucz jest już wgrany (diagnostyka / testy).
    pub fn contains(&self, key: &TextureKey) -> bool {
        self.textures.contains_key(key)
    }

    /// Zwalnia teksturę (np. po usunięciu asseta).
    pub fn release(&mut self, key: &TextureKey) {
        self.textures.remove(key);
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }

    pub fn begin_frame(&mut self) {
        self.uploaded_this_frame = 0;
    }
}
