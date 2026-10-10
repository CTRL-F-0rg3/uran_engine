//! Symulacja masywnych ilości jednostek na GPU (compute).
//!
//! Pozycje żyją wyłącznie w `storage` bufferze na karcie: CPU wysyła
//! tylko parametry, a renderer czyta pozycje w shaderze wierzchołkowym.
//! Dzięki temu koszt CPU na jednostkę jest zerowy — przy 100k jednostek
//! gra wciąż wykonuje kilka tysięcy operacji na klatkę.

mod sim;
mod unit;

pub use sim::GpuSim;
pub use unit::{GpuUnit, SimParams, SimStats, Team, UnitFlags};
