//! Symulacja masywnych ilości jednostek na GPU (compute shader).
//!
//! Model: pozycje jednostek żyją **wyłącznie** w `storage` bufferze na karcie.
//! CPU wysyła w klatce jedną strukturę [`SimParams`] (96 B) i polecenia
//! (punkt zgrupowania, pozycja gracza). Renderer czyta pozycje w shaderze
//! wierzchołkowym, więc nie ma żadnego round-tripu pozycji na CPU.
//!
//! Koszt CPU rośnie z liczbą *systemów*, nie jednostek — przy 100k
//! jednostek gra wykonuje kilka tysięcy operacji na klatkę.
//!
//! ## Schemat klatki
//!
//! ```text
//! clear_counters -> clear_grid -> build_grid -> think -> resolve
//! ```
//!
//! `build_grid` wypełnia jednolitą siatkę przestrzenną (atomiki), dzięki
//! czemu `think` znajduje sąsiadów w 3x3 komórkach zamiast przeglądać
//! wszystkie N² par. To jest różnica między 100 a 100 000 jednostek.

use std::sync::mpsc::{Receiver, TryRecvError};

use wgpu::util::DeviceExt;

use super::unit::{GpuUnit, SimParams, SimStats, UnitFlags};
use crate::backend::device::GpuContext;
use crate::batch::Globals;

/// Ile jednostek mieści się w jednej komórce siatki.
///
/// Kompromis: więcej = lepsze odpychanie, ale większa pamięć i dłuższe
/// pętle w `think`. 8 daje ~32 KB na 1000 komórek.
pub const MAX_PER_CELL: u32 = 8;

/// Rozmiar grupy roboczej (musi być <= `max_compute_invocations_per_workgroup`).
const WORKGROUP: u32 = 64;

/// Ile klatek między odczytem statystyk z GPU.
///
/// Readback jest asynchroniczny, ale `map_async` i tak kosztuje; raz na
/// ~10 klatek w zupełności starczy na HUD, a HUD nie musi być co-klatkowy.
const STATS_INTERVAL: u64 = 10;

/// Parametry wyglądu armii (uniform dla potoku rysującego).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct UnitStyle {
    /// Promień jednostki w jednostkach świata.
    pub radius: f32,
    /// Grubość obwódki (informacyjna, shader używa SDF).
    pub edge: f32,
    /// Mnożnik przezroczystości.
    pub alpha: f32,
    pub _pad: f32,
    /// Kolory drużyn: [0] = gracz, [1] = przeciwnik, [2..4] = rezerwa.
    pub colors: [[f32; 4]; 4],
}

impl Default for UnitStyle {
    fn default() -> Self {
        Self {
            radius: 3.0,
            edge: 1.0,
            alpha: 1.0,
            _pad: 0.0,
            colors: [
                [0.30, 0.62, 1.0, 1.0],  // gracz — niebieski
                [1.0, 0.36, 0.32, 1.0],  // przeciwnik — czerwony
                [0.0; 4],
                [0.0; 4],
            ],
        }
    }
}

/// Liczniki atomowe czytane z GPU.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Counters {
    alive_friendly: u32,
    alive_enemy: u32,
    kills: u32,
    shots: u32,
}

/// Gotowy potok compute (po jednym na przebieg).
struct ComputePass {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
}

/// Stan odczytu statystyk — mapowanie bufora jest asynchroniczne.
enum StatsReadback {
    Idle,
    /// Wynik `map_async` czeka w kanale (nie blokujemy wątku głównego).
    Receiving(Receiver<std::result::Result<(), wgpu::BufferAsyncError>>),
}

/// Symulator jednostek na GPU.
pub struct GpuSim {
    params: SimParams,
    style: UnitStyle,
    capacity: usize,

    units_buffer: wgpu::Buffer,
    grid_buffer: wgpu::Buffer,
    damage_buffer: wgpu::Buffer,
    counters_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,
    style_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,

    clear_counters: ComputePass,
    clear_grid: ComputePass,
    build_grid: ComputePass,
    think: ComputePass,
    resolve: ComputePass,

    draw_bind_group: wgpu::BindGroup,
    draw_style_bind_group: wgpu::BindGroup,
    draw_pipeline: wgpu::RenderPipeline,

    stats: SimStats,
    readback: StatsReadback,
    frame: u64,
}

fn binding(idx: u32, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding: idx,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    }
}

fn bg_entry(idx: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding: idx,
        resource: buffer.as_entire_binding(),
    }
}

impl GpuSim {
    /// Tworzy symulator na `capacity` jednostek.
    ///
    /// Bufory alokowane są raz i nigdy nie rosną — dzięki temu w klatce
    /// nie ma ani jednej alokacji, co jest warunkiem płynności przy
    /// 100k jednostek.
    pub fn new(
        gpu: &GpuContext,
        capacity: usize,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let device = &gpu.device;
        let max_binding = device.limits().max_storage_buffer_binding_size as usize;
        let unit_bytes = capacity.max(1) * std::mem::size_of::<GpuUnit>();
        assert!(
            unit_bytes <= max_binding,
            "{capacity} jednostek wymaga {unit_bytes} B, a limit to {max_binding} B"
        );

        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;
        let units_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Units"),
            size: unit_bytes as u64,
            usage: storage,
            mapped_at_creation: false,
        });
        let damage_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Damage"),
            size: (capacity.max(1) * 4) as u64,
            usage: storage,
            mapped_at_creation: false,
        });
        let counters_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Counters"),
            size: std::mem::size_of::<Counters>() as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Readback"),
            size: std::mem::size_of::<Counters>() as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Params"),
            size: std::mem::size_of::<SimParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let style_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Style"),
            size: std::mem::size_of::<UnitStyle>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Siatka z zapasem na 256x256 komórek (~1 MB) — pozwala dołożyć
        // jednostek bez reallokacji i bez ryzyka przepełnienia bufora.
        let cells = 256 * 256;
        let grid_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uran Sim Grid"),
            size: (cells * (1 + MAX_PER_CELL) * 4) as u64,
            usage: storage,
            mapped_at_creation: false,
        });

        let sim_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Uran Sim Shader"),
            source: wgpu::ShaderSource::Wgsl(sim_shader_source().into()),
        });
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uran Sim BGL"),
            entries: &[
                binding(
                    0,
                    wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                ),
                binding(1, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                binding(2, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                binding(3, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
                binding(4, wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                }),
            ],
        });

        let compute_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Uran Sim Pipeline Layout"),
            bind_group_layouts: &[&compute_layout],
            push_constant_ranges: &[],
        });
        // `wgpu::BindGroup` nie implementuje `Clone` w wgpu 0.19, więc każdy
        // przebieg dostaje własną bind grupę (wspólny layout). Przebiegi
        // tworzymy PRZED przeniesieniem buforów do struktury — wtedy
        // pożyczki kończą się i bufory da się przenieść.
        let make_pass = |name: &str| ComputePass {
            pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(name),
                layout: Some(&compute_pl),
                module: &sim_module,
                entry_point: name,
            }),
            bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Uran Sim Bind Group"),
                layout: &compute_layout,
                entries: &[
                    bg_entry(0, &params_buffer),
                    bg_entry(1, &units_buffer),
                    bg_entry(2, &grid_buffer),
                    bg_entry(3, &damage_buffer),
                    bg_entry(4, &counters_buffer),
                ],
            }),
        };
        let p_clear_counters = make_pass("clear_counters");
        let p_clear_grid = make_pass("clear_grid");
        let p_build_grid = make_pass("build_grid");
        let p_think = make_pass("think");
        let p_resolve = make_pass("resolve");
        let draw = build_draw_pipeline(device, format, samples, &units_buffer, &style_buffer);

        Self {
            params: SimParams::default(),
            style: UnitStyle::default(),
            capacity,
            units_buffer,
            grid_buffer,
            damage_buffer,
            counters_buffer,
            params_buffer,
            style_buffer,
            readback_buffer,
            clear_counters: p_clear_counters,
            clear_grid: p_clear_grid,
            build_grid: p_build_grid,
            think: p_think,
            resolve: p_resolve,
            draw_bind_group: draw.bind_group,
            draw_style_bind_group: draw.style_bind_group,
            draw_pipeline: draw.pipeline,
            stats: SimStats::default(),
            readback: StatsReadback::Idle,
            frame: 0,
        }
    }

    // ------------------------------------------------------------- API

    /// Parametry symulacji (delta, arena, punkt zgrupowania...).
    pub fn params(&self) -> &SimParams {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut SimParams {
        &mut self.params
    }

    /// Wygląd armii (promień, kolory drużyn).
    pub fn style(&self) -> &UnitStyle {
        &self.style
    }

    pub fn style_mut(&mut self) -> &mut UnitStyle {
        &mut self.style
    }

    /// Ostatnie odczytane statystyki (mogą być o kilka klatek nieaktualne —
    /// to cena asynchronicznego readbacku).
    pub fn stats(&self) -> SimStats {
        self.stats
    }

    /// Pojemność bufora jednostek.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Ile jednostek aktualnie symuluje.
    pub fn unit_count(&self) -> u32 {
        self.params.count.min(self.capacity as u32)
    }

    /// Wgrywa całą armię. Wywoływane przy starcie / zmianie skali.
    ///
    /// Pozycje zostają potem już tylko na GPU — kolejne wywołania `step`
    /// nigdy nie dotykają tego bufora.
    pub fn upload_units(&mut self, queue: &wgpu::Queue, units: &[GpuUnit]) {
        let n = units.len().min(self.capacity);
        self.params.count = n as u32;
        if n > 0 {
            queue.write_buffer(&self.units_buffer, 0, bytemuck::cast_slice(&units[..n]));
        }
    }

    /// Podmienia pojedynczą jednostkę (np. wskrzeszenie jednego żołnierza).
    ///
    /// Koszt: jeden mały `write_buffer`, bez zmiany rozmiaru bufora.
    pub fn write_unit(&mut self, queue: &wgpu::Queue, index: usize, unit: GpuUnit) {
        if index >= self.capacity {
            return;
        }
        let offset = (index * std::mem::size_of::<GpuUnit>()) as u64;
        queue.write_buffer(&self.units_buffer, offset, bytemuck::bytes_of(&unit));
    }

    /// Wskrzesza jednostki w podanym zasięgu indeksów, stawiając je w `at`.
    ///
    /// Z GPU nie da się wiedzieć, które sloty są martwe, bez readbacku,
    /// więc gry rezerwują sobie zakresy indeksów na swoje oddziały
    /// (np. 0..N to wieczni poddani, N..2N to jednorazowy kontingent).
    pub fn revive_range(
        &mut self,
        queue: &wgpu::Queue,
        range: std::ops::Range<usize>,
        at: uran_math::Vec2,
        team: super::unit::Team,
    ) {
        for i in range {
            if i >= self.capacity {
                break;
            }
            let unit = GpuUnit {
                position: at.to_array(),
                velocity: [0.0; 2],
                target: self.params.rally,
                health: 100.0,
                cooldown: 0.0,
                flags: UnitFlags::alive().with_team(team).0,
                seed: i as u32,
                _pad: [0.0; 2],
            };
            self.write_unit(queue, i, unit);
        }
    }

    /// Wykonuje jeden krok symulacji.
    ///
    /// Kolejność ma znaczenie: `clear_counters` zeruje liczniki, które
    /// `resolve` dopiero potem zlicza, a `clear_grid` musi być przed
    /// `build_grid`. Wszystkie dispatche trafiają do **jednego** encodera,
    /// więc GPU wykonuje je jako jeden pakiet bez synchronizacji z CPU.
    pub fn step(&mut self, encoder: &mut wgpu::CommandEncoder, queue: &wgpu::Queue) {
        let count = self.unit_count();
        if count == 0 {
            return;
        }
        let units_groups = groups(count);
        let cells = self.params.cell_count();
        let grid_groups = groups(cells.saturating_mul(1 + MAX_PER_CELL));

        // parametry i styl lecą raz na klatkę (96 + 96 B)
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));
        queue.write_buffer(&self.style_buffer, 0, bytemuck::bytes_of(&self.style));

        // 1) statystyki i siatka
        dispatch(encoder, &self.clear_counters, 1);
        dispatch(encoder, &self.clear_grid, grid_groups);
        // 2) budowa siatki przestrzennej
        dispatch(encoder, &self.build_grid, units_groups);
        // 3) ruch, rój, celowanie
        dispatch(encoder, &self.think, units_groups);
        // 4) obrażenia i zgonu
        dispatch(encoder, &self.resolve, units_groups);

        self.frame += 1;
        if self.frame % STATS_INTERVAL == 0 {
            encoder.copy_buffer_to_buffer(
                &self.counters_buffer,
                0,
                &self.readback_buffer,
                0,
                std::mem::size_of::<Counters>() as u64,
            );
            self.start_readback();
        }
    }

    /// Zaczyna asynchroniczny odczyt statystyk (nie blokuje wątku głównego).
    fn start_readback(&mut self) {
        if !matches!(self.readback, StatsReadback::Idle) {
            return; // poprzedni odczyt jeszcze w locie
        }
        let slice = self.readback_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        self.readback = StatsReadback::Receiving(rx);
    }

    /// Odbiera wynik readbacku, jeśli już dotarł. Wołane raz na klatkę.
    pub fn poll_stats(&mut self) {
        if let StatsReadback::Receiving(rx) = &self.readback {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    let view = self.readback_buffer.slice(..).get_mapped_range();
                    let c: Counters = *bytemuck::from_bytes(&view);
                    drop(view);
                    self.readback_buffer.unmap();
                    self.stats = SimStats {
                        alive_friendly: c.alive_friendly,
                        alive_enemy: c.alive_enemy,
                        kills: c.kills,
                        shots: c.shots,
                        samples: self.stats.samples + 1,
                    };
                    self.readback = StatsReadback::Idle;
                }
                Ok(Err(_)) => {
                    self.readback = StatsReadback::Idle;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.readback = StatsReadback::Idle;
                }
            }
        }
    }

    /// Rysuje wszystkie jednostki jednym `draw_indexed`.
    ///
    /// `globals_bind_group` musi wskazywać na macierz świata; pozycje
    /// czytane są wprost z bufora jednostek w shaderze wierzchołkowym.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        globals_bind_group: &'a wgpu::BindGroup,
    ) -> Option<usize> {
        let count = self.unit_count();
        if count == 0 {
            return None;
        }
        pass.set_pipeline(&self.draw_pipeline);
        pass.set_bind_group(0, globals_bind_group, &[]);
        pass.set_bind_group(1, &self.draw_style_bind_group, &[]);
        pass.set_bind_group(2, &self.draw_bind_group, &[]);
        // 6 indeksów na jednostkę; martwe zwijamy do zdegenerowanego trójkąta
        pass.draw_indexed(0..6, 0, 0..count);
        Some(count as usize)
    }
}

/// Liczba grup roboczych (zaokrąglamy w górę — ostatnia grupa może być
/// niepełna, a shader ogranicza się przez `if (i >= params.count)`).
fn groups(n: u32) -> u32 {
    n.div_ceil(WORKGROUP).max(1)
}

fn dispatch(encoder: &mut wgpu::CommandEncoder, pass: &ComputePass, groups: u32) {
    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
        label: Some("Uran Sim"),
        timestamp_writes: None,
    });
    cpass.set_pipeline(&pass.pipeline);
    cpass.set_bind_group(0, &pass.bind_group, &[]);
    cpass.dispatch_workgroups(groups, 1, 1);
}

/// Wynik budowy potoku rysującego jednostki.
struct DrawPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    style_bind_group: wgpu::BindGroup,
}

/// Potok rysujący jednostki: geometria w shaderze, pozycje w storage.
fn build_draw_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    samples: u32,
    units: &wgpu::Buffer,
    style: &wgpu::Buffer,
) -> DrawPipeline {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Uran Units Shader"),
        source: wgpu::ShaderSource::Wgsl(units_shader_source().into()),
    });

    let globals_layout = crate::backend::pipeline::create_globals_bind_group_layout(device);
    let style_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Uran Unit Style BGL"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    // `read_only: true` — w shaderze wierzchołkowym tylko czytamy pozycje
    let units_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Uran Unit Storage BGL"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Uran Unit Pipeline Layout"),
        bind_group_layouts: &[&globals_layout, &style_layout, &units_layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Uran Units Pipeline"),
        layout: Some(&pl),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: "vs_main",
            // geometria w shaderze (6 wierzchołków z `vertex_index`)
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: "fs_main",
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None, // jednostki bywają odwrócone
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState { count: samples, ..Default::default() },
        multiview: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Uran Unit Storage BG"),
        layout: &units_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: units.as_entire_binding(),
        }],
    });
    let style_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Uran Unit Style BG"),
        layout: &style_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: style.as_entire_binding(),
        }],
    });
    DrawPipeline { pipeline, bind_group, style_bind_group }
}

/// Wstawia stałą `MAX_PER_CELL` do WGSL i zwraca źródło shadera symulacji.
fn sim_shader_source() -> String {
    let src = include_str!("sim.wgsl");
    // Podstawienie zamiast `const` w WGSL: stała zależy od limitów
    // zdefiniowanych po stronie Rusta, więc musi być wstrzyknięta do tekstu.
    src.replace("MAX_PER_CELL", &MAX_PER_CELL.to_string())
}

fn units_shader_source() -> String {
    include_str!("units.wgsl").to_string()
}