//! Jednostka symulowana na GPU.
//!
//! Rozmiar i kolejność pól muszą bit w bit zgadzać się ze strukturą
//! `Unit` w `sim.wgsl` — inaczej GPU czyta śmieci (ciche, trudne do
//! zdiagnozowania błędy). Dlatego obie strony mają testy rozmiaru/offsetów.

use bytemuck::{Pod, Zeroable};
use uran_math::Vec2;

/// Drużyna (jako zwykły u32 w bitach flagi — WGSL nie ma typów bool).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum Team {
    /// Jednostki gracza (np. piechota sojusznicza).
    Friendly = 0,
    /// Jednostki przeciwnika.
    Enemy = 1,
}

impl Team {
    pub const fn from_u32(v: u32) -> Self {
        match v {
            0 => Team::Friendly,
            _ => Team::Enemy,
        }
    }

    /// Bit w `GpuUnit::flags` opisujący drużynę (bit 1).
    pub const fn bit(self) -> u32 {
        match self {
            Team::Friendly => 0,
            Team::Enemy => 1 << 1,
        }
    }
}

/// Pojedyncza jednostka w `storage` bufferze.
///
/// Kolejność pól = kolejność w WGSL (48 bajtów, wyrównane do 16).
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct GpuUnit {
    /// Pozycja w świecie.
    pub position: [f32; 2],
    /// Prędkość (compute ją aktualizuje co klatkę).
    pub velocity: [f32; 2],
    /// Punkt, do którego zmierzy jednostka (polecenie gracza).
    pub target: [f32; 2],
    /// Punkty życia. <= 0 oznacza martwą jednostkę.
    pub health: f32,
    /// Odliczanie do następnego ataku.
    pub cooldown: f32,
    /// `UnitFlags` (ALIVE, drużyna, ATTACKED).
    pub flags: u32,
    /// Ziarno losowości — kolor i rozmiar, żeby armia nie była jednolicie płaska.
    pub seed: u32,
    pub _pad: [f32; 2],
}

impl GpuUnit {
    /// Nowa żywa jednostka w drużynie `team`.
    pub fn spawn(position: Vec2, target: Vec2, team: Team, health: f32, seed: u32) -> Self {
        Self {
            position: position.to_array(),
            velocity: [0.0; 2],
            target: target.to_array(),
            health,
            cooldown: 0.0,
            flags: UnitFlags::alive().with_team(team).0,
            seed,
            _pad: [0.0; 2],
        }
    }

    pub fn pos(&self) -> Vec2 {
        Vec2::from_array(self.position)
    }

    pub fn target_pos(&self) -> Vec2 {
        Vec2::from_array(self.target)
    }

    pub fn vel(&self) -> Vec2 {
        Vec2::from_array(self.velocity)
    }

    pub fn flags(&self) -> UnitFlags {
        UnitFlags(self.flags)
    }

    pub fn is_alive(&self) -> bool {
        self.flags().is_alive()
    }

    pub fn team(&self) -> Team {
        self.flags().team()
    }

    pub fn set_position(&mut self, p: Vec2) {
        self.position = p.to_array();
    }
}

/// Bity `GpuUnit::flags`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnitFlags(pub u32);

impl UnitFlags {
    /// Jednostka żyje. Wyłączony bit = martwa, nie rysowana.
    pub const ALIVE: u32 = 1 << 0;
    /// Jednostka w tej klatce zadała obrażenia.
    pub const ATTACKED: u32 = 1 << 2;

    pub const fn empty() -> Self {
        UnitFlags(0)
    }

    pub const fn alive() -> Self {
        UnitFlags(Self::ALIVE)
    }

    pub const fn dead() -> Self {
        UnitFlags(0)
    }

    pub const fn with_alive(self, alive: bool) -> Self {
        if alive {
            UnitFlags(self.0 | Self::ALIVE)
        } else {
            UnitFlags(self.0 & !Self::ALIVE)
        }
    }

    pub const fn is_alive(self) -> bool {
        self.0 & Self::ALIVE != 0
    }

    pub const fn team(self) -> Team {
        Team::from_u32((self.0 >> 1) & 1)
    }

    pub const fn with_team(self, team: Team) -> Self {
        UnitFlags(self.0 | team.bit())
    }

    pub const fn attacked(self) -> bool {
        self.0 & Self::ATTACKED != 0
    }

    pub const fn with_attacked(self, attacked: bool) -> Self {
        if attacked {
            UnitFlags(self.0 | Self::ATTACKED)
        } else {
            UnitFlags(self.0 & !Self::ATTACKED)
        }
    }
}

/// Parametry symulacji na jedną klatkę (uniform buffer).
///
/// Kolejność pól = kolejność w WGSL. Rozmiar 96 B (wielokrotność 16,
/// wymagana przez `uniform` bufory).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct SimParams {
    /// Lewy dolny róg areny (jednostki świata).
    pub arena_min: [f32; 2],
    /// Prawy górny róg areny.
    pub arena_max: [f32; 2],
    /// Liczba komórek siatki w osi X i Y.
    pub grid_dim: [u32; 2],
    /// Liczba aktywnych jednostek (sloty powyżej są wyłączone).
    pub count: u32,
    /// Komórki na oś kwadratowej siatki.
    pub cells_per_axis: u32,
    /// Ile jednostek mieści się w jednej komórze.
    pub max_per_cell: u32,
    pub _pad0: u32,

    /// Delta czasu w sekundach.
    pub dt: f32,
    /// Czas skumulowany (do animacji).
    pub time: f32,
    /// Pozycja gracza — przyciąganie i cel dla wrogów.
    pub player_pos: [f32; 2],
    /// Punkt zgrupowania (dowodzenie piechotą).
    pub rally: [f32; 2],

    /// Maksymalna prędkość jednostki.
    pub max_speed: f32,
    /// Siła odpychania od sąsiadów.
    pub separation: f32,
    /// Promień odpychania.
    pub separation_radius: f32,
    /// Zasięg ataku.
    pub attack_range: f32,
    /// Obrażenia za strzał.
    pub attack_damage: f32,
    /// Czas między strzałami.
    pub attack_cooldown: f32,
    /// Promień jednostki w jednostkach świata.
    pub unit_size: f32,
    /// Padding: bez niego `#[repr(C)]` daje 92 B, a WGSL wymaga rozmiaru
    /// będącego wielokrotnością 16 (96 B). Nieużywane, zawsze 0.
    pub _pad1: f32,
}

impl Default for SimParams {
    fn default() -> Self {
        Self {
            arena_min: [0.0; 2],
            arena_max: [0.0; 2],
            grid_dim: [1, 1],
            count: 0,
            cells_per_axis: 1,
            max_per_cell: 1,
            _pad0: 0,
            dt: 0.0,
            time: 0.0,
            player_pos: [0.0; 2],
            rally: [0.0; 2],
            max_speed: 100.0,
            separation: 200.0,
            separation_radius: 10.0,
            attack_range: 20.0,
            attack_damage: 10.0,
            attack_cooldown: 0.5,
            unit_size: 3.0,
            _pad1: 0.0,
        }
    }
}

impl SimParams {
    /// Dobiera siatkę przestrzenną dla danej areny i liczby jednostek.
    ///
    /// Cel: średnio `units_per_cell` jednostek na komórkę. Zbyt rzadka
    /// siatka gubi sąsiadów (jednostki nachodzą na siebie), zbyt gęsta
    /// marnuje pamięć i czas na zerowanie.
    pub fn for_arena(arena: uran_math::Rect, units: usize, units_per_cell: f32) -> Self {
        let size = arena.size().max(Vec2::splat(1.0));
        let n = (units as f32 / units_per_cell.max(1.0)).max(1.0);
        // kwadratowa komórka: cell = sqrt(area / n)
        let cell = (size.x * size.y / n).sqrt().max(1.0);
        let grid = [
            (size.x / cell).floor().max(1.0) as u32,
            (size.y / cell).floor().max(1.0) as u32,
        ];
        let mut p = Self::default();
        p.arena_min = arena.min.to_array();
        p.arena_max = arena.max.to_array();
        p.grid_dim = grid;
        p.cells_per_axis = grid[0].max(grid[1]);
        p.count = units as u32;
        p
    }

    /// Liczba komórek siatki (iloczyn wymiarów).
    pub fn cell_count(&self) -> u32 {
        self.grid_dim[0].saturating_mul(self.grid_dim[1])
    }
}

/// Statystyki odczytane z GPU (asynchroniczny readback).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SimStats {
    /// Żywe jednostki gracza.
    pub alive_friendly: u32,
    /// Żywe jednostki przeciwnika.
    pub alive_enemy: u32,
    /// Zabicia od początku symulacji.
    pub kills: u32,
    /// Strzały od początku symulacji.
    pub shots: u32,
    /// Ile klatek statystyk zdążyliśmy już odczytać.
    pub samples: u32,
}

impl SimStats {
    /// Łączna liczba żywych jednostek.
    pub fn total_alive(&self) -> u32 {
        self.alive_friendly + self.alive_enemy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn unit_layout_matches_wgsl() {
        // WGSL: position(0) velocity(8) target(16) health(24) cooldown(28)
        //        flags(32) seed(36) _pad(40)  => 48 B
        assert_eq!(size_of::<GpuUnit>(), 48);
        let u = GpuUnit::default();
        let base = &u as *const _ as usize;
        let off = |p: *const u8| p as usize - base;
        assert_eq!(off(&u.position[0] as *const f32 as *const u8), 0);
        assert_eq!(off(&u.velocity[0] as *const f32 as *const u8), 8);
        assert_eq!(off(&u.target[0] as *const f32 as *const u8), 16);
        assert_eq!(off(&u.health as *const f32 as *const u8), 24);
        assert_eq!(off(&u.cooldown as *const f32 as *const u8), 28);
        assert_eq!(off(&u.flags as *const u32 as *const u8), 32);
        assert_eq!(off(&u.seed as *const u32 as *const u8), 36);
    }

    #[test]
    fn unit_stride_is_16_byte_aligned() {
        // storage buffer: krok elementu musi być wielokrotnością 16,
        // inaczej naga wyliczy inne offsety niż Rust
        assert_eq!(size_of::<GpuUnit>() % 16, 0);
    }

    #[test]
    fn params_layout_matches_wgsl() {
        // WGSL: arena_min(0) arena_max(8) grid_dim(16) count(24) cells(28)
        //        max_per_cell(32) _pad(36) dt(40) time(44) player(48)
        //        rally(56) max_speed(64) separation(68) sep_radius(72)
        //        attack_range(76) attack_damage(80) cooldown(84) size(88)
        assert_eq!(size_of::<SimParams>(), 96);
        let p = SimParams::default();
        let base = &p as *const _ as usize;
        let off = |q: *const u8| q as usize - base;
        assert_eq!(off(&p.arena_min[0] as *const f32 as *const u8), 0);
        assert_eq!(off(&p.arena_max[0] as *const f32 as *const u8), 8);
        assert_eq!(off(&p.player_pos[0] as *const f32 as *const u8), 48);
        assert_eq!(off(&p.rally[0] as *const f32 as *const u8), 56);
        assert_eq!(off(&p.max_speed as *const f32 as *const u8), 64);
        assert_eq!(off(&p.unit_size as *const f32 as *const u8), 88);
        // padding końcowy — WGSL też musi go mieć
        assert_eq!(off(&p._pad1 as *const f32 as *const u8), 92);
    }

    #[test]
    fn params_buffer_is_uniform_compatible() {
        // `uniform` bufory wymagają rozmiaru będącego wielokrotnością 16
        assert_eq!(size_of::<SimParams>() % 16, 0);
        assert_eq!(size_of::<SimParams>(), 96, "WGSL liczy tyle samo bajtów");
        assert_eq!(align_of::<SimParams>(), 4);
    }

    #[test]
    fn team_uses_bit_one() {
        assert_eq!(Team::Friendly.bit(), 0);
        assert_eq!(Team::Enemy.bit(), 2);
        let f = UnitFlags::alive().with_team(Team::Friendly);
        let e = UnitFlags::alive().with_team(Team::Enemy);
        assert!(f.is_alive() && e.is_alive());
        assert_eq!(f.team(), Team::Friendly);
        assert_eq!(e.team(), Team::Enemy);
    }

    #[test]
    fn alive_bit_is_independent_of_team_bit() {
        let f = UnitFlags::alive().with_team(Team::Friendly);
        let dead = f.with_alive(false);
        assert!(!dead.is_alive());
        assert_eq!(dead.team(), Team::Friendly, "zmiana życia nie zgubiła drużyny");
    }

    #[test]
    fn attacked_bit_does_not_clobber_team() {
        let f = UnitFlags::alive().with_team(Team::Enemy);
        let a = f.with_attacked(true);
        assert!(a.attacked());
        assert_eq!(a.team(), Team::Enemy);
        assert!(a.is_alive());
        assert!(!a.with_attacked(false).attacked());
    }

    #[test]
    fn spawn_places_alive_unit() {
        let u = GpuUnit::spawn(Vec2::new(10.0, 20.0), Vec2::ZERO, Team::Enemy, 100.0, 7);
        assert_eq!(u.pos(), Vec2::new(10.0, 20.0));
        assert!(u.is_alive());
        assert_eq!(u.team(), Team::Enemy);
        assert_eq!(u.health, 100.0);
        assert_eq!(u.seed, 7);
    }

    #[test]
    fn grid_scales_with_unit_count() {
        let arena = uran_math::Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0));
        let few = SimParams::for_arena(arena, 1_000, 8.0);
        let many = SimParams::for_arena(arena, 100_000, 8.0);
        assert!(
            many.cell_count() > few.cell_count(),
            "więcej jednostek = gęstsza siatka ({} vs {})",
            many.cell_count(),
            few.cell_count()
        );
    }

    #[test]
    fn grid_never_degenerates_to_zero() {
        let arena = uran_math::Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0));
        let p = SimParams::for_arena(arena, 0, 8.0);
        assert!(p.grid_dim[0] >= 1 && p.grid_dim[1] >= 1);
        assert!(p.cell_count() >= 1, "zerowa siatka = brak oddzielania");
    }

    #[test]
    fn grid_covers_whole_arena() {
        let arena = uran_math::Rect::new(Vec2::new(-900.0, -560.0), Vec2::new(900.0, 560.0));
        let p = SimParams::for_arena(arena, 50_000, 8.0);
        // siatka musi obejmować całą arenę, inaczej jednostki uciekają
        // poza mapę i nie widzą sąsiadów
        assert!(p.grid_dim[0] >= 2 && p.grid_dim[1] >= 2);
    }

    #[test]
    fn stats_sum_both_teams() {
        let s = SimStats { alive_friendly: 3, alive_enemy: 4, ..Default::default() };
        assert_eq!(s.total_alive(), 7);
    }
}