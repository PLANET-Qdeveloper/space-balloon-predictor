pub mod physics;
pub mod simulation;
mod terrain;

pub use terrain::{normalize_opentopo_base_url, DemSource, DEFAULT_OPENTOPO_BASE_URL};
