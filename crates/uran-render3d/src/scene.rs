//! Renderer 3D: własne potoki, własny depth buffer, własny shader.
//!
//! Współdzieli z rendererem 2D wyłącznie `Device`/`Queue` (patrz
//! `uran_render::scene3d`). Nic nie jest współdzielone na poziomie potoków,
//! buforów ani sharderów — dlatego dodanie 3D nie dotyka kodu 2D.

use uran_math::{Mat4, Vec3};
use uran_render::{Scene3d, Scene3dTarget};
use wgpu::util::DeviceExt;

use crate::camera::Camera3d;
use crate::mesh::{GpuMesh, InstanceModel, Mesh, SceneUniform};

/// Identyfikator siatki w rejestrze renderera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MeshId(pub u32);

/// Jedno polecenie rysowania: „narysuj tę siatkę w tej macierzy".
#[derive(Debug, Clone, Copy)]
pub struct DrawCmd {
    pub mesh: MeshId,
    pub model: Mat4,
    /// Mnożnik koloru wierzchołka — pozwala jedną siatkę malować na
    /// różne kolory (gracz / przeciwnik) bez duplikowania geometrii.
    pub tint: [f32; 4],
}

impl DrawCmd {
    pub fn new(mesh: MeshId, model: Mat4) -> Self {
        Self { mesh, model, tint: [1.0; 4] }
    }

    pub fn tinted(mesh: MeshId, model: Mat4, tint: [f32; 4]) -> Self {
        Self { mesh, model, tint }
    }
}

/// Parametry oświetlenia.
#[derive(Debug, Clone, Copy)]
pub struct Lighting {
    /// Kierunek, w którym świeci (wskazuje OD źródła).
    pub light_dir: Vec3,
    pub light_color: [f32; 3],
    pub ambient: [f32; 3],
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            // słońce lekko z boku i z góry — inaczej bryły są płaskie
            light_dir: Vec3::new(0.45, 0.82, 0.35),
            light_color: [1.0, 0.97, 0.90],
            ambient: [0.30, 0.34, 0.42],
        }
    }
}

/// Renderer 3D — implementacja [`Scene3d`].
///
/// Świadomie NIE trzymamy `Device`/`Queue` w polach. W wgpu 0.19 oba typy
/// nie implementują `Clone`, a urządzenie należy do renderera 2D. Zamiast
/// tego każdą operację na GPU wykonujemy przez `Scene3dTarget::gpu`, które
/// `uran-render` przekazuje w `draw()`. Jedyny wyjątek to konstrukcja,
/// gdzie `&GpuContext` mamy z parametru.
pub struct Renderer3d {
    format: wgpu::TextureFormat,
    samples: u32,

    /// Depth buffer 3D. 2D go nie ma w ogóle (rysowanie płaskie), więc
    /// każdy z rendererów ma swój.
    depth: Option<wgpu::TextureView>,
    depth_size: (u32, u32),

    scene_buffer: wgpu::Buffer,
    models_buffer: wgpu::Buffer,
    models_capacity: usize,
    bind_group: wgpu::BindGroup,
    /// Layout bind grupy trzymany osobno: `ensure_capacity` musi przebudować
    /// bind grupę po zmianie bufora modeli, a layout musi zostać ten sam.
    models_bind_layout: wgpu::BindGroupLayout,

    pipeline: wgpu::RenderPipeline,
    meshes: Vec<GpuMesh>,

    camera: Camera3d,
    lighting: Lighting,
    clear_color: [f32; 4],
    time: f32,
    commands: Vec<DrawCmd>,
    /// Ile obiektów narysowano w ostatniej klatce (do HUD-a i diagnostyki).
    pub last_draw_count: usize,
}

impl Renderer3d {
    /// Tworzy renderer 3D na tym samym urządzeniu, co renderer 2D.
    pub fn new(
        gpu: &uran_render::GpuContext,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let device = &gpu.device;

        let scene_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran 3D Scene Uniform"),
            size: std::mem::size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Tablica modeli. Rośnie potęgami 2, a bind grupa przebudowywana
        // jest raz przy zmianie rozmiaru (zdarzenie rzadkie).
        let models_capacity = 256usize;
        let models_buffer = create_models_buffer(&device, models_capacity);

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran 3D BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    // modele czyta TYLKO shader wierzchołkowy
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = make_bind_group(&device, &layout, &scene_buffer, &models_buffer);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran 3D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("s3d.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran 3D Pipeline Layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Uran 3D Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::mesh::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // pozycja, normalna, kolor — trzy float3 pod rząd
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 12,
                            shader_location: 1,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 24,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // tyłna ściana musi być rysowana, inaczej wnętrze czołgu
                // wygląda dziurawie
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let (w, h) = gpu.size();
        Self {
            format,
            samples,
            depth: None,
            depth_size: (0, 0),
            scene_buffer,
            models_buffer,
            models_capacity,
            bind_group,
            models_bind_layout: layout,
            pipeline,
            meshes: Vec::new(),
            camera: Camera3d::default(),
            lighting: Lighting::default(),
            clear_color: [0.45, 0.55, 0.68, 1.0],
            time: 0.0,
            commands: Vec::new(),
            last_draw_count: 0,
        }
        .with_depth(device, w, h)
    }

    /// Kamera (do edycji przez grę).
    pub fn camera(&self) -> &Camera3d {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera3d {
        &mut self.camera
    }

    pub fn lighting_mut(&mut self) -> &mut Lighting {
        &mut self.lighting
    }

    /// Kolor czyszczenia (niebo / mgła).
    pub fn set_clear_color(&mut self, c: [f32; 4]) {
        self.clear_color = c;
    }

    /// Przesuwa czas świata (mgła, animacje).
    pub fn advance(&mut self, dt: f32) {
        self.time += dt;
    }

    /// Rejestruje siatkę i zwraca jej identyfikator.
    ///
    /// `gpu` podajemy jawnie, bo renderer nie trzyma urządzenia w polach
    /// (patrz dokumentacja struktury).
    pub fn add_mesh(&mut self, gpu: &uran_render::GpuContext, mesh: &Mesh, label: &str) -> MeshId {
        let id = MeshId(self.meshes.len() as u32);
        self.meshes.push(mesh.upload(&gpu.device, label));
        id
    }

    /// Podaje listę obiektów na bieżącą klatkę.
    ///
    /// Przyjmujemy wektor w posiadanie: nie kopiujemy, tylko podmieniamy.
    /// `Vec<DrawCmd>` przy kilkunastu czołgach to ułamek kilikilobajta,
    /// a kopiowanie co klatkę tylko wypychałoby pamięć.
    pub fn set_commands(&mut self, commands: Vec<DrawCmd>) {
        self.commands = commands;
    }

    /// Liczba zarejestrowanych siatek.
    pub fn mesh_count(&self) -> usize {
        self.meshes.len()
    }

    /// Tworzy depth buffer pod aktualny rozmiar okna.
    ///
    /// Bufor głębokości musi mieć `sample_count` zgodny z potokiem —
    /// inaczej walidacja wgpu odrzuci pass z attachementem.
    fn with_depth(
        self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Self {
        let (w, h) = (width.max(1), height.max(1));
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uran 3D Depth"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: self.samples,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Self {
            depth: Some(texture.create_view(&wgpu::TextureViewDescriptor::default())),
            depth_size: (w, h),
            ..self
        }
    }

    /// Upewnia się, że tablica modeli pomieści `count` obiektów.
    ///
    /// Zmiana pojemności oznacza nowy bufor I nową bind grupę, dlatego
    /// trzymamy tu też `layout`, zamiast odtwarzać go przy każdym wzroście.
    fn ensure_capacity(&mut self, device: &wgpu::Device, count: usize) {
        if count <= self.models_capacity {
            return;
        }
        let mut capacity = self.models_capacity.max(1);
        while capacity < count {
            capacity *= 2;
        }
        self.models_buffer = create_models_buffer(device, capacity);
        self.models_capacity = capacity;
        self.bind_group = make_bind_group(
            device,
            &self.models_bind_layout,
            &self.scene_buffer,
            &self.models_buffer,
        );
    }

    /// Renderuje scenę: czyści kolor i głębokość, rysuje wszystkie obiekty.
    fn render_pass(&mut self, target: &Scene3dTarget<'_>) {
        let device = &target.gpu.device;
        let queue = &target.gpu.queue;
        let w = target.gpu.config.width.max(1);
        let h = target.gpu.config.height.max(1);
        if self.depth.is_none() || self.depth_size != (w, h) {
            self.depth_size = (w, h);
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Uran 3D Depth"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: self.samples,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            self.depth = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.camera.set_aspect(w as f32 / h as f32);
        }

        // uniform sceny: macierz, oko, światło
        let c = self.lighting.ambient;
        let uniform = SceneUniform {
            view_proj: self.camera.view_proj().to_cols_array_2d(),
            eye: [
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                0.0,
            ],
            light_dir: [
                self.lighting.light_dir.x,
                self.lighting.light_dir.y,
                self.lighting.light_dir.z,
                0.0,
            ],
            light_color: [
                self.lighting.light_color[0],
                self.lighting.light_color[1],
                self.lighting.light_color[2],
                0.0,
            ],
            ambient_time: [c[0], c[1], c[2], self.time],
        };
        queue.write_buffer(&self.scene_buffer, 0, bytemuck::bytes_of(&uniform));
        // tablica modeli: jeden wpis na obiekt
        self.ensure_capacity(device, self.commands.len());
        if !self.commands.is_empty() {
            let models: Vec<InstanceModel> = self
                .commands
                .iter()
                .map(|c| InstanceModel {
                    model: c.model.to_cols_array_2d(),
                    tint: c.tint,
                })
                .collect();
            queue.write_buffer(&self.models_buffer, 0, bytemuck::cast_slice(&models));
        }

        let clear = self.clear_color;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Uran 3D"),
        });

        {
            // klonujemy widok głębokości, żeby pożyczka `&mut pass` nie
            // kolidowała z pożyczką `&mut self` w pętli rysowania
            let depth_view = self.depth.as_ref().expect("depth utworzony wyżej").clone();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Uran 3D Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.color,
                    resolve_target: target.resolve,
                    ops: wgpu::Operations {
                        // 3D czyści sam: 2D dostanie potem LoadOp::Load
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: clear[0] as f64,
                            g: clear[1] as f64,
                            b: clear[2] as f64,
                            a: clear[3] as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);

            let mut drawn = 0usize;
            for (i, cmd) in self.commands.iter().enumerate() {
                let Some(mesh) = self.meshes.get(cmd.mesh.0 as usize) else {
                    // zły identyfikator — cicho pomijamy, bo walidacja
                    // wgpu nie zna indeksów naszych siatek
                    continue;
                };
                // `base_instance` = i+1, bo WGSL liczy `instance_index` od 1
                mesh.draw(&mut pass, 0, i as u32 + 1);
                drawn += 1;
            }
            self.last_draw_count = drawn;
        }

        queue.submit(Some(encoder.finish()));
    }
}

/// Bufor tablicy modeli o zadanej pojemności.
fn create_models_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Uran 3D Models"),
        size: (std::mem::size_of::<InstanceModel>() * capacity.max(1)) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Bind grupa: uniform sceny (0) + tablica modeli (1).
fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene: &wgpu::Buffer,
    models: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Uran 3D Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: scene.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: models.as_entire_binding() },
        ],
    })
}

impl Scene3d for Renderer3d {
    fn draw(&mut self, target: &Scene3dTarget<'_>) {
        self.render_pass(target);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.camera.set_aspect(width as f32 / height.max(1) as f32);
        // wymuszamy odtworzenie depth bufferu przy następnym `draw`
        self.depth = None;
    }

    fn is_active(&self) -> bool {
        true
    }
}