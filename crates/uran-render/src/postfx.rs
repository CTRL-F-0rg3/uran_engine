//! Post-processing 2D: delikatne rybie oko (barrel distortion).
//!
//! Odseparowany moduł, żeby `Renderer` nie pęczniał: on tylko przełącza
//! klatkę na teksturę pośrednią i woła [`PostFx::draw`], a wszystkie
//! szczegóły GPU (potok, uniform, tekstura sceny) żyją tutaj.
//!
//! ## Jak to działa
//!
//! 1. Klatka 2D (i 3D, jeśli jest) renderuje się do `PostFx::scene`
//!    zamiast prosto na powierzchnię okna.
//! 2. [`PostFx::draw`] zamienia tę teksturę na obraz końcowy pełnoekranowym
//!    trójkątem z shadera `post.wgsl`.
//! 3. Przy `enabled = false` nic się nie zmienia — klatka idzie prosto
//!    na powierzchnię, jakby post-processingu nie było.
//!
//! ## Wzór rybiego oka
//!
//! [`fisheye_uv`] to CPU-owe lustrzane odbicie fragment shadera. Są DWA
//! miejsca z tą samą matematyką i tylko testy (`cargo test`) pilnują,
//! żeby się nie rozeszły — shader nie da się sprawdzić bez GPU.
//!
//! Delikatny efekt to `strength ≈ 0.05..0.12`: środek kadru wybrzusza
//! się o kilka procent, krawędzie prawie stoją, a proste linie lekko
//! wyginają się jak deski beczki.

use uran_math::Vec2;

/// Ustawienia post-processingu 2D.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PostFxSettings {
    /// Czy post-processing ma być wykonywany.
    pub enabled: bool,
    /// Siła efektu rybiego oka: `0` = brak, `0.05..0.12` = delikatny,
    /// `0.3` = mocny. Wartości ujemne dają odwrócony efekt (pincushion).
    pub fisheye: f32,
}

impl Default for PostFxSettings {
    fn default() -> Self {
        Self {
            // Domyślnie wyłączony: gry, które nie proszą o post-processing,
            // renderują dokładnie tak jak wcześniej (zero zmian w klatce).
            enabled: false,
            // Delikatny — gotowy do użycia razem z `enabled: true`.
            fisheye: 0.06,
        }
    }
}

impl PostFxSettings {
    /// Przycina siłę do zakresu, którego shader jest w stanie przeżyć.
    ///
    /// Przy `strength` rzędu 1.0 mianownik normalizujący bliski zeru
    /// zamienia obraz w szum — dlatego granica, a nie liczenie na gracza.
    pub fn sanitized(mut self) -> Self {
        self.fisheye = self.fisheye.clamp(-0.5, 0.5);
        self
    }

    /// Czy pass ma w ogóle zostać wykonany (włączone i niezerowa siła).
    pub fn is_active(&self) -> bool {
        self.enabled && self.fisheye != 0.0
    }
}

/// CPU-owe odbicie wzoru z `post.wgsl` (struktura `fs_main`).
///
/// * `uv` — współrzędne próbkowania 0..1,
/// * `aspect` — proporcje kadru `w / h`,
/// * `strength` — siła efektu (0 = tożsamość).
///
/// Zwraca przesunięte `uv`, z którego shader ma pobrać piksel sceny.
/// Trzymaj w synchronizacji z shaderem — testy w tym module pilnują,
/// żeby wzór spełniał obietnice (tożsamość przy 0, próbka w kadrze itd.).
pub fn fisheye_uv(uv: Vec2, aspect: f32, strength: f32) -> Vec2 {
    let mut c = uv - Vec2::splat(0.5);
    c.x *= aspect;
    let r2 = c.length_squared();
    let corner = 0.25 * (aspect * aspect + 1.0);
    let scale = (1.0 + strength * r2) / (1.0 + strength * corner);
    let mut d = c * scale;
    d.x /= aspect;
    Vec2::splat(0.5) + d
}

/// Parametry passu (UKŁAD ZGODNY Z `Params` W `post.wgsl`).
///
/// Jeden `vec4`, bo WGSL wyrównuje uniform do 16 bajtów.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PostUniform {
    /// `x` = siła rybiego oka, `y` = proporcje (w/h), `z,w` = rezerwa.
    p: [f32; 4],
}

/// Zasoby GPU post-processingu 2D.
///
/// Tworzone przy pierwszym włączeniu (patrz `Renderer::set_post_fx`),
/// więc gry z wyłączonym post-processingiem nie płacą nawet za
/// skompilowanie shadera.
pub struct PostFx {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    /// Tekstura pośrednia: tu zapisuje się klatka 2D.
    scene: wgpu::Texture,
    scene_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    format: wgpu::TextureFormat,
    size: (u32, u32),
}

impl PostFx {
    /// Buduje potok, uniform i teksturę sceny (1×1 — prawdziwy rozmiar
    /// dopiero przy pierwszej klatce, patrz [`PostFx::ensure_size`]).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran Post 2D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("post.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran Post BGL"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran Post Pipeline Layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Uran Post Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                // Wierzchołki generuje `vs_main` z `vertex_index` —
                // pełnoekranowy trójkąt nie potrzebuje bufora.
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Zastępujemy piksele 1:1 — scena jest już pomalowana.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            // Pass idzie prosto na powierzchnię okna (zawsze 1 próbka).
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        // Liniowy sampling: distortions zmienia współrzędne ciągle,
        // a nearest-neighbor dawałby schodki na wygiętych krawędziach.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Uran Post Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Post Uniform"),
            size: std::mem::size_of::<PostUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (scene, scene_view, bind_group) =
            Self::create_scene(device, &layout, &sampler, &uniform, format, 1, 1);

        Self {
            pipeline,
            layout,
            sampler,
            uniform,
            scene,
            scene_view,
            bind_group,
            format,
            size: (1, 1),
        }
    }

    /// Widok tekstury sceny — tarcza, do której renderuje się klatka 2D.
    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene_view
    }

    /// Dopasowuje teksturę pośrednią do rozmiaru okna (no-op, gdy się
    /// nie zmienił). Wywoływane co klatkę — resize okna łapie się bez
    /// osobnego hooka.
    pub fn ensure_size(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.size == (width, height) {
            return;
        }
        let (scene, scene_view, bind_group) = Self::create_scene(
            device,
            &self.layout,
            &self.sampler,
            &self.uniform,
            self.format,
            width,
            height,
        );
        self.scene = scene;
        self.scene_view = scene_view;
        self.bind_group = bind_group;
        self.size = (width, height);
    }

    /// Wgrywa parametry bieżącej klatki (siła + proporcje kadru).
    pub fn upload_params(&self, queue: &wgpu::Queue, fisheye: f32, aspect: f32) {
        let uniform = PostUniform {
            p: [fisheye, aspect, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
    }

    /// Pass zamieniający teksturę sceny na obraz końcowy `target`.
    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Uran Post Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Trójkąt pokrywa cały kadr — clear to tylko
                    // zabezpieczenie, gdyby geometria się kiedyś zmieniła.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Tekstura sceny + widok + bind grupa (nowe przy każdym resize).
    fn create_scene(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        uniform: &wgpu::Buffer,
        format: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uran Post Scene"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uran Post Bind Group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(uniform.as_entire_buffer_binding()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });
        (texture, view, bind_group)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Próbkę trzymamy w kadrze dla każdej proporcji i siły — bez tego
    /// shader smaruje krawędzie i zostawia czarne plamy w rogach.
    #[test]
    fn sample_never_leaves_the_frame() {
        for aspect in [1.0, 4.0 / 3.0, 16.0 / 9.0, 2.4] {
            for strength in [0.0, 0.06, 0.12, 0.5] {
                for uv in [
                    Vec2::ZERO,
                    Vec2::ONE,
                    Vec2::new(1.0, 0.0),
                    Vec2::new(0.0, 1.0),
                    Vec2::new(0.5, 0.5),
                    Vec2::new(0.25, 0.75),
                ] {
                    let out = fisheye_uv(uv, aspect, strength);
                    assert!(
                        out.x >= 0.0 && out.x <= 1.0 && out.y >= 0.0 && out.y <= 1.0,
                        "próbka {out:?} wychodzi z kadru (uv={uv:?}, aspect={aspect}, strength={strength})"
                    );
                }
            }
        }
    }

    /// Przy zerowej sile efekt musi być tożsamością — inaczej same
    /// włączenie post-processingu przesuwałoby obraz.
    #[test]
    fn zero_strength_is_identity() {
        for uv in [Vec2::ZERO, Vec2::ONE, Vec2::new(0.3, 0.7), Vec2::splat(0.5)] {
            let out = fisheye_uv(uv, 16.0 / 9.0, 0.0);
            assert!((out - uv).length() < 1e-6, "{uv:?} -> {out:?}");
        }
    }

    /// Środek kadru jest punktem stałym (nie rusza się przy żadnej sile).
    #[test]
    fn center_stays_in_place() {
        let out = fisheye_uv(Vec2::splat(0.5), 16.0 / 9.0, 0.3);
        assert!((out - Vec2::splat(0.5)).length() < 1e-6, "{out:?}");
    }

    /// Beczka: skala próbki rośnie z promieniem, więc środek wybrzusza
    /// się bardziej niż krawędź. Gdyby spadała, mielibyśmy pincushion.
    #[test]
    fn distortion_grows_towards_the_edge() {
        let aspect = 16.0 / 9.0;
        let strength = 0.1;
        // Dwa punkty na tej samej poziomej osi, różne odległości od środka.
        let near = fisheye_uv(Vec2::new(0.6, 0.5), aspect, strength);
        let far = fisheye_uv(Vec2::new(0.9, 0.5), aspect, strength);
        let scale_near = (near.x - 0.5) / 0.1;
        let scale_far = (far.x - 0.5) / 0.4;
        assert!(
            scale_near < scale_far,
            "skala musi rosnąć z promieniem: {scale_near} !< {scale_far}"
        );
    }

    /// Efekt jest symetryczny względem środka kadru.
    #[test]
    fn distortion_is_symmetric() {
        let aspect = 4.0 / 3.0;
        let left = fisheye_uv(Vec2::new(0.2, 0.5), aspect, 0.1);
        let right = fisheye_uv(Vec2::new(0.8, 0.5), aspect, 0.1);
        assert!(((left.x - 0.5) + (right.x - 0.5)).abs() < 1e-6);
    }

    /// Domyślnie post-processing jest wyłączony — gry, które nie proszą
    /// o efekt, renderują bez zmian.
    #[test]
    fn default_is_disabled() {
        let s = PostFxSettings::default();
        assert!(!s.enabled);
        assert!(!s.is_active());
    }

    /// Siła spoza zakresu jest przycinana, a `is_active` pilnuje
    /// zera (pass przy zerowej sile to zmarnowana klatka GPU).
    #[test]
    fn settings_are_sanitized() {
        assert_eq!(PostFxSettings::default().sanitized().fisheye, 0.06);
        let wild = PostFxSettings {
            enabled: true,
            fisheye: 9.9,
        }
        .sanitized();
        assert_eq!(wild.fisheye, 0.5);
        assert!(!PostFxSettings {
            enabled: true,
            fisheye: 0.0,
        }
        .is_active());
        assert!(PostFxSettings {
            enabled: true,
            fisheye: 0.1,
        }
        .is_active());
    }
}

#[cfg(test)]
mod shader_tests {
    //! Testy parsujące WGSL.
    //!
    //! Shader jest zwykłym plikiem włączonym przez `include_str!`, więc
    //! literówka w nim nie zatrzymuje kompilacji Rusta — wgpu zgłosi
    //! błąd dopiero przy pierwszym uruchomieniu gry. Te testy przenoszą
    //! wykrywanie błędów na `cargo test`, gdzie nie ma GPU ani okna.

    use naga::valid::{Capabilities, ValidationFlags, Validator};

    fn validate(label: &str, source: &str) -> Result<(), String> {
        let module = naga::front::wgsl::parse_str(source)
            .map_err(|e| format!("{label}: błąd parsowania WGSL:\n{e:?}"))?;
        Validator::new(ValidationFlags::all(), Capabilities::default())
            .validate(&module)
            .map(|_| ())
            .map_err(|e| format!("{label}: błąd walidacji WGSL:\n{e:?}"))
    }

    #[test]
    fn post_shader_parses() {
        let src = include_str!("post.wgsl");
        if let Err(e) = validate("post.wgsl", src) {
            panic!("{e}");
        }
    }

    /// Potok w `postfx.rs` szuka punktów wejścia po nazwie — chronimy
    /// przed zmianą nazwy w WGSL bez zmiany w Rust.
    #[test]
    fn post_shader_exposes_expected_entry_points() {
        let src = include_str!("post.wgsl");
        for entry in ["fn vs_main", "fn fs_main"] {
            assert!(
                src.contains(entry),
                "post.wgsl nie zawiera punktu wejścia `{entry}`"
            );
        }
    }
}
