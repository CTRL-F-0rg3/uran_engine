struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

struct PushConstants {
    translation: vec4<f32>,
}

// UWAGA: Brak @group i @binding! 
var<push_constant> pc: PushConstants;

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    
    // Przesuwamy wierzchołek o wektor z ECS
    let pos = model.position + pc.translation.xyz;
    
    // Odwracamy Y, bo WGPU ma oś Y skierowaną w dół
    out.clip_position = vec4<f32>(pos.x, -pos.y, pos.z, 1.0);
    out.color = model.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}