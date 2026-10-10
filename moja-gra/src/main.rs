//! moja-gra — gra na silniku Uran.
//!
//! Uruchomienie: `cargo run`.

mod elements;
mod init;
mod object;

use uran_engine::prelude::*;

fn main() {
    App::new()
        .window(windowed(1280, 720).title("moja-gra"))
        .add_startup_system(|ctx: &mut Ctx| {
            init::init_all(ctx);
        })
        .add_system(|ctx: &mut Ctx| {
            object::update_all(ctx);
            if ctx.input.just_pressed(Key::Escape) {
                std::process::exit(0);
            }
        })
        .run();
}
