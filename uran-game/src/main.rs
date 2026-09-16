use uran_engine::prelude::*;

fn main() {
    App::new()
        .window(
            windowed(1280, 720)
                .title("Moja Gra na Uran Engine")
                .resizable(true)
                .background(0x000000) // Czarny, lub np. 0x1A1A2E dla ciemnego granatu
        )
        .run();
}