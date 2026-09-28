// Shadery renderera 2D (wgpu).
//
// Pipeline "sprite": instancjonowane prostokąty (jeden draw call na
// (tekstura, tryb mieszania)). Pipeline "mesh": dowolne siatki z wierzchołków.

// ------------------------------------------------------------------ common
struct Globals {
    view_proj: mat4x4<f32>,
}

@group(1) @binding(0) var<uniform> globals: Globals;

@group(0) @binding(0) var samp: sampler;
@group(0) @binding(1) var tex: texture_2d<f32>;

// ------------------------------------------------------------------ sprite
struct VSIn {
    @location(0) corner: vec2<f32>,   // jednostka kwadratu (-0.5..0.5)
    @location(1) m0: vec4<f32>,       // (m00, m01, m10, m11) - część liniowa
    @location(2) m1: vec4<f32>,       // (m20, m21, 0, 0) - translacja
    @location(3) uv_rect: vec4<f32>,  // (u0, v0, u1, v1)
    @location(4) tint: vec4<f32>,
}

struct VSOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn vs_main(in: VSIn) -> VSOut {
    var out: VSOut;

    // Macierz 2x3 -> 3x3 (jednostkowa trzecia kolumna).
    let model = mat3x3<f32>(
        in.m0.x, in.m0.y, 0.0,
        in.m0.z, in.m0.w, 0.0,
        in.m1.x, in.m1.y, 1.0,
    );
    let world = model * vec3<f32>(in.corner, 1.0);
    out.clip_position = globals.view_proj * vec4<f32>(world, 1.0);

    // (0,0) = lewy górny róg obrazu, (1,1) = prawy dolny
    out.uv = vec2<f32>(
        mix(in.uv_rect.x, in.uv_rect.z, in.corner.x + 0.5),
        mix(in.uv_rect.y, in.uv_rect.w, 0.5 - in.corner.y),
    );
    out.color = in.tint;
    return out;
}

@fragment
fn fs_main(in: VSOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}

// -------------------------------------------------------------------- mesh
struct MeshPush {
    // mat3x3<f32> zajmuje w WGSL 48 bajtów (3 kolumny po 16 bajtów)
    model: mat3x3<f32>,
    tint: vec4<f32>,
}

var<push_constant> pc: MeshPush;

struct MeshVIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct MeshVOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@vertex
fn mesh_vs_main(in: MeshVIn) -> MeshVOut {
    var out: MeshVOut;
    let world = pc.model * vec3<f32>(in.position, 1.0);
    out.clip_position = globals.view_proj * vec4<f32>(world, 1.0);
    out.uv = in.uv;
    // Kolory wierzchołków i tint przychodzą JUŻ w liniowym świetle
    // (konwersję sRGB -> liniowe robi CPU przy uploadzie geometrii i przy
    // budowie push constants), więc tutaj nie konwertujemy ponownie —
    // podwójna konwersja przygaszała siatki do niemal czerni.
    out.color = in.color * pc.tint;
    return out;
}

@fragment
fn mesh_fs_main(in: MeshVOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, in.uv) * in.color;
}

// sRGB -> liniowe światło (siatki nie przechodzą przez konwersję formatu
// tekstury, bo mogą być nieoświetlone)
fn srgb_to_linear(c: vec4<f32>) -> vec4<f32> {
    let cutoff = vec3<f32>(0.04045);
    let lower = c.rgb / 12.92;
    let higher = pow((c.rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(higher, lower, c.rgb <= cutoff), c.a);
}
