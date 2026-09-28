//! Bind grupy i potoki renderowania.
//!
//! Są dwa potoki:
//! * **sprite** — instancjonowane prostokąty (jeden draw call na
//!   kombinację tekstura + tryb mieszania),
//! * **mesh** — dowolne siatki trójkątów, transformacja per-draw
//!   w push constants.
//!
//! Oba mają ten sam layout bind grupy (sampler + tekstura), więc jedna
//! bind grupa obsługuje wszystkie potoki.

use std::collections::HashMap;

use uran_ecs::BlendMode;

use crate::backend::device::PUSH_CONSTANT_SIZE;

/// Bind grupy per-tekstura: sampler + widok tekstury.
pub fn create_texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Uran Texture BGL"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
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
        ],
    })
}

/// Bind grupa globalna: macierz świata -> NDC.
pub fn create_globals_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Uran Globals BGL"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Wspólny sampler: najbliższy sąsiad + mitygacja liniowa.
pub fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Uran Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}

/// Stan blendingu dla danego trybu.
pub fn blend_state(mode: BlendMode) -> Option<wgpu::BlendState> {
    use wgpu::{BlendComponent, BlendFactor, BlendOperation};
    let (src, dst) = match mode {
        BlendMode::Alpha => (BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
        BlendMode::Additive => (BlendFactor::SrcAlpha, BlendFactor::One),
        BlendMode::Premultiplied => (BlendFactor::One, BlendFactor::OneMinusSrcAlpha),
        BlendMode::Replace => return None,
    };
    Some(wgpu::BlendState {
        color: BlendComponent { src_factor: src, dst_factor: dst, operation: BlendOperation::Add },
        alpha: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::OneMinusSrcAlpha,
            operation: BlendOperation::Add,
        },
    })
}

/// Cache potoków — każdy tryb mieszania dostaje własny potok.
pub struct PipelineCache {
    sprite: HashMap<BlendMode, wgpu::RenderPipeline>,
    mesh: HashMap<BlendMode, wgpu::RenderPipeline>,
    pub globals_layout: wgpu::BindGroupLayout,
    pub samples: u32,
    pub format: wgpu::TextureFormat,
}

impl PipelineCache {
    /// `texture_layout` i `sampler` są współdzielone z rejestrem tekstur —
    /// w wgpu 0.19 te handle'e nie implementują `Clone`, więc tworzymy je
    /// raz i podajemy przez referencję.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        samples: u32,
        shader: &wgpu::ShaderModule,
        texture_layout: &wgpu::BindGroupLayout,
        _sampler: &wgpu::Sampler,
    ) -> Self {
        let globals_layout = create_globals_bind_group_layout(device);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran Pipeline Layout"),
            bind_group_layouts: &[texture_layout, &globals_layout],
            push_constant_ranges: &[],
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran Mesh Pipeline Layout"),
            bind_group_layouts: &[texture_layout, &globals_layout],
            push_constant_ranges: &[wgpu::PushConstantRange {
                stages: wgpu::ShaderStages::VERTEX,
                range: 0..PUSH_CONSTANT_SIZE,
            }],
        });

        let mut cache = Self {
            sprite: HashMap::new(),
            mesh: HashMap::new(),
            globals_layout,
            samples,
            format,
        };
        // wszystkie tryby tworzymy od razu — potem `sprite()`/`mesh()` nic nie musi
        // tworzyć w trakcie klatki
        for mode in [
            BlendMode::Alpha,
            BlendMode::Additive,
            BlendMode::Premultiplied,
            BlendMode::Replace,
        ] {
            cache
                .sprite
                .insert(mode, create_sprite_pipeline(device, &layout, shader, format, samples, mode));
            cache
                .mesh
                .insert(mode, create_mesh_pipeline(device, &mesh_layout, shader, format, samples, mode));
        }
        cache
    }

    pub fn sprite(&self, mode: BlendMode) -> &wgpu::RenderPipeline {
        self.sprite.get(&mode).expect("brak potoku sprite dla trybu mieszania")
    }

    pub fn mesh(&self, mode: BlendMode) -> &wgpu::RenderPipeline {
        self.mesh.get(&mode).expect("brak potoku mesh dla trybu mieszania")
    }

    /// Bind grupa globalna z macierzą projekcji.
    pub fn globals_bind_group(&self, device: &wgpu::Device, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran Globals BG"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        })
    }
}

fn create_sprite_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    samples: u32,
    mode: BlendMode,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Uran Sprite Pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &[
                // @location(0): wierzchołek kwadratu
                wgpu::VertexBufferLayout {
                    array_stride: 2 * 4,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2],
                },
                // @location(1..4): dane instancji
                wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::batch::SpriteInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        1 => Float32x4,
                        2 => Float32x4,
                        3 => Float32x4,
                        4 => Float32x4
                    ],
                },
            ],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: blend_state(mode),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // bez cullingu — gry 2D rysują czasem wierzchołki w obu orientacjach
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        multiview: None,
    })
}

fn create_mesh_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    samples: u32,
    mode: BlendMode,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Uran Mesh Pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "mesh_vs_main",
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<uran_ecs::Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &wgpu::vertex_attr_array![
                    0 => Float32x2, // pozycja
                    1 => Float32x2, // uv
                    2 => Float32x4, // kolor
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "mesh_fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: blend_state(mode),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        multiview: None,
    })
}


/// Jednostkowy kwadrat w zakresie -0.5..0.5 (anchor w środku).
///
/// Kolejność: lewy-górny, prawy-górny, prawy-dolny, lewy-dolny — oś Y w
/// świecie idzie w górę, a UV (0,0) to lewy górny róg obrazu.
pub fn unit_quad_vertices() -> Vec<[f32; 2]> {
    vec![[-0.5, 0.5], [0.5, 0.5], [0.5, -0.5], [-0.5, -0.5]]
}

/// Indeksy kwadratu (2 trójkąty).
pub fn quad_indices() -> Vec<u32> {
    vec![0, 1, 2, 0, 2, 3]
}
