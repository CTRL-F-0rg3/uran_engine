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
}

struct Unit {
    position: vec2<f32>,
    velocity: vec2<f32>,
    target: vec2<f32>,
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
// obrażenia do zastosowania w przebiegu (atomowe — wielu strzelców
// może trafić w tę samą jednostkę)
@group(0) @binding(3) var<storage, read_write> damage: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> counters: Counters;

const ALIVE: u32 = 1u;
const TEAM_BIT: u32 = 2u;
const ATTACKED: u32 = 4u;

fn grid_stride() -> u32 { return 1u + MAX_PER_CELL; }
fn total_cells() -> u32 { return params.grid_dim.x * params.grid_dim.y; }
fn is_alive(u: Unit) -> bool { return (u.flags & ALIVE) != 0u; }
fn is_enemy(u: Unit) -> bool { return (u.flags & TEAM_BIT) != 0u; }

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
    if (!is_alive(u)) { return; }

    let base = cell_of(u.position) * grid_stride();
    let slot = atomicAdd(&grid[base], 1u);
    // przepełniona komórka: gubimy nadmiar (akceptowalne — to tylko
    // odpychanie i wybór celu, a nie dokładne kolizje)
    if (slot < MAX_PER_CELL) {
        atomicStore(&grid[base + 1u + slot], i);
    }
}

/// Indeks najbliższego wroga w zasięgu, albo `0xFFFFFFFF`.
fn nearest_victim(from: vec2<f32>, want_enemy: bool, range: f32) -> u32 {
    let c = cell_of(from);
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
                if (!is_alive(o)) { continue; }
                if (is_enemy(o) != want_enemy) { continue; }
                let delta = o.position - from;
                let d2 = dot(delta, delta);
                if (d2 < best_d2) { best_d2 = d2; best = j; }
            }
        }
    }
    return best;
}

// ---------------------------------------------------------------- 4. think
@compute @workgroup_size(64)
fn think(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    var u = units[i];
    if (!is_alive(u)) { return; }

    let me = u.position;
    let my_cell = cell_of(me);
    let cx = my_cell % params.grid_dim.x;
    let cy = my_cell / params.grid_dim.x;

    var push = vec2<f32>(0.0);
    var enemy_pos = vec2<f32>(0.0);
    var enemy_d2 = 1.0e30;
    var saw_enemy = false;

    // przeglądamy 3x3 sąsiednich komórek
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
                let o = units[j];
                if (!is_alive(o)) { continue; }
                let delta = o.position - me;
                let d2 = dot(delta, delta);

                // odpychanie od każdego sąsiada, niezależnie od drużyny
                let r2 = params.separation_radius * params.separation_radius;
                if (d2 < r2 && d2 > 0.0001) {
                    push = push - delta * (1.0 / d2);
                }
                // zapamiętujemy najbliższego wroga
                if (is_enemy(o) != is_enemy(u) && d2 < enemy_d2) {
                    enemy_d2 = d2;
                    enemy_pos = o.position;
                    saw_enemy = true;
                }
            }
        }
    }

    // dokąd iść: do wroga, jeśli jakiś jest w okolicy, inaczej do celu
    var goal = u.target;
    if (saw_enemy) {
        goal = enemy_pos;
    }
    let to_goal = goal - me;
    let dist = length(to_goal);
    var dir = vec2<f32>(0.0);
    if (dist > 0.001) {
        dir = to_goal / dist;
    } else if (!saw_enemy) {
        // stoi w miejscu i nie ma wroga — rozłóżmy go lekko,
        // inaczej setki tysięcy jednostek zastyga w jednym punkcie
        let ang = hash01(u.seed + u32(params.time * 60.0)) * 6.2831853;
        dir = vec2<f32>(cos(ang), sin(ang));
    }

    let accel = dir * 900.0 + push * params.separation;
    var vel = u.velocity + accel * params.dt;
    let speed = length(vel);
    if (speed > params.max_speed) {
        vel = vel * (params.max_speed / speed);
    }
    u.velocity = vel;

    var pos = me + vel * params.dt;
    // klamra do areny + odbicie od ściany
    let r = params.unit_size;
    if (pos.x < params.arena_min.x + r) { pos.x = params.arena_min.x + r; u.velocity.x = abs(u.velocity.x); }
    if (pos.x > params.arena_max.x - r) { pos.x = params.arena_max.x - r; u.velocity.x = -abs(u.velocity.x); }
    if (pos.y < params.arena_min.y + r) { pos.y = params.arena_min.y + r; u.velocity.y = abs(u.velocity.y); }
    if (pos.y > params.arena_max.y - r) { pos.y = params.arena_max.y - r; u.velocity.y = -abs(u.velocity.y); }
    u.position = pos;

    // atak: tylko jeśli wróg jest w zasięgu
    var attacked = false;
    if (saw_enemy && dist <= params.attack_range) {
        u.cooldown = u.cooldown - params.dt;
        if (u.cooldown <= 0.0) {
            let victim = nearest_victim(me, is_enemy(u), params.attack_range);
            if (victim != 0xFFFFFFFFu) {
                // obrażenia jako u32 w tysięcznych — atomowe, więc dwóch
                // strzelców trafiających w cel nie nadpisuje się nawzajem
                atomicAdd(&damage[victim], u32(max(params.attack_damage, 0.0) * 1000.0));
                atomicAdd(&counters.shots, 1u);
                attacked = true;
            }
            u.cooldown = params.attack_cooldown;
        }
    } else {
        u.cooldown = max(u.cooldown, 0.0);
    }

    if (attacked) { u.flags = u.flags | ATTACKED; } else { u.flags = u.flags & ~ATTACKED; }
    units[i] = u;
}

// --------------------------------------------------------------- 5. resolve
@compute @workgroup_size(64)
fn resolve(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= params.count) { return; }
    var u = units[i];
    if (!is_alive(u)) { return; }

    // liczniki statystyk (wyzerowane przez `clear_counters` na początku)
    if (is_enemy(u)) {
        atomicAdd(&counters.alive_enemy, 1u);
    } else {
        atomicAdd(&counters.alive_friendly, 1u);
    }

    // atomicExchange zeruje bufor, więc obrażenia nie kumulują się w nieskończoność
    let dmg = atomicExchange(&damage[i], 0u);
    if (dmg != 0u) {
        u.health = u.health - f32(dmg) / 1000.0;
        if (u.health <= 0.0) {
            u.health = 0.0;
            u.flags = u.flags & ~ALIVE;   // martwej nie rysujemy i nie liczymy
            atomicAdd(&counters.kills, 1u);
        }
    }
    units[i] = u;
}