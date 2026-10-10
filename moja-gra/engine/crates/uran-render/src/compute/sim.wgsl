// Symulacja jednostek na GPU. Schemat klatki (5 dispatche):
//
//   1. clear_counters — zerujemy liczniki statystyk
//   2. clear_grid     — zerujemy liczniki komórek
//   3. build_grid     — każda żywa jednostka wpisuje się do swojej komórki
//   4. think          — szuka wrogów w 3x3 komórkach, odsuwa się od sąsiadów,
//                      aktualizuje prędkość i pozycję
//   5. resolve        — nakłada zebrane obrażenia (atomowo), wyłącza martwe
//
// Pozycje nigdy nie wracają na CPU — jedynie cztery liczniki statystyk
// są odczytywane asynchronicznie raz na kilka klatek.
//
// Uwaga na nazwy pól: `target` i `from` są zarezerwowanymi słowami
// kluczowymi WGSL, dlatego struktury używają `goal` i `origin`.

struct Params {
    arena_min: vec2<f32>,
    arena_max: vec2<f32>,
    grid_dim: vec2<u32>,
    count: u32,
    cells_per_axis: u32,
    max_per_cell: u32,
    _pad0: u32,
    dt: f32,
    time: f32,
    player_pos: vec2<f32>,
    rally: vec2<f32>,
    max_speed: f32,
    separation: f32,
    separation_radius: f32,
    attack_range: f32,
    attack_damage: f32,
    attack_cooldown: f32,
    unit_size: f32,
    // bez tego WGSL liczy 92 B, a `uniform` wymaga wielokrotności 16
    _pad1: f32,
}

struct Unit {
    position: vec2<f32>,
    velocity: vec2<f32>,
    goal: vec2<f32>,
    health: f32,
    cooldown: f32,
    flags: u32,
    seed: u32,
    _pad: vec2<f32>,
}

struct Counters {
    alive_friendly: atomic<u32>,
    alive_enemy: atomic<u32>,
    kills: atomic<u32>,
    shots: atomic<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read_write> units: array<Unit>;
// siatka: dla komórki c słowo [c*STRIDE] to licznik, potem MAX_PER_CELL indeksów
@group(0) @binding(2) var<storage, read_write> grid: array<atomic<u32>>;
// obrażenia do zastosowania (atomowe — wielu strzelców może trafić
// w tę samą jednostkę)
@group(0) @binding(3) var<storage, read_write> damage: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> counters: Counters;

const ALIVE: u32 = 1u;
const TEAM_BIT: u32 = 2u;
const ATTACKED: u32 = 4u;

fn grid_stride() -> u32 { return 1u + MAX_PER_CELL; }
fn total_cells() -> u32 { return params.grid_dim.x * params.grid_dim.y; }
fn unit_alive(u: Unit) -> bool { return (u.flags & ALIVE) != 0u; }
fn unit_enemy(u: Unit) -> bool { return (u.flags & TEAM_BIT) != 0u; }

/// Indeks komórki siatki dla pozycji (z klamrowaniem do areny).
fn cell_of(p: vec2<f32>) -> u32 {
    let size = params.arena_max - params.arena_min;
    let t = (p - params.arena_min) / max(size, vec2<f32>(0.0001));
    let gx = min(u32(t.x * f32(params.grid_dim.x)), params.grid_dim.x - 1u);
    let gy = min(u32(t.y * f32(params.grid_dim.y)), params.grid_dim.y - 1u);
    return gy * params.grid_dim.x + gx;
}

/// Deterministyczny szum [0,1) z ziarna — miesza jednostki w komórce.
fn hash01(v: u32) -> f32 {
    var x = v * 747796405u + 2891336453u;
    x = ((x >> ((x >> 28u) + 4u)) ^ x) * 277803737u;
    x = (x >> 22u) ^ x;
    return f32(x & 0x00ffffffu) / 16777215.0;
}

/// Indeks najbliższego wroga w zasięgu, albo `0xFFFFFFFF`.
fn nearest_victim(origin: vec2<f32>, want_enemy: bool, range: f32) -> u32 {
    let c = cell_of(origin);
    let cx = c % params.grid_dim.x;
    let cy = c / params.grid_dim.x;
    var best = 0xFFFFFFFFu;
    var best_d2 = range * range;
    for (var oy = -1; oy <= 1; oy++) {
        let ny = i32(cy) + oy;
        if (ny < 0 || ny >= i32(params.grid_dim.y)) { continue; }
        for (var ox = -1; ox <= 1; ox++) {
            let nx = i32(cx) + ox;
            if (nx < 0 || nx >= i32(params.grid_dim.x)) { continue; }
            let base = (u32(ny) * params.grid_dim.x + u32(nx)) * grid_stride();
            let n = min(atomicLoad(&grid[base]), MAX_PER_CELL);
            for (var k = 0u; k < n; k++) {
                let j = atomicLoad(&grid[base + 1u + k]);
                if (j >= params.count) { continue; }
                let o = units[j];
                if (!unit_alive(o)) { continue; }
                if (unit_enemy(o) != want_enemy) { continue; }
                let delta = o.position - origin;
                let d2 = dot(delta, delta);
                if (d2 < best_d2) { best_d2 = d2; best = j; }
            }
        }
    }
    return best;
}

// --------------------------------------------------------- 1/2. zerowanie
@compute @workgroup_size(1)
fn clear_counters() {
    atomicStore(&counters.alive_friendly, 0u);
    atomicStore(&counters.alive_enemy, 0u);
}

@compute @workgroup_size(64)
fn clear_grid(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= total_cells() * grid_stride()) { return; }
    atomicStore(&grid[i], 0u);
}

// ---------------------------------------------------------------- 3. build
@compute @workgroup_size(64)
fn build_grid(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    let u = units[i];
    if (!unit_alive(u)) { return; }

    let base = cell_of(u.position) * grid_stride();
    let slot = atomicAdd(&grid[base], 1u);
    // przepełniona komórka: gubimy nadmiar (akceptowalne — to tylko
    // odpychanie i wybór celu, a nie dokładne kolizje)
    if (slot < MAX_PER_CELL) {
        atomicStore(&grid[base + 1u + slot], i);
    }
}

// ---------------------------------------------------------------- 4. think
@compute @workgroup_size(64)
fn think(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    var u = units[i];
    if (!unit_alive(u)) { return; }

    let enemy = unit_enemy(u);
    if (enemy) {
        atomicAdd(&counters.alive_enemy, 1u);
    } else {
        atomicAdd(&counters.alive_friendly, 1u);
    }

    // --- cel: najbliższy wróg w zasięgu, inaczej punkt zwołania
    var goal = params.rally;
    let victim = nearest_victim(u.position, !enemy, params.attack_range);
    if (victim != 0xFFFFFFFFu && victim < params.count) {
        goal = units[victim].position;
        if (u.cooldown <= 0.0) {
            atomicAdd(&damage[victim], u32(params.attack_damage * 1000.0));
            atomicAdd(&counters.shots, 1u);
            u.cooldown = params.attack_cooldown;
            u.flags = u.flags | ATTACKED;
        }
    }

    // --- ruch w stronę celu
    var dir = goal - u.position;
    let dist = length(dir);
    if (dist > 0.001) { dir = dir / dist; } else { dir = vec2<f32>(0.0); }

    // --- odpychanie od sąsiadów z 3x3 komórek
    var push_dir = vec2<f32>(0.0);
    let c = cell_of(u.position);
    let cx = c % params.grid_dim.x;
    let cy = c / params.grid_dim.x;
    for (var oy = -1; oy <= 1; oy++) {
        let ny = i32(cy) + oy;
        if (ny < 0 || ny >= i32(params.grid_dim.y)) { continue; }
        for (var ox = -1; ox <= 1; ox++) {
            let nx = i32(cx) + ox;
            if (nx < 0 || nx >= i32(params.grid_dim.x)) { continue; }
            let base = (u32(ny) * params.grid_dim.x + u32(nx)) * grid_stride();
            let n = min(atomicLoad(&grid[base]), MAX_PER_CELL);
            for (var k = 0u; k < n; k++) {
                let j = atomicLoad(&grid[base + 1u + k]);
                if (j == i || j >= params.count) { continue; }
                let delta = u.position - units[j].position;
                let d2 = dot(delta, delta);
                if (d2 > 0.00001 && d2 < params.separation_radius * params.separation_radius) {
                    push_dir = push_dir - delta * (1.0 / d2);
                }
            }
        }
    }
    // szum: jednostki nie zjeżdżają się w jeden punkt
    let jitter = (hash01(u.seed) - 0.5) * 0.25;
    let ang = hash01(u.seed ^ 0x9E3779B9u) * 6.2831853;
    let wobble = vec2<f32>(cos(ang), sin(ang)) * jitter;

    // Odsuwanie NORMALIZUJEMY. Surowa suma `1/d2` rośnie bez ograniczeń
    // w gęstej kolumnie i przy 40k żołnierzy dosłownie „wystrzeliwuje"
    // wszystkich z formacji — armia rozlewa się po mapie. Po normalizacji
    // odpychanie ma stałą wagę i da się je dobrać jednym parametrem.
    let sep_len = length(push_dir);
    var sep_dir = vec2<f32>(0.0);
    if (sep_len > 0.0001) { sep_dir = push_dir / sep_len; }

    var steering = dir + sep_dir * params.separation + wobble;
    // steering ograniczamy do długości 1 — wychodzenie poza to oznaczałoby
    // prędkość większą niż max_speed, a to nic nie daje
    let sl = length(steering);
    if (sl > 1.0) { steering = steering / sl; }

    let target_v = steering * params.max_speed;
    // wygładzanie, żeby nie było szarpnięć
    let v = mix(u.velocity, target_v, clamp(params.dt * 8.0, 0.0, 1.0));

    var pos = u.position + v * params.dt;

    // --- ograniczenia do areny (odpychanie od ścian)
    let pad = params.unit_size;
    let lo = params.arena_min + vec2<f32>(pad);
    let hi = params.arena_max - vec2<f32>(pad);
    pos = clamp(pos, lo, hi);

    u.position = pos;
    u.velocity = v;
    u.goal = goal;
    u.cooldown = max(u.cooldown - params.dt, 0.0);
    units[i] = u;
}

// -------------------------------------------------------------- 5. resolve
@compute @workgroup_size(64)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    var u = units[i];
    if (!unit_alive(u)) { return; }

    let raw = atomicExchange(&damage[i], 0u);
    if (raw > 0u) {
        u.health = u.health - f32(raw) / 1000.0;
    }
    if (u.health <= 0.0) {
        u.flags = u.flags & ~ALIVE;
        atomicAdd(&counters.kills, 1u);
    }
    units[i] = u;
}
