// =============================================================================
//  pbr.wgsl — realistyczny shader PBR (Physically Based Rendering) pod 3D
//
//  Zawiera:
//   - Cook-Torrance BRDF (GGX + Smith height-correlated + Schlick Fresnel)
//   - Workflow metallic/roughness (zgodny z glTF 2.0)
//   - Normal mapping (TBN), AO, emissive
//   - Światła: directional, point, spot (do 8), z fizyczną attenuacją
//   - Cień słońca (światło nr 0) z PCF 3x3
//   - IBL: irradiance (diffuse) + prefiltered env (odbicia) + BRDF LUT
//   - Kompensacja energii (multi-scatter), specular occlusion
//   - Specular anti-aliasing (mniej migotania odbić)
//   - Tone mapping ACES + ekspozycja + korekcja sRGB + dithering
//
//  Układ bind groupów:
//   group(0) — scena:    0 camera, 1 lights, 2 shadow params, 3 shadow map,
//                        4 shadow sampler (comparison), 5 env irradiance (cube),
//                        6 env prefiltered (cube, z mipmapami), 7 BRDF LUT (2d rg),
//                        8 env sampler (linear, mipmap linear)
//   group(1) — materiał: 0 uniform, 1 sampler, 2 albedo (rgba8unorm-srgb),
//                        3 normal (rgba8unorm), 4 ORM (r=AO, g=roughness, b=metallic),
//                        5 emissive (rgba8unorm-srgb)
//   group(2) — obiekt:   0 model + normal matrix
//
//  Atrybuty wierzchołków: 0 position vec3, 1 normal vec3, 2 tangent vec4, 3 uv vec2
// =============================================================================

const PI: f32 = 3.14159265359;
const MAX_LIGHTS: u32 = 8u;
const LIGHT_DIRECTIONAL: u32 = 0u;
const LIGHT_POINT: u32 = 1u;
const LIGHT_SPOT: u32 = 2u;
const MIN_ROUGHNESS: f32 = 0.045;

// true, jeśli render target NIE jest *-srgb (np. bgra8unorm). Ustaw przez `constants`.
override APPLY_SRGB_ENCODE: bool = true;

// ---------------------------------------------------------------- struktury --

struct Camera {
    view_proj: mat4x4<f32>,
    position: vec3<f32>,
    exposure: f32,
    env_intensity: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

struct Light {
    position: vec3<f32>,
    kind: u32,            // 0 dir, 1 point, 2 spot
    direction: vec3<f32>, // kierunek, w którym świeci światło
    range: f32,           // 0 = bez limitu zasięgu
    color: vec3<f32>,     // liniowy RGB
    intensity: f32,
    inner_cos: f32,       // cos kąta wewnętrznego stożka (spot)
    outer_cos: f32,       // cos kąta zewnętrznego stożka (spot)
    _pad: vec2<f32>,
};

struct Lights {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    items: array<Light, 8>,
};

struct ShadowParams {
    view_proj: mat4x4<f32>,
    depth_bias: f32,
    normal_bias: f32,
    texel_size: f32,   // 1.0 / rozdzielczość shadow mapy
    strength: f32,     // 0..1
};

struct Material {
    base_color: vec4<f32>,
    emissive: vec3<f32>,
    normal_scale: f32,
    metallic: f32,      // mnożnik
    roughness: f32,     // mnożnik
    occlusion: f32,     // siła AO 0..1
    alpha_cutoff: f32,  // 0 = wyłączone
};

struct Model {
    model: mat4x4<f32>,
    normal_matrix: mat4x4<f32>, // transpose(inverse(model))
};

// ----------------------------------------------------------------- bindingi --

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> lights: Lights;
@group(0) @binding(2) var<uniform> shadow: ShadowParams;
@group(0) @binding(3) var shadow_map: texture_depth_2d;
@group(0) @binding(4) var shadow_sampler: sampler_comparison;
@group(0) @binding(5) var env_diffuse: texture_cube<f32>;
@group(0) @binding(6) var env_specular: texture_cube<f32>;
@group(0) @binding(7) var brdf_lut: texture_2d<f32>;
@group(0) @binding(8) var env_sampler: sampler;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var mat_sampler: sampler;
@group(1) @binding(2) var albedo_tex: texture_2d<f32>;
@group(1) @binding(3) var normal_tex: texture_2d<f32>;
@group(1) @binding(4) var orm_tex: texture_2d<f32>;
@group(1) @binding(5) var emissive_tex: texture_2d<f32>;

@group(2) @binding(0) var<uniform> object: Model;

// ------------------------------------------------------------------ vertex ---

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) uv: vec2<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) tangent: vec3<f32>,
    @location(3) tangent_sign: f32,
    @location(4) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let world = object.model * vec4<f32>(in.position, 1.0);
    out.world_pos = world.xyz;
    out.clip = camera.view_proj * world;
    out.normal = (object.normal_matrix * vec4<f32>(in.normal, 0.0)).xyz;
    out.tangent = (object.model * vec4<f32>(in.tangent.xyz, 0.0)).xyz;
    out.tangent_sign = in.tangent.w;
    out.uv = in.uv;
    return out;
}

// -------------------------------------------------------------- funkcje BRDF -

// Rozkład normalnych GGX (Trowbridge-Reitz)
fn distribution_ggx(n_dot_h: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let d = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * d * d);
}

// Widoczność: Smith GGX height-correlated (zawiera już człon 1/(4 NdotL NdotV))
fn visibility_smith_ggx(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let a2 = alpha * alpha;
    let gv = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - a2) + a2);
    let gl = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - a2) + a2);
    return 0.5 / max(gv + gl, 1e-5);
}

// Fresnel (przybliżenie Schlicka)
fn fresnel_schlick(v_dot_h: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - v_dot_h, 5.0);
}

// Fresnel z uwzględnieniem roughness (do IBL)
fn fresnel_schlick_roughness(n_dot_v: f32, f0: vec3<f32>, roughness: f32) -> vec3<f32> {
    let grazing = max(vec3<f32>(1.0 - roughness), f0);
    return f0 + (grazing - f0) * pow(1.0 - n_dot_v, 5.0);
}

// ------------------------------------------------------------------ światła --

fn distance_attenuation(dist2: f32, range: f32) -> f32 {
    let inv_sq = 1.0 / max(dist2, 1e-4);
    if (range <= 0.0) {
        return inv_sq;
    }
    let f = dist2 / (range * range);
    let w = saturate(1.0 - f * f);
    return inv_sq * w * w; // gładkie wygaszanie na granicy zasięgu
}

fn spot_attenuation(l: vec3<f32>, dir: vec3<f32>, inner_cos: f32, outer_cos: f32) -> f32 {
    let cd = dot(-l, normalize(dir));
    let t = saturate((cd - outer_cos) / max(inner_cos - outer_cos, 1e-4));
    return t * t;
}

// Cień: PCF 3x3 z bias-em wzdłuż normalnej
fn sun_shadow(world_pos: vec3<f32>, n: vec3<f32>) -> f32 {
    let offset_pos = world_pos + n * shadow.normal_bias;
    let lc = shadow.view_proj * vec4<f32>(offset_pos, 1.0);
    let ndc = lc.xyz / lc.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let depth = ndc.z - shadow.depth_bias;

    var sum = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let o = vec2<f32>(f32(x), f32(y)) * shadow.texel_size;
            sum += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + o, depth);
        }
    }
    sum /= 9.0;

    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0)) && ndc.z <= 1.0;
    return select(1.0, mix(1.0, sum, shadow.strength), inside);
}

// ----------------------------------------------------------------- pomocnicze -

fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return saturate((x * (a * x + b)) / (x * (c * x + d) + e));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Interleaved gradient noise — tani dithering przeciw bandingowi
fn ign(p: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(p, vec2<f32>(0.06711056, 0.00583715))));
}

// ---------------------------------------------------------------- fragment ---

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // --- geometria ---
    var ng = normalize(in.normal);
    ng = select(-ng, ng, front); // dwustronne powierzchnie

    // Specular AA (Kaplanyan/Tokuyoshi) — musi być przed discard
    let dndx = dpdx(ng);
    let dndy = dpdy(ng);
    let variance = 0.25 * (dot(dndx, dndx) + dot(dndy, dndy));
    let kernel = min(2.0 * variance, 0.18);

    // --- tekstury ---
    let albedo_s = textureSample(albedo_tex, mat_sampler, in.uv);
    let nmap = textureSample(normal_tex, mat_sampler, in.uv).xyz;
    let orm = textureSample(orm_tex, mat_sampler, in.uv).rgb;
    let emissive_s = textureSample(emissive_tex, mat_sampler, in.uv).rgb;

    let base = albedo_s * material.base_color;
    if (base.a < material.alpha_cutoff) {
        discard;
    }

    // --- parametry materiału ---
    let albedo = base.rgb;
    let ao = mix(1.0, orm.r, material.occlusion);
    let roughness = clamp(orm.g * material.roughness, MIN_ROUGHNESS, 1.0);
    let metallic = saturate(orm.b * material.metallic);

    var alpha = roughness * roughness;
    alpha = sqrt(min(alpha * alpha + kernel, 1.0));

    // --- normalna z normal mapy (TBN) ---
    let t0 = normalize(in.tangent);
    let t = normalize(t0 - ng * dot(ng, t0));
    let b = cross(ng, t) * in.tangent_sign;
    var tn = nmap * 2.0 - 1.0;
    tn = vec3<f32>(tn.xy * material.normal_scale, tn.z);
    let n = normalize(t * tn.x + b * tn.y + ng * tn.z);

    let v = normalize(camera.position - in.world_pos);
    let n_dot_v = max(dot(n, v), 1e-4);

    // F0: dielektryki ~4%, metale przejmują kolor albedo
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);
    let diffuse_color = albedo * (1.0 - metallic);

    // --- światła bezpośrednie ---
    var lo = vec3<f32>(0.0);
    let count = min(lights.count, MAX_LIGHTS);
    for (var i = 0u; i < count; i++) {
        let light = lights.items[i];

        var l = vec3<f32>(0.0);
        var atten = 1.0;
        var shadow_term = 1.0;

        if (light.kind == LIGHT_DIRECTIONAL) {
            l = normalize(-light.direction);
            if (i == 0u) {
                shadow_term = sun_shadow(in.world_pos, ng);
            }
        } else {
            let to_light = light.position - in.world_pos;
            let dist2 = max(dot(to_light, to_light), 1e-4);
            l = to_light * inverseSqrt(dist2);
            atten = distance_attenuation(dist2, light.range);
            if (light.kind == LIGHT_SPOT) {
                atten *= spot_attenuation(l, light.direction, light.inner_cos, light.outer_cos);
            }
        }

        let n_dot_l = saturate(dot(n, l));
        if (n_dot_l > 0.0) {
            let h = normalize(v + l);
            let n_dot_h = saturate(dot(n, h));
            let v_dot_h = saturate(dot(v, h));

            let d = distribution_ggx(n_dot_h, alpha);
            let vis = visibility_smith_ggx(n_dot_v, n_dot_l, alpha);
            let f = fresnel_schlick(v_dot_h, f0);

            let specular = d * vis * f;
            let diffuse = (vec3<f32>(1.0) - f) * diffuse_color / PI;

            let radiance = light.color * light.intensity * atten * shadow_term;
            lo += (diffuse + specular) * radiance * n_dot_l;
        }
    }

    // --- IBL (oświetlenie z otoczenia + odbicia) ---
    let r = reflect(-v, n);
    let mip_count = f32(textureNumLevels(env_specular) - 1u);

    let irradiance = textureSampleLevel(env_diffuse, env_sampler, n, 0.0).rgb;
    let prefiltered = textureSampleLevel(env_specular, env_sampler, r, roughness * mip_count).rgb;
    let brdf = textureSampleLevel(brdf_lut, env_sampler, vec2<f32>(n_dot_v, roughness), 0.0).rg;

    let f_r = fresnel_schlick_roughness(n_dot_v, f0, roughness);
    let kd = (vec3<f32>(1.0) - f_r) * (1.0 - metallic);

    var spec_ibl = prefiltered * (f0 * brdf.x + brdf.y);
    // kompensacja utraty energii przy wielokrotnym rozpraszaniu (chropowate metale)
    let energy = vec3<f32>(1.0) + f0 * (1.0 / max(brdf.x + brdf.y, 1e-4) - 1.0);
    spec_ibl *= energy;

    // specular occlusion (Lagarde) — odbicia nie "świecą" w zagłębieniach
    let spec_occ = saturate(pow(n_dot_v + ao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ao);

    // kd już zawiera (1 - metallic), więc mnożymy przez czyste albedo
    let diffuse_ibl = irradiance * albedo * kd;
    let ambient = (diffuse_ibl * ao + spec_ibl * spec_occ) * camera.env_intensity;

    // --- emissive ---
    let emissive = emissive_s * material.emissive;

    // --- końcowy kolor ---
    var color = (ambient + lo + emissive) * camera.exposure;
    color = aces_tonemap(color);

    if (APPLY_SRGB_ENCODE) {
        color = linear_to_srgb(color);
    }

    // dithering (±0.5 LSB) redukuje pasy w gradientach
    color += (ign(in.clip.xy) - 0.5) / 255.0;

    return vec4<f32>(color, base.a);
}

// ------------------------------------------------- pass cieni (opcjonalnie) --
// Osobny pipeline, tylko głębia: użyj z view_proj światła w camera.view_proj.

@vertex
fn vs_shadow(in: VertexIn) -> @builtin(position) vec4<f32> {
    return camera.view_proj * object.model * vec4<f32>(in.position, 1.0);
}
