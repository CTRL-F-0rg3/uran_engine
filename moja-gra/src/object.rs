//! Centralny rejestr obiektów — wywołuje `object::update` każdego elementu.
//!
//! Nowy element dodajesz przez `uran element <nazwa>` (albo ręcznie:
//! moduł w `elements/mod.rs` + wywołanie poniżej).

use uran_engine::prelude::*;

use crate::elements;

/// Aktualizuje wszystkie elementy gry (co klatkę).
pub fn update_all(ctx: &mut Ctx) {
    // [uran:calls]
}
