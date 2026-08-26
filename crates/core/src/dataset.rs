pub mod gefs;
pub mod gfs;
pub mod loader;

pub use gefs::{
    GefsForecast, GefsForecastSet, GefsMember, GefsResolution, gefs_filter_url,
    resolve_gefs_forecasts,
};
pub use gfs::{GfsForecast, GfsForecastSet, GfsRegion, gfs_filter_url, resolve_gfs_forecasts};

use std::sync::Arc;

use chrono::{DateTime, Utc};
use log::info;

use crate::grib::{Atmosphere, PressureUnit};

#[derive(Clone)]
pub struct Dataset {
    atmospheres: Arc<Vec<(DateTime<Utc>, Atmosphere)>>,
    base_time: DateTime<Utc>,
}

impl Dataset {
    pub fn from_atmospheres(
        atmospheres: Vec<(DateTime<Utc>, Atmosphere)>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if atmospheres.is_empty() {
            return Err("Dataset requires at least one atmosphere time step".into());
        }
        let mut sorted = atmospheres;
        sorted.sort_by_key(|(dt, _)| *dt);
        let base_time = sorted.first().map(|(dt, _)| *dt).unwrap_or_else(Utc::now);
        info!(
            "Dataset created with {} time steps (base: {})",
            sorted.len(),
            base_time.format("%Y-%m-%dT%H:%M:%SZ"),
        );
        Ok(Self {
            atmospheres: Arc::new(sorted),
            base_time,
        })
    }

    pub fn from_grib_files(
        grib_paths: &[String],
        launch_time: DateTime<Utc>,
        pressure_unit: PressureUnit,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (atmospheres, _base_time) =
            loader::load_grib_series(grib_paths, launch_time, pressure_unit)?;
        Self::from_atmospheres(atmospheres)
    }

    /// MSMとGFSの組み合わせ
    pub fn from_ensemble(
        primary_path: &str,
        secondary_paths: &[String],
        launch_time: DateTime<Utc>,
        pressure_unit: PressureUnit,
        lat: f64,
        lon: f64,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let (atmospheres, _base_time) = loader::load_ensemble_series(
            primary_path,
            secondary_paths,
            launch_time,
            pressure_unit,
            lat,
            lon,
        )?;
        Self::from_atmospheres(atmospheres)
    }

    pub fn atmospheres(&self) -> &[(DateTime<Utc>, Atmosphere)] {
        &self.atmospheres
    }

    pub(crate) fn clone_inner(&self) -> Arc<Vec<(DateTime<Utc>, Atmosphere)>> {
        Arc::clone(&self.atmospheres)
    }

    pub fn base_time(&self) -> DateTime<Utc> {
        self.base_time
    }

    pub fn len(&self) -> usize {
        self.atmospheres.len()
    }

    pub fn is_empty(&self) -> bool {
        self.atmospheres.is_empty()
    }
}
