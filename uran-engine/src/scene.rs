//! Zarządzanie scenami — wygodne przełączanie (switch) i stos (push/pop).
//!
//! Scena to samodzielny kawałek gry: menu, poziom, pauza. [`SceneManager`]
//! trzyma stos scen i obsługuje przejścia zwracane z [`Scene::update`].
//!
//! ```ignore
//! use uran_engine::prelude::*;
//!
//! struct Menu;
//! impl Scene for Menu {
//!     fn update(&mut self, ctx: &mut Ctx) -> Transition {
//!         if ctx.input.just_pressed(Key::Enter) {
//!             Transition::Switch(Box::new(Gra))
//!         } else {
//!             Transition::None
//!         }
//!     }
//! }
//! struct Gra;
//! impl Scene for Gra {
//!     fn update(&mut self, ctx: &mut Ctx) -> Transition {
//!         if ctx.input.just_pressed(Key::Escape) {
//!             Transition::Switch(Box::new(Menu))
//!         } else {
//!             Transition::None
//!         }
//!     }
//! }
//!
//! let mut sceny = SceneManager::new();
//! // w systemie: if sceny.update(ctx) { std::process::exit(0); }
//! ```

use crate::context::Ctx;

/// Co scena chce zrobić po swojej aktualizacji.
pub enum Transition {
    /// Scena gra dalej.
    None,
    /// Zastąp bieżącą scenę nową (stara jest niszczona).
    Switch(Box<dyn Scene + Send>),
    /// Połóż nową scenę na wierzch (np. menu pauzy nad grą).
    Push(Box<dyn Scene + Send>),
    /// Zdejmij bieżącą scenę i wróć do poprzedniej.
    Pop,
    /// Zakończ grę.
    Quit,
}

/// Scena — samodzielny kawałek gry.
///
/// `update` robi i logikę, i rysowanie (ma dostęp do [`Ctx::gfx`]), tak jak
/// zwykły system silnika.
pub trait Scene {
    /// Wywoływane raz, gdy scena staje się aktywna.
    fn enter(&mut self, _ctx: &mut Ctx) {}
    /// Wywoływane co klatkę. Zwraca, co dalej (patrz [`Transition`]).
    fn update(&mut self, _ctx: &mut Ctx) -> Transition {
        Transition::None
    }
    /// Wywoływane raz, gdy scena przestaje być aktywna.
    fn exit(&mut self, _ctx: &mut Ctx) {}
}

/// Stos scen — trzyma wierzchnią scenę aktywną i obsługuje przejścia.
pub struct SceneManager {
    stack: Vec<Box<dyn Scene + Send>>,
    quitting: bool,
}

impl SceneManager {
    pub fn new() -> Self {
        Self {
            stack: Vec::new(),
            quitting: false,
        }
    }

    /// Ustawia scenę startową.
    pub fn start(&mut self, mut scene: Box<dyn Scene + Send>, ctx: &mut Ctx) {
        scene.enter(ctx);
        self.stack.push(scene);
    }

    /// Zastępuje bieżącą scenę nową.
    pub fn switch(&mut self, mut scene: Box<dyn Scene + Send>, ctx: &mut Ctx) {
        self.pop_one(ctx);
        scene.enter(ctx);
        self.stack.push(scene);
    }

    /// Kładzie scenę na wierzch (np. menu pauzy nad rozgrywką).
    pub fn push(&mut self, mut scene: Box<dyn Scene + Send>, ctx: &mut Ctx) {
        scene.enter(ctx);
        self.stack.push(scene);
    }

    /// Zdejmuje wierzchnią scenę i wraca do poprzedniej.
    pub fn pop(&mut self, ctx: &mut Ctx) {
        self.pop_one(ctx);
    }

    /// Aktualizuje wierzchnią scenę i obsługuje przejścia.
    ///
    /// Zwraca `true`, gdy gra ma się zakończyć (scena zwróciła
    /// [`Transition::Quit`]).
    pub fn update(&mut self, ctx: &mut Ctx) -> bool {
        let transition = match self.stack.last_mut() {
            Some(top) => top.update(ctx),
            None => return self.quitting,
        };
        match transition {
            Transition::None => {}
            Transition::Switch(scene) => self.switch(scene, ctx),
            Transition::Push(scene) => self.push(scene, ctx),
            Transition::Pop => self.pop(ctx),
            Transition::Quit => self.quitting = true,
        }
        self.quitting
    }

    /// Czy stos jest pusty.
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    /// Liczba scen na stosie.
    pub fn len(&self) -> usize {
        self.stack.len()
    }

    fn pop_one(&mut self, ctx: &mut Ctx) {
        if let Some(mut top) = self.stack.pop() {
            top.exit(ctx);
        }
    }
}

impl Default for SceneManager {
    fn default() -> Self {
        Self::new()
    }
}
