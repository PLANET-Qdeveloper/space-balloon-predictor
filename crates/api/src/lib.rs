pub mod error;
pub mod predict;
pub mod tawhiri;
pub mod weather;

pub use error::{ApiError, ApiErrorKind};
pub use predict::{PredictRequest, PredictResult, predict};
pub use tawhiri::{
    ParsedRequest, SuccessResponse, parse_balloon_class, parse_request,
    parse_request_with_balloon_class, run_prediction,
};
pub use weather::{fetch_gfs_dataset, resolve_gfs_run_time};
