// Dyxel Language Server - Library

pub mod rsx_analyzer;
pub use rsx_analyzer::RsxAnalyzer;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
