use uran_engine::prelude::*;
use uran_ecs::Transform; // Usunęliśmy Visibility
use uran_render::MeshBuilder;
use uran_math::{Vec3, Color};
fn main() {
    let mut app = App::new()
        .window(
            windowed(1280, 720)
                .title("Uran Engine - Fluent Mesh API")
                .resizable(true)
                .background(0x1A1A2E)
        );

    // --- TWORZENIE OBIEKTU Z FLUENT API ---
    let (mesh, material, visibility) = MeshBuilder::triangle(0.0, 0.0, 0.0, 0.5)
        .color(Color::from_hex(0xFFAA00)) // Pomarańczowy
        .visibility(true)
        .is_shaded(false)
        .build();

    // Spawnujemy encję z komponentami: Transform (dla ruchu) + nasze nowe komponenty
    app.world.spawn((
        Transform {
            translation: Vec3::new(-0.8, 0.0, 0.0), // Zaczyna po lewej
            rotation: 0.0,
            scale: Vec3::ONE,
        },
        mesh,
        material,
        visibility,
    ));

    println!("Stworzono trójkąt z Fluent API!");
    app.run();
}