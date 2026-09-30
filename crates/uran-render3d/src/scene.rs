//! Renderer 3D: własne potoki, własny depth buffer, własny shader.
//!
//! Współdzieli z rendererem 2D wyłącznie `Device`/`Queue` (patrz
//! `uran_render::scene3d`). Nic nie jest współdzielone na poziomie potoków,
//! buforów ani sharderów — dlatego dodanie 3D nie dotyka kodu 2D.

use uran_math::{Mat4, Vec3};
use uran_render::{Scene3d, Scene3dTarget};

use crate::camera::Camera3d;
use crate::material::{MaterialBank, MaterialId};
use crate::mesh::{GpuMesh, InstanceModel, Mesh, SceneUniform};
use crate::postfx::{PostFx, PostSettings};
use crate::shadow::{SceneBounds, ShadowMap, ShadowSettings};

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
    /// Materiał PBR: tekstury, chropowatość, metaliczność.
    ///
    /// Domyślnie `MaterialId(0)` — to zawsze istniejący materiał
    /// zapasowy (biały, bez map). Dzięki niemu kod, który nie zna
    /// pojęcia materiału (cały `uran-tanks`), nie musi go udawać
    /// opcjonalnym i nie psuje się przy zmianie struktury komendy.
    pub material: MaterialId,
}

impl DrawCmd {
    pub fn new(mesh: MeshId, model: Mat4) -> Self {
        Self {
            mesh,
            model,
            tint: [1.0; 4],
            material: MaterialId(0),
        }
    }

    pub fn tinted(mesh: MeshId, model: Mat4, tint: [f32; 4]) -> Self {
        Self {
            mesh,
            model,
            tint,
            material: MaterialId(0),
        }
    }

    /// Przypisuje materiał — np. ten, który przyszedł z pliku `.mtl`.
    pub fn with_material(mut self, material: MaterialId) -> Self {
        self.material = material;
        self
    }
}

/// Parametry oświetlenia.
#[derive(Debug, Clone, Copy)]
pub struct Lighting {
    /// Kierunek, w którym świeci (wskazuje OD źródła).
    pub light_dir: Vec3,
    pub light_color: [f32; 3],
    /// Natężenie słońca w przestrzeni LINIOWEJ.
    ///
    /// Światło liczymy liniowo, gdzie 1.0 to „biały" — czyli wartość
    /// bliska 1 daje bardzo słabe słońce. Typowe słońce w bezchmurznym
    /// dniu to 3..6, a zachód słońca 1.5..2.5.
    ///
    /// Dawniej shader mnożył światło przez stałą 1.35, co było
    /// sprzężone z modelem Phonga; przy oświetleniu PBR i ACES ta
    /// wartość zmieniała jasność inaczej na każdej powierzchni.
    pub intensity: f32,
    pub ambient: [f32; 3],
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            // Słońce około 50° nad horyzontem i wyraźnie z boku. Niski kąt
            // daje długie, czytelne cienie i jasno pokazuje, w którą stronę
            // pada światło — przy słońcu prosto w głowę bryły są płaskie.
            light_dir: Vec3::new(0.40, 0.62, 0.68),
            // Ciepłe, lekko złotawe — słońce niskiego popołudnia.
            light_color: [1.0, 0.93, 0.80],
            // 4.0 = jasne popołudniowe słońce; z ACES w post-processingu
            // daje to biel na białych powierzchniach i nie prześwietla
            // asfaltu
            intensity: 4.0,
            // Ambient NIESIE NIEBO: chłodny, wyraźnie niebieski. Podnosi
            // cienie do poziomu otoczenia, zamiast zostawiać je czarne.
            ambient: [0.30, 0.40, 0.58],
        }
    }
}

/// Parametry hybrydy PBR + NPR — miara „rysunkowości" sceny.
///
/// To jest globalna skala, do której mnożymy stylizację z materiału
/// (`Surface::stylize`). Dzięki temu jedna scena może mieć rysunkową
/// postać i fizyczne otoczenie: materiał mówi „chcę tu rampę", a ten
/// parametr mówi „na ile mocno".
///
/// Przy `0.0` zachowanie jest czyste PBR. Wartość domyślna to `1.0`,
/// bo to gra decyduje, ile rysunkowości chce — nie renderer.
#[derive(Debug, Clone, Copy)]
pub struct Stylization {
    /// Globalna siła rampy anime 0..1.
    pub amount: f32,
    /// Miękkość progu terminatora. Małe = ostrzejszy rysunek.
    pub softness: f32,
    /// Globalna siła podpowierzchniowego rozpraszania (mnoży tę z materiału).
    pub sss: f32,
}

impl Default for Stylization {
    fn default() -> Self {
        Self {
            // 1.0 = materiał decyduje w pełni. Ustawienie 0.0 byłoby
            // „czyste PBR zawsze", a to odbierałoby grze narzędzie.
            amount: 1.0,
            // 0.12 to rampa wyraźnie widoczna, ale z gradientem szerokości
            // kilku stopni. Poniżej 0.05 krawędź zaczyna migotać przy
            // ruchu kamery.
            softness: 0.12,
            sss: 1.0,
        }
    }
}

/// Mgła i niebo — atmosfera sceny.
#[derive(Debug, Clone, Copy)]
pub struct Atmosphere {
    /// Czy rysować fizyczne niebo (Rayleigh + Mie).
    ///
    /// Gdy `false`, kadr wypełnia [`Renderer3d::set_clear_color`], a mgła
    /// nadal działa. To pozwala grze przełączać „wewnętrzna hala" (brak
    /// nieba) i „pod gołym niebem" jednym wywołaniem.
    pub sky: bool,
    /// Gęstość mgły 0..1 — odpowiada za `1 - e^(-d·ρ)`.
    ///
    /// 0.004 daje mgłę widoczną od ~150 m; 0.012 zaczyna zasłaniać
    /// odległe budynki. Powyżej 0.03 scena znika całkowicie.
    pub fog_density: f32,
    /// Kolor mgły w sRGB, zakres 0..1.
    ///
    /// Niebo wzmacnia go w kierunku słońca (rozpraszanie Mie), więc
    /// podstawowy kolor można ustawić neutralnie i nieba nie trzeba
    /// przeliczać ręcznie.
    pub fog_color: [f32; 3],
    /// Wysokość (w metrach), na której mgła całkiem zanika.
    ///
    /// ## Dlaczego to pole istnieje
    ///
    /// Mgła nie jest jednorodna: jej gęstość rośnie w dół i zanika
    /// powyżej pewnej wysokości. Bez tego parametru mgła zasłaniała
    /// tak samo kamień leżący na ziemi i dach domu stojącego 30 m
    /// wyżej — a w realnym świecie dach jest czysty.
    ///
    /// Shader liczy analityczny całkowity gęstości wzdłuż promienia
    /// oka (jak WickedEngine w `fogHF.hlsli`), więc ta wartość wprost
    /// steruje tym, jak szybko mgła zanika ku górze.
    ///
    /// 0 wyłącza ten efekt i daje z powrotem jednolitą mgłę.
    pub fog_height: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Self {
            sky: true,
            // 0.0045 = mgła ledwo widoczna na dystansie kilkudziesięciu
            // metrów. Wyższa wartość zjadała czytelność sylwetek
            // przeciwników, co w grze akcji jest niepożądane.
            fog_density: 0.0045,
            // Lekko chłodna, pasująca do niebieskiego ambientu. Ciepła mgła
            // przy zimnym otoczeniu wygląda jak brud na obiektywie.
            fog_color: [0.58, 0.70, 0.86],
            // 55 m: powyżej wierzchu typowych budynków mgła już dawno
            // zniknęła, więc dachy i kominy zostają czyste, a dolne
            // piętra i ulica toną. 0 dałoby z powrotem jednolitą
            // „mleczną" warstwę na każdej wysokości.
            fog_height: 55.0,
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
    /// Sama tekstura głębokości — trzymana osobno od widoku, bo
    /// post-processing potrzebuje jej jako ŹRÓDŁA do kopii
    /// (`PostFx::copy_depth`). Sam `TextureView` nie wystarczy:
    /// `copy_texture_to_texture` przyjmuje `&Texture`.
    depth_tex: Option<wgpu::Texture>,
    depth_size: (u32, u32),

    scene_buffer: wgpu::Buffer,
    models_buffer: wgpu::Buffer,
    models_capacity: usize,
    bind_group: wgpu::BindGroup,
    /// Layout bind grupy trzymany osobno: `ensure_capacity` musi przebudować
    /// bind grupę po zmianie bufora modeli, a layout musi zostać ten sam.
    models_bind_layout: wgpu::BindGroupLayout,

    /// Post-processing (AA, bloom, tonemapping) — własny moduł.
    postfx: PostFx,

    pipeline: wgpu::RenderPipeline,
    /// Pipeline nieba: pełnoekranowy trójkąt z fizycznym rozpraszaniem
    /// atmosferycznym. Rysowany jako pierwszy w passie głównym.
    ///
    /// Niebo opcjonalne — patrz [`Atmosphere`]. Gdy `enabled` jest wyłączone,
    /// pipeline w ogóle nie jest rysowany, a kadr wypełnia kolor czyszczenia.
    sky_pipeline: wgpu::RenderPipeline,
    /// Pipeline passu cieni: ten sam shader, inny punkt wejścia
    /// (`vs_shadow`) i brak fragment shadera. Rysuje wyłącznie
    /// głębokość do mapy cieni.
    shadow_pipeline: wgpu::RenderPipeline,
    meshes: Vec<GpuMesh>,
    /// Materiały PBR i ich bind grupy (bind group 1 w potoku).
    pub materials: MaterialBank,

    /// Mapa cieni kierunkowych (tekstura głębokości + próbnik porównawczy).
    pub shadow: ShadowMap,
    /// Layout bind grupy samego passu cieni.
    ///
    /// OSOBNY layout, bo wgpu zabrania wiązania zasobu, który w tym
    /// samym passie jest celem renderowania. Gdybyśmy użyli tu tej
    /// samej grupy co w passie głównym, mapa cieni byłaby jednocześnie
    /// źródłem i celem — walidacja odrzuci pass.
    shadow_bind_layout: wgpu::BindGroupLayout,
    /// Bind grupy passu cieni: uniform sceny + tablica modeli.
    shadow_bind_group: wgpu::BindGroup,

    camera: Camera3d,
    lighting: Lighting,
    /// Globalna skala rysunkowości (rampa anime na dyfuzji).
    stylization: Stylization,
    /// Niebo i mgła.
    atmosphere: Atmosphere,
    clear_color: [f32; 4],
    time: f32,
    commands: Vec<DrawCmd>,
    /// Ile obiektów narysowano w ostatniej klatce (do HUD-a i diagnostyki).
    pub last_draw_count: usize,
}

impl Renderer3d {
    /// Tworzy renderer 3D na tym samym urządzeniu, co renderer 2D.
    pub fn new(gpu: &uran_render::GpuContext, format: wgpu::TextureFormat, samples: u32) -> Self {
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
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    // mapę cieni czyta shader FRAGMENT
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // `Depth` wymusza `texture_depth_2d` w WGSL, co
                        // jest warunkiem użycia `textureSampleCompare`.
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    // `Comparison` daje `sampler_comparison` w WGSL.
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });

        // Mapa cieni musi istnieć PRZED bind grupą: binding 2 w grupie 0
        // wskazuje na jej widok, a binding 3 na jej próbnik.
        let shadow = ShadowMap::new(&device, ShadowSettings::default());

        let bind_group = make_bind_group(
            &device,
            &layout,
            &scene_buffer,
            &models_buffer,
            Some(shadow.view()),
            Some(shadow.sampler()),
        );

        // Layout passu cieni: TYLKO uniform sceny i tablica modeli.
        // Świadomie bez mapy cieni — w trakcie tego passu jest ona
        // celem renderowania, a wgpu odrzuca zasób użyty w jednym
        // passie jako źródło i jako attachment.
        let shadow_bind_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Uran 3D Shadow BGL"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
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
        // Layout passu cienia deklaruje TYLKO bindingi 0 i 1, wiec
        // przekazujemy `None`/`None` — `create_bind_group` wymaga
        // dokladnie tylu wpisow, ile jest w layoutcie.
        let shadow_bind_group = make_bind_group(
            &device,
            &shadow_bind_layout,
            &scene_buffer,
            &models_buffer,
            None,
            None,
        );

        // Materialy PBR: layout z grupy 1. Bank tworzymy tu, bo
        // `pipeline_layout` musi znać jego layout, a sam bank potrzebuje
        // `device` i `queue`.
        // Layout trzymamy w `Arc`, bo `wgpu::BindGroupLayout` nie
        // implementuje `Clone` w wgpu 0.19.
        let materials = MaterialBank::new(&device, &gpu.queue);
        let materials_layout = std::sync::Arc::clone(&materials.layout);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran 3D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("s3d.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran 3D Pipeline Layout"),
            bind_group_layouts: &[&layout, materials_layout.as_ref()],
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
                    // Kolejność musi być identyczna z `Vertex`: pozycja,
                    // normalna, UV, kolor. Offsety liczymy ręcznie, bo
                    // `#[repr(C)]` daje 44 B bez wypełnienia.
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
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 32,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                // DWA cele kolorowe (G-Buffer):
                //   [0] HDR — to, co widzi gracz,
                //   [1] normalna w przestrzeni oka + chropowatość, z czego
                //       post-processing bierze SSR, kontury, SSS i DoF.
                // Oba muszą mieć format zgodny z załącznikami render passu
                // (patrz `PostFx::gbuffer_view`), inaczej walidacja wgpu
                // odrzuci potok.
                targets: &[
                    Some(wgpu::ColorTargetState {
                        // Scena rysuje do celu HDR (Rgba16Float), a NIE na
                        // powierzchnię. Format musi się zgadzać z załącznikiem
                        // render passu, inaczej walidacja wgpu to odrzuci.
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                ],
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

        // --- pipeline nieba ---
        //
        // Pełnoekranowy trójkąt rysowany PRZED bryłami, z wyłączonym
        // zapisem głębokości. Dzięki temu:
        //   * niebo wypełnia cały kadru bez geometrii,
        //   * każda bryła go zasłania, bo ma mniejszą głębokość.
        //
        // Layout ma TYLKO grupę 0 (uniform sceny) i to samo, co główny
        // potok — niebo potrzebuje kierunku słońca, a nic więcej.
        // Nie ustawiamy grupy 1 (materiały), bo shader nieba nie ma
        // żadnego `@group(1)`.
        // Layout ma TYLKO grupę 0 (uniform sceny) — i to wymaga OSOBNEGO
        // `PipelineLayout`, bo `pipeline_layout` głównego potoku zawiera
        // również grupę 1 (materiały). Gdybyśmy podali go tutaj, wgpu
        // wymagałby powiązania grupy 1 przy każdym rysowaniu nieba,
        // a `draw` nie ustawia żadnej — błąd walidacji wyskakiwałby
        // dopiero w trakcie renderowania, mimo że shader nieba nie ma
        // ani jednego `@group(1)`.
        let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran 3D Layout Nieba"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Uran 3D Sky Pipeline"),
            layout: Some(&sky_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_sky",
                // Pusty bufor wierzchołków: pozycje generuje sam shader
                // z `vertex_index`, więc geometria nie jest potrzebna.
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_sky",
                // Drugi cel MUSI tu być, nawet jeśli nic do niego nie
                // zapisujemy: render pass ma DWA załączniki kolorowe
                // (HDR + G-Buffer), a wgpu wymaga, żeby potok deklarował
                // dokładnie tyle samo targetów, ile pass ma załączników.
                // Bez tego walidacja odrzuca `set_pipeline` komunikatem
                // „targets are incompatible" — i to dopiero w trakcie
                // renderowania, czyli po zbudowaniu okna i wczytaniu
                // assetów, nie na `cargo test`.
                targets: &[
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    }),
                    Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba16Float,
                        blend: Some(wgpu::BlendState::REPLACE),
                        // Niebo nie ma normalnej ani chropowatości, więc
                        // G-Bufera nie ruszamy. `empty()` mówi
                        // „załącznik jest, ale nic do niego nie piszemy" —
                        // taniej niż generowanie zerowego wektora
                        // normalnej w shaderze.
                        write_mask: wgpu::ColorWrites::empty(),
                    }),
                ],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                // `false` jest tu kluczowe: gdyby niebo zapisywało
                // głębokość 1.0, zamalowałoby nim bryły narysowane
                // wcześniej (przy `Less` nie, ale przy zmianie `LessEqual`
                // w przyszłości — tak).
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
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

        // --- pipeline passu cieni ---
        //
        // Różni się od głównego w trzech miejscach:
        //   1. entry point `vs_shadow` zamiast `vs_main`,
        //   2. `fragment: None` — brak color attachmentu,
        //   3. inny layout bind grup (tylko uniform + modele).
        //
        // Układ wierzchołków MUSI być identyczny z głównym potokiem:
        // oba czytają ten sam `GpuMesh` z tym samym buforem, a layout
        // bufora jest częścią kontraktu z potokiem.
        let shadow_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Uran 3D Shadow Pipeline Layout"),
                bind_group_layouts: &[&shadow_bind_layout],
                push_constant_ranges: &[],
            });

        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Uran 3D Shadow Pipeline"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_shadow",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<crate::mesh::Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
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
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 24,
                            shader_location: 3,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x3,
                            offset: 32,
                            shader_location: 2,
                        },
                    ],
                }],
            },
            // `None` = brak stage'u fragmentowego. Wymagane przez
            // specyfikację, gdy nie ma color attachmentu.
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Culling musi być TAKIE SAME jak w głównym potoku.
                // Różnica dałaby cienie „od tyłu" dla obiektów, których
                // ściany są jednorodne — np. płoty i ściany stodoła.
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                // Depth bias na poziomie potoku: zależny od nachylenia
                // powierzchni, więc działa tam, gdzie statyczny bias
                // w shaderze nie wystarcza (skośne dachy, pochyłe ściany).
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            // 1: pass cienia nie używa MSAA — rysujemy samą głębokość.
            multisample: wgpu::MultisampleState {
                count: 1,
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
            depth_tex: None,
            depth_size: (0, 0),
            scene_buffer,
            models_buffer,
            models_capacity,
            bind_group,
            models_bind_layout: layout,
            postfx: PostFx::new(device, format, samples),
            pipeline,
            sky_pipeline,
            shadow_pipeline,
            meshes: Vec::new(),
            materials,
            shadow,
            shadow_bind_layout,
            shadow_bind_group,
            camera: Camera3d::default(),
            lighting: Lighting::default(),
            stylization: Stylization::default(),
            atmosphere: Atmosphere::default(),
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

    /// Globalna skala rysunkowości sceny (do edycji przez grę).
    pub fn stylization_mut(&mut self) -> &mut Stylization {
        &mut self.stylization
    }

    /// Niebo i mgła (do edycji przez grę).
    pub fn atmosphere_mut(&mut self) -> &mut Atmosphere {
        &mut self.atmosphere
    }

    /// Kolor czyszczenia (niebo / mgła).
    pub fn set_clear_color(&mut self, c: [f32; 4]) {
        self.clear_color = c;
    }

    /// Ustawienia post-processingu (tonemapping, bloom, kontury, DoF…).
    ///
    /// `postfx` jest prywatne, bo jego wnętrze (tekstury, potoki,
    /// bind grupy) nie jest częścią API sceny. Same `PostSettings`
    /// są za to w pełni publiczne — wystarczy jedno `&mut`, żeby gra
    /// mogła zmienić dowolny parametr w locie, np. otworzyć
    /// przysłonę przy celowaniu.
    ///
    /// Osobno od `lighting_mut`/`stylization_mut`, bo te ustawienia
    /// wchodzą do bufora sceny, a post-processing liczy je dopiero
    /// przy kompozycji. Mieszanie obu w jednym `&mut` wymuszałoby
    /// wybór: albo blokowanie całej sceny na potrzeby pojedynczej
    /// liczby, albo dodawanie osobnych setterów na każdy parametr.
    pub fn post_settings_mut(&mut self) -> &mut PostSettings {
        self.postfx.settings_mut()
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

    /// Rejestruje materiał PBR (tekstury + parametry) i zwraca jego
    /// identyfikator.
    ///
    /// `gpu` podajemy jawnie z tego samego powodu co w [`Self::add_mesh`]:
    /// renderer nie trzyma `Queue` w polach. Wczytanie PNG-a wymaga
    /// kolejki, bo `write_texture` kopiuje piksele na kartę.
    pub fn add_material(
        &mut self,
        gpu: &uran_render::GpuContext,
        material: &crate::import::Material,
    ) -> MaterialId {
        self.materials.add(&gpu.device, &gpu.queue, material)
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

    /// Bufor głębokości pod aktualny rozmiar okna.
    ///
    /// Bufor głębokości musi mieć `sample_count` zgodny z potokiem —
    /// inaczej walidacja wgpu odrzuci pass z attachementem.
    fn with_depth(mut self, device: &wgpu::Device, width: u32, height: u32) -> Self {
        let (w, h) = (width.max(1), height.max(1));
        let texture = depth_texture(device, w, h, self.samples);
        self.depth = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
        self.depth_tex = Some(texture);
        self.depth_size = (w, h);
        self
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
            Some(self.shadow.view()),
            Some(self.shadow.sampler()),
        );
        // Pass cienia ma OSOBNĄ bind grupę wskazującą na tę samą tablicę
        // modeli — inaczej po powiększeniu bufora rysowałby z dziurą.
        self.shadow_bind_group = make_bind_group(
            device,
            &self.shadow_bind_layout,
            &self.scene_buffer,
            &self.models_buffer,
            None,
            None,
        );
    }

    /// AABB sceny liczone z macierzy modeli wszystkich komend.
    ///
    /// Bierzemy osiem narożników jednostkowego sześcianu (-1..1) i
    /// przekształcamy je każdą macierzą. `GpuMesh` NIE trzyma już
    /// geometrii na CPU (`upload` zostawia ją tylko na karcie), więc
    /// AABB liczymy z transformacji, a nie z wierzchołków.
    ///
    /// Jednostkowy sześcian zamiast bryły modelu daje **nadmiarowy**
    /// kadr: dopasowany do sześcianu, a nie do rzeczywistej bryły.
    /// To kosztuje rozdzielczość cienia (większy kadr na tę samą mapę),
    /// ale jest bezpieczne — nigdy nie wytniemy obiektu.
    fn bounds_from_cache(&self) -> SceneBounds {
        let mut b = SceneBounds::empty();
        for cmd in &self.commands {
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 { -1.0 } else { 1.0 },
                    if i & 2 == 0 { -1.0 } else { 1.0 },
                    if i & 4 == 0 { -1.0 } else { 1.0 },
                );
                b.expand(cmd.model.transform_point3(corner));
            }
        }
        b
    }

    /// Pozycja tarczy słońca w UV albo `None`, gdy jest poza kadrem.
    ///
    /// Promień do słońca rzutujemy tą samą macierzą `view_proj`, co
    /// geometrię, więc wynik jest zgodny z tym, co widać. Dodatkowo
    /// sprawdzamy, czy punkt leży PRZED kamerą (po stronie -Z): słońce
    /// za plecami dawałoby smugi wychodzące z krawędzi obrazu, bo
    /// `to_sun` w shaderze wskazywałoby w stronę odwrotną.
    fn sun_screen_uv(&self) -> Option<(f32, f32)> {
        // `light_dir` wskazuje OD źródła, więc pozycja tarczy leży
        // w kierunku `-light_dir`.
        let to_sun = -self.lighting.light_dir.normalize_or_zero();
        let world = self.camera.position + to_sun * 1000.0;
        let clip = self.camera.view_proj() * uran_math::Vec4::new(world.x, world.y, world.z, 1.0);
        // `w <= 0` = punkt za kamerą (po stronie +Z).
        if clip.w <= 1e-4 {
            return None;
        }
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        // Trzymamy się tu+1 zapasu: flara i promienie są szerokie,
        // więc tarcza tuż za krawędzią wciąż powinna dawać poświat.
        if !(-1.2..=1.2).contains(&ndc_x) || !(-1.2..=1.2).contains(&ndc_y) {
            return None;
        }
        // NDC -> UV. Oś Y odwrócona, bo w NDC rośnie w górę.
        Some(((ndc_x + 1.0) * 0.5, (1.0 - ndc_y) * 0.5))
    }

    /// Renderuje scenę: czyści kolor i głębokość, rysuje wszystkie obiekty.
    fn render_pass(&mut self, target: &Scene3dTarget<'_>) {
        let device = &target.gpu.device;
        let queue = &target.gpu.queue;
        let w = target.gpu.config.width.max(1);
        let h = target.gpu.config.height.max(1);
        if self.depth.is_none() || self.depth_size != (w, h) {
            self.depth_size = (w, h);
            let texture = depth_texture(device, w, h, self.samples);
            self.depth = Some(texture.create_view(&wgpu::TextureViewDescriptor::default()));
            self.depth_tex = Some(texture);
            self.camera.set_aspect(w as f32 / h as f32);
        }

        // Kadr cienia dobieramy do AABB CAŁEJ sceny (liczy go
        // `bounds_from_cache`). Bez tego obiekty poza kadrem nie
        // rzucałyby cienia wcale — wygląda to jak wycięcie budynku.
        let bounds = self.bounds_from_cache();
        self.shadow.fit(bounds, self.lighting.light_dir);

        let c = self.lighting.ambient;
        let s = &self.shadow.settings;
        let shadow_on = if s.enabled { 1.0 } else { 0.0 };
        // `view_proj` liczymy RAZ i dzielimy na trzy macierze, które
        // potrzebują różnych passów. Trzy osobne wywołania `view_proj()`
        // dałyby te same liczby, ale kosztowałyby trzy pełne mnożenia
        // macierzy na klatkę i groziłyby rozjazdem o 1 ULP.
        let view_proj = self.camera.view_proj();
        let uniform = SceneUniform {
            view_proj: view_proj.to_cols_array_2d(),
            // Odwrotność potrzebna niebu (odtworzenie promienia) i
            // post-processingu (odtworzenie pozycji z głębokości).
            // Przy poprawnej kamerze macierz jest zawsze odwracalna;
            // `glam` zwraca zera dla osobliwości, a wtedy `w` w shaderze
            // daje 0 i dzielenie zostawia NaN — dlatego shader nieba
            // normalizuje różnicę punktów, nie ich współrzędnych.
            inv_view_proj: view_proj.inverse().to_cols_array_2d(),
            view: self.camera.view().to_cols_array_2d(),
            eye: [
                self.camera.position.x,
                self.camera.position.y,
                self.camera.position.z,
                self.camera.tan_half_fov(),
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
                // `w` to natężenie — wcześniej było tu 0.0, przez co
                // shader PBR liczył słońce o natężeniu zerowym
                self.lighting.intensity,
            ],
            ambient_time: [c[0], c[1], c[2], self.time],
            light_view_proj: self.shadow.light_view_proj().to_cols_array_2d(),
            shadow_params: [s.depth_bias, s.normal_offset, s.strength, s.radius],
            shadow_map_info: [self.shadow.texel_uv(), shadow_on, 0.0, 0.0],
            // Near/far w jednym miejscu: od nich zależą liniaryzacja
            // głębokości w postfx i rekonstrukcja pozycji w SSR.
            screen: [w as f32, h as f32, self.camera.near, self.camera.far],
            stylize: [
                self.stylization.amount,
                self.stylization.softness,
                self.stylization.sss,
                0.0,
            ],
            atmos: [
                self.atmosphere.fog_density,
                self.atmosphere.fog_color[0],
                self.atmosphere.fog_color[1],
                self.atmosphere.fog_color[2],
            ],
            // `x` steruje zanikaniem mgły ku górze. Gdy 0, shader
            // używa jednolitej gęstości i zachowuje się jak wcześniej.
            fog: [self.atmosphere.fog_height, 0.0, 0.0, 0.0],
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
        // post-processing: cel HDR i tekstury zależne od rozmiaru okna
        self.postfx.ensure_size(device, w, h);
        self.postfx.upload_params(
            queue,
            w,
            h,
            self.time,
            self.sun_screen_uv(),
            self.camera.near,
            self.camera.far,
            self.camera.tan_half_fov(),
            self.camera.aspect,
        );

        // --- pass cieni: sama głębokość z pozycji słońca ---
        //
        // Musi być PRZED passem głównym, bo shader sceny czyta tę mapę.
        // Bind grupy jest OSOBNA (bez mapy cieni), bo wgpu nie pozwala
        // w tym samym passie czytać zasobu, który jest jego celem.
        if self.shadow.settings.enabled {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Uran 3D Shadow Pass"),
                // Pusty kolor: `color_attachments` musi być pustą listą,
                // nie listą z `None` — pass bez color attachmentu.
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: self.shadow.view(),
                    // 1.0 = „nic nie blokuje" — dalej od słońca znaczy
                    // większa głębokość, więc 1 to maksimum.
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.shadow_pipeline);
            pass.set_bind_group(0, &self.shadow_bind_group, &[]);

            // Ten sam bufor modeli co w passie głównym, więc indeks
            // instancji (`base_instance`) musi się zgadzać: `instance_index`
            // w WGSL liczy od 1, bo `first_instance` to i+1.
            for (i, cmd) in self.commands.iter().enumerate() {
                let Some(mesh) = self.meshes.get(cmd.mesh.0 as usize) else {
                    continue;
                };
                mesh.draw(&mut pass, 0, i as u32 + 1);
            }
        }

        {
            // klonujemy widok głębokości, żeby pożyczka `&mut pass` nie
            // kolidowała z pożyczką `&mut self` w pętli rysowania
            // pożyczka widoku głębokości trwa tylko do końca passu
            let depth_view = self.depth.as_ref().expect("depth utworzony wyżej");
            // Dwóch celów kolorowych w kolejności zgodnej z `targets`
            // w pipeline głównym. Nie wolno ich zamienić miejscami —
            // wgpu odrzuci pass, a komunikat błędu nie mówi wprost,
            // o które chodzi.
            let hdr_view = self.postfx.hdr_view();
            let gbuf_view = self.postfx.gbuffer_view();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Uran 3D Pass"),
                color_attachments: &[
                    Some(wgpu::RenderPassColorAttachment {
                        // Rysujemy do celu HDR, NIE na powierzchnię. Na ekran
                        // wrzuca nas dopiero `postfx.composite` (tonemapping,
                        // bloom, AA) — bez tego poświaty obciąłoby 8 bitów.
                        view: hdr_view,
                        resolve_target: None,
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
                    }),
                    Some(wgpu::RenderPassColorAttachment {
                        // G-Bufer: normalna w przestrzeni oka + chropowatość.
                        // Czyścimy na czarno, co po liniaryzacji głębokości
                        // daje „tło nieba" (normalna = 0, chropowatość = 0,
                        // czyli najgładziej) — a niebo i tak nadpisuje ten
                        // cel swoim własnym kolorem, więc w praktyce
                        // wartość czyszczenia nie jest tu krytyczna.
                        view: gbuf_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 0.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    }),
                ],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // --- niebo: PIERWSZE, zanim jakakolwiek bryła ---
            //
            // Kolejność jest tu istotna. Niebo ma wyłączony zapis
            // głębokości i porównanie `LessEqual`, więc rysowane później
            // nadpisałoby kolor na bryłach (wygrałoby, bo 1.0 <= głębokość
            // bryły). Rysowane wcześniej — wypełnia tło i zostaje
            // zasłonięte przez bryły, które mają mniejszą głębokość.
            if self.atmosphere.sky {
                pass.set_pipeline(&self.sky_pipeline);
                // Ta sama grupa 0 co dla brył: uniform sceny z kierunkiem
                // słońca i macierzą `inv_view_proj`.
                pass.set_bind_group(0, &self.bind_group, &[]);
                // 3 wierzchołki = pełny prostokąt, bez bufora wierzchołków.
                pass.draw(0..3, 0..1);
            }

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);

            let mut drawn = 0usize;
            for (i, cmd) in self.commands.iter().enumerate() {
                let Some(mesh) = self.meshes.get(cmd.mesh.0 as usize) else {
                    // zły identyfikator — cicho pomijamy, bo walidacja
                    // wgpu nie zna indeksów naszych siatek
                    continue;
                };
                // Bind grupy 1 = materiał. Musi być ustawiona PRZED
                // draw, bo to osobny stan potoku: poprzednia komenda
                // zostawiłaby tu swój materiał i obiekt zostałby
                // pomalowany cudzym kolorem.
                if let Some(bg) = self.materials.bind_group(cmd.material) {
                    pass.set_bind_group(1, bg, &[]);
                }
                // `base_instance` = i+1, bo WGSL liczy `instance_index` od 1
                mesh.draw(&mut pass, 0, i as u32 + 1);
                drawn += 1;
            }
            self.last_draw_count = drawn;
        }

        // --- kopia głębokości dla post-processingu ---
        //
        // MUSI być poza blokiem powyżej: w tym samym passie nie wolno
        // czytać tekstury, która jest jego celem. Kopia do osobnej
        // tekstury (z `COPY_DST`) jest już legalna, bo tamten pass
        // się skończył.
        //
        // Głębokość jest potrzebna dla: konturów (krawędzie obiektów),
        // DoF (odległość), SSR (kolidencja promienia) i mgły w passie
        // kompozycji. Bez niej te efekty miałyby zgadywać geometrię
        // z samego koloru.
        if self.samples == 1 {
            // Przy MSAA kopia byłaby niemożliwa (głębokość
            // multisamplingowa nie da się skopiować jako 2D), więc
            // efekty ekranowe po prostu nie włączamy. Gry 3D ustawiają
            // `.samples(1)`, więc ścieżka jest domyślna.
            if let Some(tex) = self.depth_tex.as_ref() {
                self.postfx.copy_depth(&mut encoder, tex);
            }
        }

        // post-processing na powierzchnię: bloom (3 passy) + kompozycja
        self.postfx.composite(&mut encoder, target);

        queue.submit(Some(encoder.finish()));
    }
}

/// Bufor głębokości sceny.
///
/// Osobna funkcja, bo głębokość tworzymy w dwóch miejscach
/// (`with_depth` przy starcie i `render_pass` przy resize) i obie
/// ścieżki muszą zgadzać się co do formatu oraz `sample_count` —
/// rozjazd kończy się odrzuceniem potoku przez walidację, bez
/// wskazania, która z dwóch ścieżek się rozjechała.
fn depth_texture(device: &wgpu::Device, w: u32, h: u32, samples: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Uran 3D Depth"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        // `COPY_SRC` jest potrzebne do skopiowania głębokości dla
        // post-processingu (`PostFx::copy_depth`). Bez tego flagi
        // wgpu odrzuci `copy_texture_to_texture` dopiero w trakcie
        // renderowania.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
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

/// Bind grupa grupy 0: uniform sceny (0) + tablica modeli (1) +
/// mapa cieni (2) + próbnik porównawczy (3).
///
/// Budujemy ją z **nazwy** layoutu zamiast z pozycji wpisów, bo
/// `create_bind_group` wymaga tylu wpisów, ile jest w layoutcie. Layout
/// passu cienia ma tylko 2, więc przekazanie pustego `TextureView`
/// w `Option` pozwala obsłużyć oba przypadki jedną funkcją.
fn make_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    scene: &wgpu::Buffer,
    models: &wgpu::Buffer,
    shadow_tex: Option<&wgpu::TextureView>,
    shadow_samp: Option<&wgpu::Sampler>,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: scene.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: models.as_entire_binding(),
        },
    ];
    // Dodajemy 2 i 3 TYLKO gdy layout je deklaruje. Wpisania pustego
    // `Option` do bind grupy wgpu odrzuci z „binding not found".
    if let (Some(tex), Some(samp)) = (shadow_tex, shadow_samp) {
        entries.push(wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(tex),
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 3,
            resource: wgpu::BindingResource::Sampler(samp),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Uran 3D Bind Group"),
        layout,
        entries: &entries,
    })
}

impl Scene3d for Renderer3d {
    fn draw(&mut self, target: &Scene3dTarget<'_>) {
        self.render_pass(target);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.camera.set_aspect(width as f32 / height.max(1) as f32);
        // zerujemy `depth_size` i `sized`, żeby `with_depth` oraz
        // `postfx.ensure_size` odtworzyły tekstury w nowym rozmiarze
        self.depth_size = (0, 0);
        self.postfx.invalidate();
    }

    fn is_active(&self) -> bool {
        true
    }
}
