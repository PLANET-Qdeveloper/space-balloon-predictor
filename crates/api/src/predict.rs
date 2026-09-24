use std::path::Path;

use chrono::{DateTime, Utc};
use space_balloon_predictor_rs::engine::physics::ascent_coeff_k;
use space_balloon_predictor_rs::engine::simulation::{AscentParams, SimConfig, Simulator, Trajectory};
use space_balloon_predictor_rs::export::kml;
use space_balloon_predictor_rs::geo::coords::Geodetic;

use crate::weather::{fetch_gfs_dataset, resolve_gfs_run_time};

/// API リクエスト相当の予測入力。
#[derive(Debug, Clone)]
pub struct PredictRequest {
    pub launch_time: DateTime<Utc>,
    /// None なら自動で最新の利用可能サイクルを選ぶ。
    pub gfs_run_time: Option<DateTime<Utc>>,
    pub launch_site: Geodetic,
    pub start_in_descent: bool,
    pub ascent_rate_m_s: f64,
    pub balloon_class_g: u32,
    pub gross_mass_kg: f64,
    pub ground_descend_rate_m_s: f64,
    pub burst_altitude_m: f64,
    pub dt: f64,
}

impl Default for PredictRequest {
    fn default() -> Self {
        Self {
            launch_time: Utc::now() + chrono::Duration::hours(6),
            gfs_run_time: None,
            launch_site: Geodetic {
                lat: 35.0,
                lon: 139.0,
                alt: 10.0,
            },
            start_in_descent: false,
            ascent_rate_m_s: 5.0,
            balloon_class_g: 2000,
            gross_mass_kg: 6.0,
            ground_descend_rate_m_s: 5.0,
            burst_altitude_m: 30_000.0,
            dt: 5.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PredictResult {
    pub gfs_run_time: DateTime<Utc>,
    pub launch_time: DateTime<Utc>,
    pub trajectory: Trajectory,
    pub kml: String,
}

/// GFS を自動取得して軌道予測を実行する。
pub fn predict(
    request: &PredictRequest,
    cache_dir: &Path,
) -> Result<PredictResult, Box<dyn std::error::Error + Send + Sync>> {
    let gfs_run_time = request
        .gfs_run_time
        .unwrap_or_else(|| resolve_gfs_run_time(request.launch_time));

    let dataset = fetch_gfs_dataset(
        gfs_run_time,
        request.launch_time,
        request.launch_site.lat,
        request.launch_site.lon,
        cache_dir,
    )?;

    let coeff_k = ascent_coeff_k(request.balloon_class_g).ok_or_else(|| {
        format!(
            "Unknown balloon class: {}g (expected 1000, 1500, 2000, or 3000)",
            request.balloon_class_g
        )
    })?;

    let config = SimConfig {
        launch_site: request.launch_site,
        start_in_descent: request.start_in_descent,
        ascent: AscentParams {
            gross_mass_kg: request.gross_mass_kg,
            target_rate_m_s: request.ascent_rate_m_s,
            coeff_k,
            burst_volume_m3: None,
        },
        ground_descend_rate_m_s: request.ground_descend_rate_m_s,
        burst_altitude_m: request.burst_altitude_m,
        dt: request.dt,
    };

    let trajectory = Simulator::new(config, dataset, request.launch_time).run();
    let kml = kml::trajectory_to_kml(&trajectory, request.launch_time);

    Ok(PredictResult {
        gfs_run_time,
        launch_time: request.launch_time,
        trajectory,
        kml,
    })
}
