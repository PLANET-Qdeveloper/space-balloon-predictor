use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};

use crate::grib::{AnyGribFile, Atmosphere, HeightUnit, PressureUnit, open_grib};

pub fn load_grib_series(
    grib_paths: &[String],
    launch_time: DateTime<Utc>,
    pressure_unit: PressureUnit,
    height_unit: HeightUnit,
) -> Result<(Vec<(DateTime<Utc>, Atmosphere)>, DateTime<Utc>), Box<dyn std::error::Error>> {
    let mut all_atmospheres: Vec<(DateTime<Utc>, Atmosphere)> = Vec::new();

    for path_str in grib_paths {
        let path = std::path::Path::new(path_str);
        info!("Reading file: {}", path_str);

        match open_grib(path) {
            Ok(AnyGribFile::V2(grib2)) => match grib2.to_atmosphere(pressure_unit) {
                Ok(vec) => {
                    debug!("  -> Read {} time steps from GRIB2", vec.len());
                    for (dt, _) in &vec {
                        debug!("     {}", dt.format("%Y-%m-%dT%H:%M:%SZ"));
                    }
                    all_atmospheres.extend(vec);
                }
                Err(e) => {
                    warn!("Failed to parse as GRIB2: {}", e);
                }
            },
            Ok(AnyGribFile::V1(grib1)) => match grib1.to_atmosphere(pressure_unit, height_unit) {
                Ok(vec) => {
                    debug!("  -> Read {} time steps from GRIB1", vec.len());
                    for (dt, _) in &vec {
                        debug!("     {}", dt.format("%Y-%m-%dT%H:%M:%SZ"));
                    }
                    all_atmospheres.extend(vec);
                }
                Err(e) => {
                    error!("Failed to parse as GRIB1: {}", e);
                }
            },
            Err(e) => {
                error!("Failed to open file: {}", e);
            }
        }
    }

    if all_atmospheres.is_empty() {
        return Err("No valid atmosphere data found from the provided files".into());
    }

    all_atmospheres.sort_by_key(|(dt, _)| *dt);
    all_atmospheres.dedup_by_key(|(dt, _)| *dt);

    if all_atmospheres.len() < 2 {
        return Err(format!(
            "Need at least 2 distinct time steps, got {}",
            all_atmospheres.len()
        )
        .into());
    }

    let selected = if all_atmospheres.len() <= 3 {
        all_atmospheres
    } else {
        let base_idx = all_atmospheres
            .iter()
            .position(|(dt, _)| *dt > launch_time)
            .unwrap_or(all_atmospheres.len());

        let mid_idx = if base_idx == 0 {
            0
        } else if base_idx >= all_atmospheres.len() {
            all_atmospheres.len() - 3
        } else {
            base_idx.saturating_sub(1)
        };
        let end = (mid_idx + 3).min(all_atmospheres.len());
        let start = end.saturating_sub(3);
        all_atmospheres.drain(start..end).collect()
    };

    let base_time = selected.first().map(|(dt, _)| *dt).unwrap_or(launch_time);
    info!(
        "Selected {} time steps around {} (base: {})",
        selected.len(),
        launch_time.format("%H:%M"),
        base_time.format("%Y-%m-%dT%H:%M:%SZ"),
    );

    Ok((selected, base_time))
}

pub fn load_ensemble_series(
    primary_path: &str,
    secondary_paths: &[String],
    launch_time: DateTime<Utc>,
    pressure_unit: PressureUnit,
    height_unit: HeightUnit,
    lat: f64,
    lon: f64,
) -> Result<(Vec<(DateTime<Utc>, Atmosphere)>, DateTime<Utc>), Box<dyn std::error::Error>> {
    info!(
        "Ensemble mode: primary='{}', secondary={:?}",
        primary_path, secondary_paths
    );

    info!("Reading primary (MSM) file: {}", primary_path);
    let primary_path_obj = std::path::Path::new(primary_path);

    let msm_series: Vec<(DateTime<Utc>, Atmosphere)> = match open_grib(primary_path_obj) {
        Ok(AnyGribFile::V2(grib2)) => grib2.to_atmosphere(pressure_unit)?,
        Ok(AnyGribFile::V1(grib1)) => grib1.to_atmosphere(pressure_unit, height_unit)?,
        Err(e) => return Err(format!("Failed to open primary file: {}", e).into()),
    };
    debug!("  -> {} time steps from MSM", msm_series.len());

    if let Some((_, atmo)) = msm_series.first() {
        debug!("  MSM pressure levels (Pa): {:?}", atmo.pressure_levels());
    }

    let mut gfs_series: Vec<(DateTime<Utc>, Atmosphere)> = Vec::new();
    for path_str in secondary_paths {
        let path = std::path::Path::new(path_str);
        info!("Reading secondary (GFS) file: {}", path_str);

        match open_grib(path) {
            Ok(AnyGribFile::V2(grib2)) => match grib2.to_atmosphere(pressure_unit) {
                Ok(vec) => {
                    debug!("  -> {} time steps from GRIB2", vec.len());
                    gfs_series.extend(vec);
                }
                Err(e) => {
                    warn!("Failed to parse as GRIB2: {}", e);
                }
            },
            Ok(AnyGribFile::V1(grib1)) => match grib1.to_atmosphere(pressure_unit, height_unit) {
                Ok(vec) => {
                    debug!("  -> {} time steps from GRIB1", vec.len());
                    gfs_series.extend(vec);
                }
                Err(e) => {
                    error!("Failed to parse as GRIB1: {}", e);
                }
            },
            Err(e) => {
                error!("Failed to open file: {}", e);
            }
        }
    }

    gfs_series.sort_by_key(|(dt, _)| *dt);
    debug!("  Total secondary time steps: {}", gfs_series.len());

    if let Some((_, atmo)) = gfs_series.first() {
        debug!("  GFS pressure levels (Pa): {:?}", atmo.pressure_levels());
    }

    if gfs_series.is_empty() {
        return Err("No valid secondary (GFS) data found".into());
    }

    let mut merged_series: Vec<(DateTime<Utc>, Atmosphere)> = Vec::new();

    for (msm_dt, msm_atmo) in &msm_series {
        let msm_time_secs = msm_dt.timestamp() as f64;

        let gfs_interpolated: Atmosphere = if gfs_series.len() == 1 {
            gfs_series[0].1.clone()
        } else {
            let idx = gfs_series
                .iter()
                .position(|(dt, _)| dt.timestamp() as f64 > msm_time_secs)
                .unwrap_or(gfs_series.len());

            if idx == 0 {
                gfs_series[0].1.clone()
            } else if idx >= gfs_series.len() {
                gfs_series.last().unwrap().1.clone()
            } else {
                let (t_lo, ref_lo) = &gfs_series[idx - 1];
                let (t_hi, ref_hi) = &gfs_series[idx];
                let span = (t_hi.timestamp() - t_lo.timestamp()) as f64;
                let ratio = if span > 0.0 {
                    ((msm_time_secs - t_lo.timestamp() as f64) / span).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                debug!(
                    "  Interpolating GFS for {}: t={}→t={}, ratio={:.3}",
                    msm_dt.format("%H:%M"),
                    t_lo.format("%H:%M"),
                    t_hi.format("%H:%M"),
                    ratio
                );
                ref_lo
                    .interpolate_between(ref_hi, ratio)
                    .unwrap_or_else(|| ref_lo.clone())
            }
        };

        let mut merged = msm_atmo.clone();
        merged.merge_with(&gfs_interpolated, lat, lon);

        if merged_series.is_empty() {
            debug!("  Merged levels (Pa): {:?}", merged.pressure_levels());
            debug!(
                "  Secondary levels (Pa): {:?}",
                merged.secondary_level_set()
            );
            debug!("  Wind profile at launch site (lat={}, lon={}):", lat, lon);
            merged.debug_print_winds_at(lat, lon);
        }

        merged_series.push((*msm_dt, merged));
    }

    merged_series.sort_by_key(|(dt, _)| *dt);
    let selected = if merged_series.len() <= 3 {
        merged_series
    } else {
        let base_idx = merged_series
            .iter()
            .position(|(dt, _)| *dt > launch_time)
            .unwrap_or(merged_series.len());
        let mid_idx = if base_idx == 0 {
            0
        } else if base_idx >= merged_series.len() {
            merged_series.len() - 3
        } else {
            base_idx.saturating_sub(1)
        };
        let end = (mid_idx + 3).min(merged_series.len());
        let start = end.saturating_sub(3);
        merged_series.drain(start..end).collect()
    };

    let base_time = selected.first().map(|(dt, _)| *dt).unwrap_or(launch_time);
    info!(
        "Ensemble result: {} time steps (base: {})",
        selected.len(),
        base_time.format("%Y-%m-%dT%H:%M:%SZ"),
    );

    Ok((selected, base_time))
}
