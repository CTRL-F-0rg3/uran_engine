//! Główny renderer: zamienia `DrawList` na komendy GPU.

use std::collections::HashMap;
use std::sync::Arc;

use uran_asset::AssetServer;
use uran_math::{Mat3, Rect, Vec2};
use wgpu::util::DeviceExt;
use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::backend::device::{GpuContext, RenderError};
use crate::backend::pipeline::{
    create_sampler, create_texture_bind_group_layout, quad_indices, unit_quad_vertices,
    PipelineCache,
};
use crate::backend::texture::TextureRegistry;
use crate::batch::{
    DrawList, Globals, MeshGeometry, MeshPushConstants, SpriteDraw, SpriteInstance, TextureKey,
};
use crate::camera::Camera2d;
use crate::compute::GpuSim;
use crate::text::FontRegistry;

/// Statystyki pojedynczej klatki (do HUD-a i debugu).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FrameStats {
    /// Liczba sprite'ów (w tym glifów tekstu).
    pub sprites: usize,
    /// Liczba siatek.
    pub meshes: usize,
    /// Faktyczne draw calle — to jest liczba, która boli, gdy rośnie.
    pub draw_calls: usize,
    /// Liczba instancji przesłanych do GPU.
    pub instances: usize,
    /// Tekstury wgrane w tej klatce (0 = wszystko już w pamięci).
    pub textures_uploaded: usize,
}

impl std::fmt::Display for FrameStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} sprite'ów, {} siatek, {} draw calli, {} instancji",
            self.sprites, self.meshes, self.draw_calls, self.instances
        )
    }
}

/// Bufor geometrii siatki trzymany na GPU.
struct GpuMesh {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    last_used_frame: u64,
}

/// Renderer 2D oparty o instancjonowany batching.
pub struct Renderer {
    gpu: GpuContext,
    pipelines: PipelineCache,
    textures: TextureRegistry,
    fonts: FontRegistry,

    /// Wierzchołki i indeksy jednostkowego kwadratu (tworzone raz).
    quad_vertex_buffer: wgpu::Buffer,
    quad_index_buffer: wgpu::Buffer,
    quad_index_count: u32,

    /// Bufor danych instancji (rośnie w miarę potrzeb).
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,

    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    /// Osobna macierz dla przestrzeni ekranu (HUD/UI) — inna projekcja,
    /// więc inny uniform i inna bind groupa.
    screen_globals_buffer: wgpu::Buffer,
    screen_globals_bind_group: wgpu::BindGroup,

    /// Cache siatek po haszu geometrii.
    meshes: HashMap<u64, GpuMesh>,
    /// Tekstura MSAA (gdy `samples > 1`).
    msaa_texture: Option<wgpu::TextureView>,

    /// Zliczacz klatek do czyszczenia nieużywanych siatek.
    frame_index: u64,
    /// Ile klatek siatka może być nieużywana, zanim ją zwolnimy.
    mesh_ttl: u64,
    /// Symulacja jednostek na GPU (pozycje nigdy nie wracają na CPU).
    sim: Option<GpuSim>,
    /// Scena 3D rysowana przed passem 2D (patrz `scene3d`).
    ///
    /// Obiekt należy do gry — tu trzymamy tylko `Box<dyn Scene3d>`,
    /// żeby nie wymuszać zależności `uran-render -> uran-render3d`.
    scene3d: Option<Box<dyn crate::scene3d::Scene3d>>,
    /// Zgłoszenie zrzutu ekranu (patrz `Renderer::request_screenshot`).
    screenshot: Option<ScreenshotRequest>,
    /// Post-processing 2D (rybie oko). `None` = zasoby GPU jeszcze nie
    /// utworzone — gra nie włączyła post-processingu i nie płacimy nawet
    /// za skompilowanie shadera.
    post: Option<crate::postfx::PostFx>,
    /// Ostatnie ustawienia post-processingu.
    post_settings: crate::PostFxSettings,
    /// Czy atlas czcionki został już zrzucony do pliku (URAN_DUMP_ATLAS).
    atlas_dumped: bool,
    pub stats: FrameStats,
}

/// Żądanie zapisu klatki do pliku PNG.
pub struct ScreenshotRequest {
    pub path: std::path::PathBuf,
    /// Po ilu klatkach zapisać (0 = następna).
    pub frames: u32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>, vsync: bool, samples: u32) -> Result<Self, RenderError> {
        let gpu = GpuContext::new(window, vsync, samples).await?;
        let device = &gpu.device;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran 2D Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        // sampler i layout bind grupy tworzymy raz i współdzielimy między
        // potokami a rejestrem tekstur (w wgpu 0.19 nie implementują `Clone`)
        let pipelines_layout = create_texture_bind_group_layout(device);
        let sampler = create_sampler(device);

        let pipelines = PipelineCache::new(
            device,
            gpu.config.format,
            samples,
            &shader,
            &pipelines_layout,
            &sampler,
        );
        let textures = TextureRegistry::new(
            device,
            &gpu.queue,
            pipelines_layout,
            sampler,
            gpu.is_srgb_surface(),
        );

        let quad_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uran Quad Vertices"),
            contents: bytemuck::cast_slice(&unit_quad_vertices()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uran Quad Indices"),
            contents: bytemuck::cast_slice(&quad_indices()),
            usage: wgpu::BufferUsages::INDEX,
        });

        let globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind_group = pipelines.globals_bind_group(device, &globals_buffer);

        let screen_globals_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Screen Globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let screen_globals_bind_group =
            pipelines.globals_bind_group(device, &screen_globals_buffer);

        let instance_capacity = 1024;
        // bufor instancji tworzymy przed przeniesieniem `gpu` — inaczej
        // pożyczka `&gpu.device` żyłaby dłużej niż samo `gpu`
        let instance_buffer = create_instance_buffer(device, instance_capacity);

        Ok(Self {
            gpu,
            pipelines,
            textures,
            fonts: FontRegistry::new(),
            quad_vertex_buffer,
            quad_index_buffer,
            quad_index_count: 6,
            instance_buffer,
            instance_capacity,
            globals_buffer,
            globals_bind_group,
            screen_globals_buffer,
            screen_globals_bind_group,
            meshes: HashMap::new(),
            msaa_texture: None,
            frame_index: 0,
            mesh_ttl: 600,
            sim: None,
            scene3d: None,
            screenshot: None,
            post: None,
            post_settings: crate::PostFxSettings::default(),
            atlas_dumped: false,
            stats: FrameStats::default(),
        })
    }

    /// Zaplanuje zrzut bieżącej klatki do pliku PNG.
    ///
    /// Przydatne do testów „czy coś się w ogóle rysuje" oraz do menu
    /// „zrób zrzut ekranu" w grze.
    pub fn request_screenshot(&mut self, path: impl Into<std::path::PathBuf>, frames: u32) {
        self.screenshot = Some(ScreenshotRequest {
            path: path.into(),
            frames,
        });
    }

    /// Ustawia post-processing 2D (na razie: delikatne rybie oko).
    ///
    /// Zasoby GPU są tworzone przy pierwszym włączeniu, więc bezpiecznie
    /// można wołać tę metodę w systemie startupowym albo co klatkę.
    /// Wyłączenie (`enabled = false`) nie zwalnia zasobów — pass po prostu
    /// przestaje być wykonywany, a ponowne włączenie jest natychmiastowe.
    ///
    /// Gdy post-processing jest wyłączony (domyślny stan), renderer
    /// renderuje dokładnie tak jak wcześniej — zero zmian w klatce.
    pub fn set_post_fx(&mut self, settings: crate::PostFxSettings) {
        let settings = settings.sanitized();
        if settings.enabled && self.post.is_none() {
            self.post = Some(crate::postfx::PostFx::new(
                &self.gpu.device,
                self.pipelines.format,
            ));
        }
        self.post_settings = settings;
    }

    /// Bieżące ustawienia post-processingu.
    pub fn post_fx(&self) -> crate::PostFxSettings {
        self.post_settings
    }

    /// Włącza symulację jednostek na GPU.
    ///
    /// Bufor na `capacity` jednostek alokowany jest raz. Od tego momentu
    /// pozycje żyją wyłącznie na karcie: CPU wysyła w klatce 96 B
    /// parametrów, a pozycje czyta shader wierzchołkowy.
    pub fn enable_gpu_sim(&mut self, capacity: usize) {
        let sim = GpuSim::new(
            &self.gpu,
            capacity,
            self.pipelines.format,
            self.pipelines.samples,
        );
        self.sim = Some(sim);
    }

    /// Wyłącza symulację GPU (zwalnia bufory).
    pub fn disable_gpu_sim(&mut self) {
        self.sim = None;
    }

    /// Czy symulacja GPU jest włączona.
    pub fn has_gpu_sim(&self) -> bool {
        self.sim.is_some()
    }

    /// Kolejka GPU (do jednorazowego wgrywania buforów symulacji).
    pub fn queue(&self) -> &wgpu::Queue {
        &self.gpu.queue
    }

    /// Kontekst GPU — z niego renderer 3D (`uran-render3d`) bierze
    /// `Device` i `Queue`, żeby nie tworzyć drugiego urządzenia.
    ///
    /// Dwa `Device` na jedno okno to dwa niezależne stany wgpu: osobne
    /// bufory nie widzą się nawzajem i nie ma jak współdzielić powierzchni.
    pub fn gpu(&self) -> &crate::backend::device::GpuContext {
        &self.gpu
    }

    /// Format powierzchni — potok 3D musi się zgadzać z 2D, inaczej
    /// `RenderPassColorAttachment` zostanie odrzucony przez walidację.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.pipelines.format
    }

    /// Ile próbek na piksel (1 = bez MSAA). Renderer 3D musi użyć tej
    /// samej wartości w depth bufferze, co 2D w załączniku koloru.
    pub fn samples(&self) -> u32 {
        self.pipelines.samples
    }

    /// Podpina scenę 3D rysowaną przed passem 2D.
    ///
    /// Scena należy do gry (tu trafia tylko `Box`), dzięki czemu
    /// `uran-render` nie zależy od `uran-render3d`.
    pub fn set_scene3d(&mut self, scene: Box<dyn crate::scene3d::Scene3d>) {
        let (w, h) = self.size();
        let mut scene = scene;
        scene.resize(w, h);
        self.scene3d = Some(scene);
    }

    /// Zdejmuje scenę 3D (zwraca do gry, np. przy zmianie poziomu).
    pub fn take_scene3d(&mut self) -> Option<Box<dyn crate::scene3d::Scene3d>> {
        self.scene3d.take()
    }

    /// Czy podpięto scenę 3D.
    pub fn has_scene3d(&self) -> bool {
        self.scene3d.is_some()
    }

    /// Wskrzesza jednostki w zakresie indeksów, stawiając je w `at`.
    ///
    /// Wgrywamy tylko te sloty (48 B na jednostkę), a nie całą armię.
    pub fn revive_sim_range(
        &mut self,
        from: usize,
        to: usize,
        at: uran_math::Vec2,
        team: crate::Team,
    ) {
        if let Some(sim) = &mut self.sim {
            sim.revive_range(&self.gpu.queue, from..to, at, team);
        }
    }

    /// Wgrywa armię na GPU. Pozycje zostają już tylko na karcie.
    pub fn upload_sim_units(&mut self, units: &[crate::GpuUnit]) {
        if let Some(sim) = &mut self.sim {
            sim.upload_units(&self.gpu.queue, units);
        }
    }

    /// Symulator jednostek (do parametrów, wgrywania armii, statystyk).
    pub fn sim(&self) -> Option<&GpuSim> {
        self.sim.as_ref()
    }

    pub fn sim_mut(&mut self) -> Option<&mut GpuSim> {
        self.sim.as_mut()
    }

    /// Rozmiar okna w fizycznych pikselach.
    pub fn size(&self) -> (u32, u32) {
        self.gpu.size()
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.gpu.resize(size);
        self.msaa_texture = None; // trzeba odtworzyć w nowym rozmiarze
        let (w, h) = self.size();
        if let Some(scene) = &mut self.scene3d {
            // scena 3D trzyma własny depth buffer — musi go odtworzyć,
            // bo rozmiar bufora głębokości jest zapisany na sztywno
            scene.resize(w, h);
        }
    }

    /// Czy podany prostokąt świata widać (odrzucanie pracy poza ekranem).
    pub fn is_visible(&self, camera: &Camera2d, aabb: uran_math::Rect) -> bool {
        let (w, h) = self.size();
        camera
            .visible_rect(Vec2::new(w as f32, h as f32))
            .intersects(&aabb)
    }

    /// Rozszerza listę o tekst: zamienia `TextDraw` na sprite'y glifów.
    ///
    /// Robimy to **przed** sortowaniem, żeby glify brały udział w batchowaniu
    /// razem z resztą sceny.
    pub fn prepare_text(&mut self, list: &mut DrawList, assets: &AssetServer) {
        if list.texts().is_empty() {
            return;
        }
        let texts = list.texts().to_vec();
        let mut sprites = list.sprites().to_vec();
        for text in &texts {
            if let Some(data) = assets.font(text.font) {
                let _ = self.fonts.load(text.font, &data.data);
            }
            self.fonts
                .layout(text, Mat3::from_translation(text.position), &mut sprites);
        }
        list.replace_sprites(sprites);
        list.clear_texts();
    }

    /// Wgrywa atlasy czcionek, które zmieniły się od ostatniej klatki.
    pub fn sync_font_atlases(&mut self, fonts_used: &[uran_asset::HandleId]) {
        if std::env::var("URAN_DUMP_ATLAS").is_ok() && !self.atlas_dumped {
            for id in fonts_used {
                if let Some(data) = self.fonts.atlas_data(*id) {
                    let size = crate::text::ATLAS_SIZE;
                    let mut rgba = Vec::with_capacity(data.len() * 4);
                    for &c in data {
                        // pokrycie -> czarno-biały obraz dla oka
                        rgba.extend_from_slice(&[c, c, c, 255]);
                    }
                    if let Ok(img) = uran_asset::Image::from_rgba(size, size, rgba) {
                        let path = format!("/tmp/uran_atlas_{}.png", id.index());
                        let _ = img.save_png(std::path::Path::new(&path));
                        eprintln!("[uran] zapisano atlas czcionki: {path}");
                    }
                }
            }
            self.atlas_dumped = true;
        }
        for key in fonts_used {
            if !self.fonts.is_dirty(*key) {
                continue;
            }
            if let Some(data) = self.fonts.atlas_data(*key) {
                // `data` to wycinek rejestru, a `upload_atlas` potrzebuje
                // dostępu do `self` — kopiujemy
                let data = data.to_vec();
                self.textures.upload_atlas(
                    &self.gpu.device,
                    TextureKey::FontAtlas(*key),
                    crate::text::ATLAS_SIZE,
                    &data,
                    &self.gpu.queue,
                );
                self.fonts.mark_clean(*key);
            }
        }
    }

    /// Rozwijanie 9-slice: 9 kafelków w zależności od rozmiaru obrazu.
    ///
    /// Robimy to przed sortowaniem, bo renderer zna rozmiar tekstury, którego
    /// nie ma w `DrawList`.
    pub fn expand_nine_slices(&mut self, list: &mut DrawList, assets: &AssetServer) {
        if list.nine_slices().is_empty() {
            return;
        }
        let slices = list.nine_slices().to_vec();
        let mut sprites = list.sprites().to_vec();

        for slice in slices {
            let image_size = assets
                .image(slice.texture)
                .map(|i| Vec2::new(i.width as f32, i.height as f32))
                .unwrap_or(Vec2::ONE);

            let dst = slice.dst;
            let size = dst.size();
            // margines w pikselach przeliczamy na UV i na piksele docelowe
            let bx = (slice.border / image_size.x).clamp(0.0, 0.5);
            let by = (slice.border / image_size.y).clamp(0.0, 0.5);
            let px = (slice.border).min(size.x * 0.5);
            let py = (slice.border).min(size.y * 0.5);

            // (u0, v0, u1, v1) dla 3x3 kafelków w kolejności: L-D, Ś-D, P-D,
            // L-Ś, Ś-Ś, P-Ś, L-G, Ś-G, P-G
            let src = [
                (0.0, 0.0, bx, by),
                (bx, 0.0, 1.0 - bx, by),
                (1.0 - bx, 0.0, 1.0, by),
                (0.0, by, bx, 1.0 - by),
                (bx, by, 1.0 - bx, 1.0 - by),
                (1.0 - bx, by, 1.0, 1.0 - by),
                (0.0, 1.0 - by, bx, 1.0),
                (bx, 1.0 - by, 1.0 - bx, 1.0),
                (1.0 - bx, 1.0 - by, 1.0, 1.0),
            ];
            let targets = [
                Rect::new(dst.min, dst.min + Vec2::new(px, py)),
                Rect::new(
                    Vec2::new(dst.min.x + px, dst.min.y),
                    Vec2::new(dst.max.x - px, dst.min.y + py),
                ),
                Rect::new(
                    Vec2::new(dst.max.x - px, dst.min.y),
                    Vec2::new(dst.max.x, dst.min.y + py),
                ),
                Rect::new(
                    Vec2::new(dst.min.x, dst.min.y + py),
                    Vec2::new(dst.min.x + px, dst.max.y - py),
                ),
                Rect::new(
                    Vec2::new(dst.min.x + px, dst.min.y + py),
                    Vec2::new(dst.max.x - px, dst.max.y - py),
                ),
                Rect::new(
                    Vec2::new(dst.max.x - px, dst.min.y + py),
                    Vec2::new(dst.max.x, dst.max.y - py),
                ),
                Rect::new(
                    Vec2::new(dst.min.x, dst.max.y - py),
                    Vec2::new(dst.min.x + px, dst.max.y),
                ),
                Rect::new(
                    Vec2::new(dst.min.x + px, dst.max.y - py),
                    Vec2::new(dst.max.x - px, dst.max.y),
                ),
                Rect::new(
                    Vec2::new(dst.max.x - px, dst.max.y - py),
                    Vec2::new(dst.max.x, dst.max.y),
                ),
            ];

            for ((u0, v0, u1, v1), target) in src.iter().zip(targets) {
                let mut sprite = SpriteDraw::rect(
                    target,
                    slice.transform,
                    uran_ecs::UvRect::new(Vec2::new(*u0, *v0), Vec2::new(*u1, *v1)),
                    slice.color,
                );
                sprite.texture = TextureKey::Image(slice.texture.into());
                sprite.blend = slice.blend;
                sprite.layer = slice.layer;
                sprite.screen_space = slice.screen_space;
                sprites.push(sprite);
            }
        }
        list.replace_sprites(sprites);
        list.clear_nine_slices();
    }

    /// Renderuje jedną klatkę. Lista zostanie posortowana (zmienia kolejność
    /// elementów wewnątrz `list`).
    pub fn render(
        &mut self,
        list: &mut DrawList,
        camera: &Camera2d,
        clear_color: uran_math::Color,
        assets: &AssetServer,
    ) -> FrameStats {
        self.frame_index += 1;
        self.textures.begin_frame();

        // 1) tekst -> sprite'y, 9-slice -> 9 prostokątów, potem sortowanie
        self.expand_nine_slices(list, assets);
        self.prepare_text(list, assets);
        list.sort();

        // 2) atlasy czcionek, których używamy w tej klatce
        let mut fonts_used: Vec<uran_asset::HandleId> = Vec::new();
        for sprite in list.sprites() {
            if let TextureKey::FontAtlas(id) = sprite.texture {
                if !fonts_used.contains(&id) {
                    fonts_used.push(id);
                }
            }
        }
        self.sync_font_atlases(&fonts_used);

        // 3) instancje sprite'ów w kolejności rysowania
        let sprites = list.sprites();
        let capacity = (sprites.len().max(1) * 2).next_power_of_two();
        if capacity > self.instance_capacity {
            self.instance_buffer = create_instance_buffer(&self.gpu.device, capacity);
            self.instance_capacity = capacity;
        }
        if !sprites.is_empty() {
            let instances: Vec<SpriteInstance> = sprites
                .iter()
                .map(|s| {
                    // Projekcja ekranowa ma oś Y skierowaną w dół, więc górna
                    // krawędź kwadratu ląduje na dole ekranu. Bez zamiany V
                    // tekst i obrazy w HUD-ie byłyby odwrócone pionowo.
                    let uv = if s.screen_space { s.uv.flip_y() } else { s.uv };
                    SpriteInstance::new(s.transform, uv, s.color)
                })
                .collect();
            self.gpu
                .queue
                .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        }

        // 3) faza przygotowania (potrzebuje &mut): wgrywamy tekstury i tworzymy
        // bufory siatek, żeby w pętli rysującej móc korzystać z &self
        for sprite in list.sprites() {
            self.textures
                .ensure_bound(&self.gpu.device, sprite.texture, assets, &self.gpu.queue);
        }
        for mesh in list.meshes() {
            if let Some(geo) = list.geometry().get(mesh.geometry as usize) {
                self.ensure_mesh(geo);
            }
        }

        // 4) macierz świata -> NDC oraz macierz ekranu (dla HUD-u)
        let (width, height) = self.gpu.size();
        let window_size = Vec2::new(width as f32, height as f32);
        let globals = Globals::new(camera.view_projection(window_size));
        let screen_globals = Globals::new(crate::camera::screen_matrix(window_size));
        self.gpu
            .queue
            .write_buffer(&self.globals_buffer, 0, bytemuck::bytes_of(&globals));
        self.gpu.queue.write_buffer(
            &self.screen_globals_buffer,
            0,
            bytemuck::bytes_of(&screen_globals),
        );

        if std::env::var("URAN_DEBUG").is_ok() && self.frame_index <= 1 {
            eprintln!("[uran-render] okno={window_size:?}");
            eprintln!("[uran-render]   świat view_proj={:?}", globals.view_proj);
            eprintln!(
                "[uran-render]   ekran view_proj={:?}",
                screen_globals.view_proj
            );
        }

        // 5) render pass
        let surface_texture = match self.gpu.surface.get_current_texture() {
            Ok(texture) => texture,
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.gpu
                    .surface
                    .configure(&self.gpu.device, &self.gpu.config);
                return FrameStats::default();
            }
            Err(wgpu::SurfaceError::Timeout) => return FrameStats::default(),
            Err(e) => {
                eprintln!("⚠️  błąd powierzchni: {e}");
                return FrameStats::default();
            }
        };

        let view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let clear = clear_color.to_linear();
        // `wgpu::TextureView` nie implementuje `Clone`, a potrzebujemy
        // pożyczenia widoku MSAA bez trzymania pożyczki `&mut self` na czas
        // wywołań rysujących. Dlatego „wypożyczamy" go z `Option`
        // i wkładamy z powrotem na końcu klatki.
        let mut msaa = self.msaa_texture.take();
        if msaa.is_none() && self.pipelines.samples > 1 {
            msaa = Some(self.create_msaa_view());
        }

        // --- post-processing: klatka idzie NAJPIERW na teksturę pośrednią --
        // Dopiero po passie 2D zamieniamy ją na obraz końcowy (patrz niżej
        // „post-processing 2D"). Przy wyłączonym efekcie zachowanie jest
        // dokładnie takie jak wcześniej: scena trafia prosto na powierzchnię.
        if self.post_settings.enabled {
            if let Some(post_fx) = self.post.as_mut() {
                let (w, h) = self.gpu.size();
                let device = &self.gpu.device;
                post_fx.ensure_size(device, w.max(1), h.max(1));
            }
        }
        let post_active = self.post_settings.is_active() && self.post.is_some();
        let scene_view = if post_active {
            self.post.as_ref().map(|p| p.scene_view())
        } else {
            None
        };

        let (attachment, resolve) = match (msaa.as_ref(), scene_view) {
            // MSAA + post: resolve trafia do tekstury pośredniej, a ta
            // dopiero potem na ekran.
            (Some(msaa_view), Some(scene)) => (msaa_view, Some(scene)),
            // MSAA bez post — zachowanie historyczne.
            (Some(msaa_view), None) => (msaa_view, Some(msaa_view)),
            // Bez MSAA, z post: scena renderuje się od razu do pośredniej.
            (None, Some(scene)) => (scene, None),
            // Bez MSAA i bez post — prosto na powierzchnię okna.
            (None, None) => (&view, None),
        };

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Uran Encoder"),
            });

        // 5a) symulacja jednostek na GPU — compute w tym samym encoderze,
        // więc w tej samej klatce render pass widzi już nowe pozycje
        if let Some(sim) = &mut self.sim {
            sim.poll_stats();
            sim.step(&mut encoder, &self.gpu.queue);
        }

        let mut stats = FrameStats {
            sprites: sprites.len(),
            meshes: list.meshes().len(),
            ..Default::default()
        };

        // --- scena 3D: OSOBNY render pass, własny depth buffer ----------
        // Idzie PRZED passem 2D i czyści kolor + głębokość. 2D dostaje potem
        // `LoadOp::Load`, więc farma trafia na wierzech świata 3D (HUD,
        // celownik, panele), a nie pod niego. Gdy 3D nie ma, 2D czyści sam.
        let scene3d_active = self.scene3d.as_ref().is_some_and(|s| s.is_active());
        if scene3d_active {
            let target = crate::scene3d::Scene3dTarget {
                color: attachment,
                resolve,
                format: self.pipelines.format,
                samples: self.pipelines.samples,
                gpu: &self.gpu,
            };
            if let Some(scene) = self.scene3d.as_mut() {
                scene.draw(&target);
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Uran Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: attachment,
                    resolve_target: resolve,
                    ops: wgpu::Operations {
                        // 3D zdążyło już wyrysować kadr — nie wolno go
                        // wyczyścić, bo zniknęłaby cała scena.
                        load: if scene3d_active {
                            wgpu::LoadOp::Load
                        } else {
                            wgpu::LoadOp::Clear(wgpu::Color {
                                r: clear.r as f64,
                                g: clear.g as f64,
                                b: clear.b as f64,
                                a: clear.a as f64,
                            })
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // 2D nie potrzebuje głębi: kolejność zależy od listy rysowania
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Kolejność ma znaczenie dla czytelności:
            //   1. armia z GPU (jest „pod" resztą sceny),
            //   2. świat: sprite'y i siatki z listy rysowania,
            //   3. HUD w przestrzeni ekranu.
            //
            // Armia musi być PRZED sprite'ami świata, inaczej 100k kółek
            // zasłoniłoby teren; HUD zostaje na wierzchu, bo narysowany
            // sprite'ami 2D i nie ma głębi.
            if let Some(sim) = &self.sim {
                let world_globals = &self.globals_bind_group;
                if let Some(n) = sim.draw(&mut pass, world_globals) {
                    stats.draw_calls += 1;
                    stats.instances += n;
                }
            }
            // faza rysowania — obie metody tylko odczytują stan renderera
            stats.draw_calls += self.draw_sprites(&mut pass, list.sprites(), &mut stats);
            stats.draw_calls += self.draw_meshes(&mut pass, list);
        }

        // --- post-processing 2D: scena z tekstury pośredniej na ekran -----
        // Pass zamienia klatkę na obraz końcowy z efektem rybiego oka.
        // Musi być w tym samym encoderze co scena — wtedy wgpu sam wstawi
        // barierę pamięci między zapisem a odczytem tekstury.
        if post_active {
            if let Some(post_fx) = self.post.as_ref() {
                let (w, h) = self.gpu.size();
                let aspect = if h > 0 { w as f32 / h as f32 } else { 1.0 };
                post_fx.upload_params(&self.gpu.queue, self.post_settings.fisheye, aspect);
                post_fx.draw(&mut encoder, &view);
            }
        }

        self.gpu.queue.submit(Some(encoder.finish()));

        // Zmapowanie bufora statystyk DOPIERO po submicie — wcześniej
        // walidacja wgpu odrzuca submit, bo bufor byłby zmapowany, a dopiero
        // potem miał zostać zapisany przez `copy_buffer_to_buffer`.
        if let Some(sim) = &mut self.sim {
            sim.after_submit();
        }

        // Callback z `map_async` wywołuje wgpu dopiero przy `poll`. Bez tego
        // statystyki nigdy nie dotarłyby z GPU i HUD pokazywałby zera.
        // `Poll` nie czeka na GPU — tylko obsługuje callbacki, więc
        // nie kosztuje przyszłego klatki.
        self.gpu.device.poll(wgpu::Maintain::Poll);

        if std::env::var("URAN_DEBUG").is_ok() && self.frame_index <= 1 {
            eprintln!("[uran-render] okno={window_size:?}");
            eprintln!("[uran-render]   świat  view_proj={:?}", globals.view_proj);
            eprintln!(
                "[uran-render]   ekran  view_proj={:?}",
                screen_globals.view_proj
            );
            for (i, s) in list.sprites().iter().take(4).enumerate() {
                let c = s.transform.to_cols_array_2d();
                eprintln!(
                    "[uran-render]   sprite[{i}] tekstura={:?} ekran={} macierz kolumny={:?}",
                    s.texture, s.screen_space, c
                );
            }
        }

        // zrzut ekranu — kopiujemy zanim klatka zniknie z swapchainu
        if let Some(request) = self.screenshot.as_mut() {
            if request.frames == 0 {
                let path = request.path.clone();
                self.save_screenshot(&surface_texture, &path);
                self.screenshot = None;
            } else {
                request.frames -= 1;
            }
        }

        surface_texture.present();

        // oddajemy widok MSAA do renderera
        self.msaa_texture = msaa;
        self.cleanup_unused_meshes();
        self.stats = stats;
        stats
    }

    /// Tworzy teksturę multisamplingową w aktualnym rozmiarze okna.
    fn create_msaa_view(&self) -> wgpu::TextureView {
        let (width, height) = self.gpu.size();
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Uran MSAA"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: self.pipelines.samples,
            dimension: wgpu::TextureDimension::D2,
            format: self.pipelines.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Rysuje sprite'y: instancje leżą już w kolejności rysowania, więc
    /// wystarczy rozciąć listę na przebiegi o jednakowej (tekstura, tryb).
    ///
    /// Wymaga wcześniejszego `ensure_bound` dla użytych tekstur (faza
    /// przygotowania) — wtedy tu wystarczy niemutowalny dostęp.
    fn draw_sprites<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        sprites: &[SpriteDraw],
        stats: &mut FrameStats,
    ) -> usize {
        if sprites.is_empty() {
            return 0;
        }
        stats.instances = sprites.len();
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.quad_index_buffer.slice(..), wgpu::IndexFormat::Uint32);

        let mut draw_calls = 0;
        let mut run_start = 0usize;
        while run_start < sprites.len() {
            let first = &sprites[run_start];
            let mut run_end = run_start + 1;
            while run_end < sprites.len() {
                let next = &sprites[run_end];
                // `screen_space` musi rozdzielać przebiegi: HUD i scena mają
                // różne macierze, więc bez tego warunek cały HUD dostałby
                // macierz świata (albo odwrotnie) i pojechałby w złym miejscu.
                if next.texture != first.texture
                    || next.blend != first.blend
                    || next.screen_space != first.screen_space
                {
                    break;
                }
                run_end += 1;
            }

            pass.set_pipeline(self.pipelines.sprite(first.blend));
            if first.screen_space {
                pass.set_bind_group(1, &self.screen_globals_bind_group, &[]);
            } else {
                pass.set_bind_group(1, &self.globals_bind_group, &[]);
            }
            pass.set_bind_group(0, self.textures.get_bound(first.texture), &[]);
            pass.draw_indexed(
                0..self.quad_index_count,
                0,
                run_start as u32..run_end as u32,
            );
            draw_calls += 1;
            run_start = run_end;
        }
        draw_calls
    }

    /// Rysuje siatki — każda to osobny draw call (mają własną geometrię).
    ///
    /// Wymaga wcześniejszego `ensure_mesh` (faza przygotowania).
    fn draw_meshes<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, list: &DrawList) -> usize {
        let geometry = list.geometry();
        let mut draw_calls = 0;

        for mesh in list.meshes() {
            let Some(geo) = geometry.get(mesh.geometry as usize) else {
                continue;
            };
            if geo.indices.is_empty() {
                continue;
            }
            let Some(gpu_mesh) = self.meshes.get(&geo.hash) else {
                continue;
            };

            let push = MeshPushConstants::new(mesh.transform, mesh.color);
            pass.set_pipeline(self.pipelines.mesh(mesh.blend));
            // HUD rysowany z siatek (np. ikony UI) musi dostać macierz ekranu
            if mesh.screen_space {
                pass.set_bind_group(1, &self.screen_globals_bind_group, &[]);
            } else {
                pass.set_bind_group(1, &self.globals_bind_group, &[]);
            }
            pass.set_bind_group(0, self.textures.get_bound(mesh.texture), &[]);
            pass.set_push_constants(wgpu::ShaderStages::VERTEX, 0, bytemuck::bytes_of(&push));
            pass.set_vertex_buffer(0, gpu_mesh.vertex_buffer.slice(..));
            pass.set_index_buffer(gpu_mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..gpu_mesh.index_count, 0, 0..1);
            draw_calls += 1;
        }
        draw_calls
    }

    /// Tworzy bufory GPU dla geometrii, jeśli jeszcze ich nie ma (faza
    /// przygotowania; `last_used_frame` odświeżamy tu, bo w pętli rysującej
    /// mamy tylko `&self`).
    fn ensure_mesh(&mut self, geometry: &MeshGeometry) {
        let frame = self.frame_index;
        if let Some(mesh) = self.meshes.get_mut(&geometry.hash) {
            mesh.last_used_frame = frame;
            return;
        }
        // Kolory wierzchołków konwertujemy sRGB -> liniowe TUTAJ, przy
        // uploadzie. Dzięki temu shader nie musi tego robić, a tint
        // (który `MeshPushConstants::new` też konwertuje) mnoży już
        // liniowe wartości — inaczej siatki wychodzą niemal czarne.
        let mut vertices = geometry.vertices.clone();
        for v in &mut vertices {
            let c = uran_math::Color::rgba(v.color[0], v.color[1], v.color[2], v.color[3]);
            v.color = c.to_linear().to_array();
        }
        let vertex_buffer = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Uran Mesh Vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let index_buffer = self
            .gpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Uran Mesh Indices"),
                contents: bytemuck::cast_slice(&geometry.indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.meshes.insert(
            geometry.hash,
            GpuMesh {
                vertex_buffer,
                index_buffer,
                index_count: geometry.indices.len() as u32,
                last_used_frame: frame,
            },
        );
    }

    /// Zwalnia siatki nieużywane od dawna (rosnący ślad w cache'u).
    fn cleanup_unused_meshes(&mut self) {
        if self.meshes.len() < 256 {
            return;
        }
        let frame = self.frame_index;
        let ttl = self.mesh_ttl;
        self.meshes
            .retain(|_, mesh| frame.saturating_sub(mesh.last_used_frame) < ttl);
    }

    /// Zapisuje bieżącą klatkę swapchainu do pliku PNG.
    ///
    /// Czyta piksele z GPU i zamienia format powierzchni (może być BGRA)
    /// na RGBA, którego oczekuje `image`.
    fn save_screenshot(&self, surface: &wgpu::SurfaceTexture, path: &std::path::Path) {
        let (width, height) = self.gpu.size();
        let format = self.gpu.config.format;
        let bytes_per_row = width * 4;
        // `write_buffer`/kopiowanie wymaga wiersza wyrównanego do 256 bajtów
        let padded = bytes_per_row.div_ceil(256) * 256;
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Screenshot"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Screenshot Copy"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::ImageCopyTexture {
                texture: &surface.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::ImageCopyBuffer {
                buffer: &buffer,
                layout: wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.gpu.queue.submit(Some(encoder.finish()));

        // mapowanie bufora jest asynchroniczne — blokujemy wątek, bo
        // zrzut ekranu nie jest operacją hot pathu
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.gpu.device.poll(wgpu::Maintain::Wait);
        if rx.recv().is_err() {
            eprintln!("⚠️  nie udało się zmapować bufora zrzutu");
            return;
        }

        let data = slice.get_mapped_range();
        // zdekodowanie wierszy wyrównanych do 256 bajtów + ewentualna zamiana
        // BGRA -> RGBA
        let bgra = format == wgpu::TextureFormat::Bgra8Unorm
            || format == wgpu::TextureFormat::Bgra8UnormSrgb;
        let mut rgba = Vec::with_capacity((bytes_per_row * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            let line = &data[start..start + bytes_per_row as usize];
            if bgra {
                rgba.extend(line.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]));
            } else {
                rgba.extend_from_slice(line);
            }
        }
        drop(data);
        buffer.unmap();

        match uran_asset::Image::from_rgba(width, height, rgba) {
            Ok(image) => match image.save_png(path) {
                Ok(()) => println!("📸 zapisano zrzut ekranu: {}", path.display()),
                Err(e) => eprintln!("⚠️  nie udało się zapisać PNG: {e}"),
            },
            Err(e) => eprintln!("⚠️  nie udało się zbudować obrazu: {e}"),
        }
    }

    /// Zwalnia wszystkie zasoby GPU siatek.
    pub fn clear_mesh_cache(&mut self) {
        self.meshes.clear();
    }
}

/// Bufor danych instancji tworzony wraz z buforem i doraźnie przy rozroście.
fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Uran Sprite Instances"),
        size: (std::mem::size_of::<SpriteInstance>() * capacity.max(1)) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
