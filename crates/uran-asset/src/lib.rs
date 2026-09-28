//! System assetów: uchwyty, ładowanie obrazów i czcionek, cache po ścieżce.

pub mod asset_server;
pub mod handle;
pub mod loader;
pub mod server;

pub use asset_server::AssetServer;
pub use handle::{Handle, HandleId};
pub use loader::{AssetError, FontData, Image};
pub use server::Assets;
