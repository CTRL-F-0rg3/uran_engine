//! Post-processing: AA, bloom, tonemapping, aberracja, winieta, ziarno.
//!
//! Celowo ODSEPAROWANY moduł zamiast rozbudowy `Renderer3d`. Trzyma
//! własne tekstury, potoki i bind grupy, a `Renderer3d` rozmawia z nim
//! tylko przez trzy metody: `hdr_view`, `composite` i `invalidate`.
//!
//! Dzięki temu reszta renderera 3D pozostaje nietknięta — warunek,
//! żeby dało się to wdrożyć małymi krokami.

use uran_render::Scene3dTarget;

/// Parametry post-processingu (UKŁAD ZGODNY Z `Params` W `post.wgsl`).
///
/// 3 * `vec4` = 48 B. Wszystko `vec4`, bo WGSL wyrównuje `vec3` do 16 B.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PostParams {
    /// `x` = ekspozycja, `y` = siła bloom, `z` = winieta, `w` = aberracja.
    pub grade: [f32; 4],
    /// `x` = próg bloom, `y` = ziarno, `z` = saturacja, `w` = kontrast.
    pub fx: [f32; 4],
    /// `x,y` = rozmiar kadru, `z` = czas, `w` = siła AA.
    pub screen: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<PostParams>() == 48);

/// Ustawienia wyglądu, które gra może zmieniać w locie.
#[derive(Debug, Clone, Copy)]
pub struct PostSettings {
    pub exposure: f32,
    pub bloom: f32,
    pub bloom_threshold: f32,
    pub vignette: f32,
    pub chromatic: f32,
    pub grain: f32,
    pub saturation: f32,
    pub contrast: f32,
    /// Siła antyaliasingu krawędzi 0..1.
    pub antialias: f32,
}

impl Default for PostSettings {
    fn default() -> Self {
        Self {
            // scena mnoży się już przez 1.35 w shaderze, więc tu zostawiamy
            // ekspozycję 1.0 i rozjaśniamy dopiero przed tonemappingiem
            exposure: 1.0,
            // bloom SUBTELNY — przy 0.6+ świeci cała krawędź i obraz
            // wygląda jak stary film, a nie jak gra
            bloom: 0.32,
            // próg wysoko ponad 1.0: poświatę daje tylko słońce i
            // wierzchołki, a nie każda jasna ściana
            bloom_threshold: 1.15,
            // delikatna winieta — powyżej 0.4 robi się „tunel"
            vignette: 0.30,
            // bardzo słaba aberracja: łamie idealną gładkość krawędzi
            chromatic: 0.0020,
            // ziarno ledwie widoczne; za dużo szumia w ciemnym terenie
            grain: 0.014,
            saturation: 1.10,
            contrast: 1.05,
            antialias: 0.65,
        }
    }
}

/// Wszystkie zasoby post-processingu.
pub struct PostFx {
    format: wgpu::TextureFormat,
    /// ZAWSZE 1. HDR jest próbkowane jako zwykła `texture_2d`, a taka
    /// próbka musi mieć `sample_count = 1`; przy MSAA wgpu odrzuci
    /// bind group. Krawędzie łagodzi shader (`fs_composite`), a nie
    /// multisampling — dlatego `uran-tanks` ustawia `.samples(1)`.
    samples: u32,

    /// Cel HDR sceny — tu trafiają bryły i niebo, dopóki nie przejdzie
    /// przez tonemapping.
    hdr: Option<wgpu::TextureView>,
    /// Dwa cele pośrednie bloom, oba w połowie rozmiaru kadru.
    bloom_a: Option<wgpu::TextureView>,
    bloom_b: Option<wgpu::TextureView>,
    sized: (u32, u32),

    params_buffer: wgpu::Buffer,
    /// Trzy bufory parametrów: bright, blur poziomy, blur pionowy.
    blur_buffers: [wgpu::Buffer; 3],

    layout: wgpu::BindGroupLayout,
    composite_pipeline: wgpu::RenderPipeline,
    bright_pipeline: wgpu::RenderPipeline,
    blur_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    /// Bind grupy zależne od tekstur — tworzone w `ensure_size`.
    composite_bg: Option<wgpu::BindGroup>,
    /// 0: hdr -> bright, 1: bloom_a -> blur H, 2: bloom_b -> blur V
    bloom_bgs: [Option<wgpu::BindGroup>; 3],
    settings: PostSettings,
}

impl PostFx {
    /// Tworzy potoki i bufory. `format` musi być formatem powierzchni.
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, samples: u32) -> Self {
        let params_buffer = uniform(device, "Uran Post Params", 48);
        let blur_buffers = [
            uniform(device, "Uran Blur Bright", 48),
            uniform(device, "Uran Blur H", 48),
            uniform(device, "Uran Blur V", 48),
        ];

        // JEDEN layout obsługuje wszystkie cztery passy: bright/blur/
        // composite mają ten sam zestaw bindingów, a nieużywane pomija
        // WGSL. Dzięki temu mamy jeden bind group zamiast trzech layoutów.
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran Post BGL"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran Post Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran Post Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });

        let composite_pipeline =
            pipeline(device, format, samples, &layout, &module, "fs_composite");
        // cele pośrednie bloom są 1:1 (bez MSAA) — rozmycie i tak gubi
        // szczegóły, a 4x więcej próbek kosztowałoby bez efektu
        let bright_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_bright");
        let blur_pipeline = pipeline(device, HDR_FORMAT, 1, &layout, &module, "fs_blur");

        Self {
            format,
            samples,
            hdr: None,
            bloom_a: None,
            bloom_b: None,
            sized: (0, 0),
            params_buffer,
            blur_buffers,
            layout,
            composite_pipeline,
            bright_pipeline,
            blur_pipeline,
            sampler,
            composite_bg: None,
            bloom_bgs: [None, None, None],
            settings: PostSettings::default(),
        }
    }

    /// Ustawienia wyglądu (do dostrojenia z gry).
    pub fn settings_mut(&mut self) -> &mut PostSettings {
        &mut self.settings
    }

    /// Format celu HDR: 16-bitowy float.
    ///
    /// `Rgba8Unorm` obciąłby wszystko powyżej 1.0, czyli dokładnie
    /// poświaty i tarczę słońca — a o nie chodzi w bloomie.
    pub fn hdr_format(&self) -> wgpu::TextureFormat {
        HDR_FORMAT
    }

    /// Cel HDR — do niego rysuje scena.
    pub fn hdr_view(&self) -> &wgpu::TextureView {
        self.hdr.as_ref().expect("ensure_size przed hdr_view")
    }

    /// Wymusza odtworzenie tekstur przy najbliższym `composite`
    /// (po zmianie rozmiaru okna).
    pub fn invalidate(&mut self) {
        self.sized = (0, 0);
    }
}

impl PostFx {
    /// Tworzy tekstury i bind grupy dla aktualnego rozmiaru okna.
    pub fn ensure_size(&mut self, device: &wgpu::Device, w: u32, h: u32) {
        if self.sized == (w, h) {
            return;
        }
        self.sized = (w, h);

        self.hdr = Some(color_view(
            device,
            w,
            h,
            "Uran Post HDR",
            HDR_FORMAT,
            self.samples,
        ));
        let (bw, bh) = ((w / 2).max(1), (h / 2).max(1));
        self.bloom_a = Some(color_view(device, bw, bh, "Uran Bloom A", HDR_FORMAT, 1));
        self.bloom_b = Some(color_view(device, bw, bh, "Uran Bloom B", HDR_FORMAT, 1));

        let hdr = self.hdr.as_ref().expect("właśnie utworzone");
        let ba = self.bloom_a.as_ref().expect("właśnie utworzone");
        let bb = self.bloom_b.as_ref().expect("właśnie utworzone");
        // w bright/blur binding 2 to samo źródło — nieużywany przez WGSL
        self.bloom_bgs[0] = Some(self.bind(device, &self.blur_buffers[0], hdr, hdr));
        self.bloom_bgs[1] = Some(self.bind(device, &self.blur_buffers[1], ba, ba));
        self.bloom_bgs[2] = Some(self.bind(device, &self.blur_buffers[2], bb, bb));
        self.composite_bg = Some(self.bind(device, &self.params_buffer, hdr, ba));
    }

    /// Bind grupa: uniform + dwie tekstury + sampler.
    fn bind(
        &self,
        device: &wgpu::Device,
        params: &wgpu::Buffer,
        src: &wgpu::TextureView,
        bloom: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran Post BG"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(src),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(bloom),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Wpisuje uniformy na bieżącą klatkę.
    ///
    /// Jeden `write_buffer` na pass — łącznie 4 na klatkę, niezależnie
    /// od liczby obiektów w scenie.
    pub fn upload_params(&self, queue: &wgpu::Queue, w: u32, h: u32, time: f32) {
        let s = &self.settings;
        let main = PostParams {
            grade: [s.exposure, s.bloom, s.vignette, s.chromatic],
            fx: [s.bloom_threshold, s.grain, s.saturation, s.contrast],
            screen: [w as f32, h as f32, time, s.antialias],
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&main));

        // bright: próg w `fx.x`, kierunek w `grade.x` jest tu nieistotny
        let bright = PostParams {
            grade: [0.0; 4],
            fx: [s.bloom_threshold, 0.0, 1.0, 1.0],
            screen: [w as f32, h as f32, 0.0, 0.0],
        };
        queue.write_buffer(&self.blur_buffers[0], 0, bytemuck::bytes_of(&bright));

        // blur: `grade.x` = kierunek (1 = poziomo, 0 = pionowo),
        // `screen.xy` = rozmiar źródła, bo krok kernela jest w UV
        let mut blur_h = bright;
        blur_h.grade[0] = 1.0;
        blur_h.screen = [(w / 2).max(1) as f32, (h / 2).max(1) as f32, 0.0, 0.0];
        queue.write_buffer(&self.blur_buffers[1], 0, bytemuck::bytes_of(&blur_h));
        let mut blur_v = blur_h;
        blur_v.grade[0] = 0.0;
        queue.write_buffer(&self.blur_buffers[2], 0, bytemuck::bytes_of(&blur_v));
    }
}

/// Bufor uniformu o zadanym rozmiarze w bajtach.
fn uniform(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl PostFx {
    /// Wykonuje bloom (3 passy) i kompozycję na powierzchnię gry.
    ///
    /// Wszystkie passy idą w TYM SAMYM encoderze, więc GPU nie czeka
    /// na osobne synchronizacje między nimi.
    pub fn composite(&self, enc: &mut wgpu::CommandEncoder, target: &Scene3dTarget<'_>) {
        let ba = self.bloom_a.as_ref().expect("ensure_size");
        let bb = self.bloom_b.as_ref().expect("ensure_size");

        // --- bright-pass: hdr -> bloom_a
        if let Some(bg) = self.bloom_bgs[0].as_ref() {
            self.fullscreen(enc, "Uran Bright", &self.bright_pipeline, bg, ba);
        }
        // --- rozmycie poziome: bloom_a -> bloom_b
        if let Some(bg) = self.bloom_bgs[1].as_ref() {
            self.fullscreen(enc, "Uran Blur H", &self.blur_pipeline, bg, bb);
        }
        // --- rozmycie pionowe: bloom_b -> bloom_a
        if let Some(bg) = self.bloom_bgs[2].as_ref() {
            self.fullscreen(enc, "Uran Blur V", &self.blur_pipeline, bg, ba);
        }

        // --- kompozycja na powierzchnię (tu widać efekty)
        let Some(bg) = self.composite_bg.as_ref() else {
            return;
        };
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Uran Composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.color,
                resolve_target: target.resolve,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        p.set_pipeline(&self.composite_pipeline);
        p.set_bind_group(0, bg, &[]);
        p.draw(0..3, 0..1);
    }

    /// Pełnoekranowy pass pośredni (bez depth, jeden trójkąt).
    fn fullscreen(
        &self,
        enc: &mut wgpu::CommandEncoder,
        label: &str,
        pipeline: &wgpu::RenderPipeline,
        bind_group: &wgpu::BindGroup,
        view: &wgpu::TextureView,
    ) {
        let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        p.set_pipeline(pipeline);
        p.set_bind_group(0, bind_group, &[]);
        // 3 wierzchołki = pełny prostokąt, bez bufora wierzchołków
        p.draw(0..3, 0..1);
    }
}

/// Pełnoekranowy potok (bloom, kompozycja) — wspólna struktura.
fn pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    layout: &wgpu::BindGroupLayout,
    module: &wgpu::ShaderModule,
    entry: &str,
) -> wgpu::RenderPipeline {
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Uran Post Layout"),
        bind_group_layouts: &[layout],
        push_constant_ranges: &[],
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Uran Post Pipeline"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module,
            entry_point: "vs_fullscreen",
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: entry,
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: samples,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
    })
}

/// Widok kolorowy do renderowania i próbkowania.
fn color_view(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    label: &str,
    format: wgpu::TextureFormat,
    samples: u32,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            // TEXTURE_BINDING, bo passy bloom i kompozycja je czytają
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Format celów HDR i pośrednich.
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
