use chrono::{DateTime, Timelike, Utc};
use serde::Serialize;
use std::fs::File;
use std::io::copy;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

use space_balloon_predictor_rs::dataset::{Dataset, GfsRegion, gfs_filter_url, gefs_filter_url};
use space_balloon_predictor_rs::dataset::gfs::REGION_MARGIN_DEG;
use space_balloon_predictor_rs::dataset::gefs::{GefsMember, GefsResolution};
use space_balloon_predictor_rs::engine::simulation::{AscentParams, SimConfig, Simulator, Trajectory};
use space_balloon_predictor_rs::engine::physics::ascent_coeff_k;
use space_balloon_predictor_rs::geo::coords::{EARTH_RADIUS, Geodetic};
use space_balloon_predictor_rs::grib::{HeightUnit, PressureUnit};

use rand::Rng;
use rand_distr::Normal;
use rayon::prelude::*;

#[derive(Serialize, Clone)]
#[serde(tag = "stage", rename_all = "snake_case")]
enum ProgressEvent {
    DownloadingGfs { current: u32, total: u32 },
    DownloadingGefs { current: u32, total: u32, member: String },
    DecodingGrib,
    RunningSimulation,
    RunningMonteCarlo { current: u32, total: u32 },
}

#[derive(Serialize, Clone)]
struct TrajectoryPoint {
    lat: f64,
    lon: f64,
    alt: f64,
    time: f64,
}

#[derive(Serialize)]
struct SimulationResult {
    ascent_path: Vec<TrajectoryPoint>,
    descent_path: Vec<TrajectoryPoint>,
    stratosphere_duration_s: f64,
    max_altitude: f64,
    landing_lat: f64,
    landing_lon: f64,
    drift_km: f64,
    total_duration_s: f64,
}

#[derive(Serialize)]
struct MonteCarloPoint {
    landing_lat: f64,
    landing_lon: f64,
    burst_altitude: f64,
    /// 平均バースト高度からの偏差 (σ)。アンサンブル等 σが定義できない場合は None
    deviation_sigma: Option<f64>,
}

#[derive(Serialize)]
struct MonteCarloTrajectory {
    ascent_path: Vec<TrajectoryPoint>,
    descent_path: Vec<TrajectoryPoint>,
}

#[derive(Serialize)]
struct MonteCarloResult {
    points: Vec<MonteCarloPoint>,
    mean_landing_lat: f64,
    mean_landing_lon: f64,
    mean_ascent_path: Vec<TrajectoryPoint>,
    mean_descent_path: Vec<TrajectoryPoint>,
    trajectories: Vec<MonteCarloTrajectory>,
}

/// 成層圏（高度11,000m以上）に滞在した時間（秒）を返す
fn stratosphere_duration_s(trajectory: &Trajectory) -> f64 {
    const TROPOPAUSE_M: f64 = 11_000.0;
    let mut enter_time: Option<f64> = None;
    let mut total = 0.0;

    for w in trajectory.states.windows(2) {
        let a = &w[0];
        let b = &w[1];
        let a_above = a.alt >= TROPOPAUSE_M;
        let b_above = b.alt >= TROPOPAUSE_M;

        if a_above && enter_time.is_none() {
            enter_time = Some(a.time);
        }

        if let Some(t0) = enter_time {
            if !b_above {
                total += b.time - t0;
                enter_time = None;
            }
        }
    }

    if let Some(t0) = enter_time {
        if let Some(last) = trajectory.states.last() {
            total += last.time - t0;
        }
    }

    total
}

fn download_gfs_file(
    work_dir: &Path,
    date_str: &str,
    cycle_str: &str,
    forecast_hour: u32,
    region: &GfsRegion,
) -> Result<String, String> {
    let local_path = work_dir.join(format!(
        "gfs_{}_{}_f{:03}_{}.grib2",
        date_str,
        cycle_str,
        forecast_hour,
        region.cache_key()
    ));

    if local_path.exists() {
        println!(
            "  File '{}' already exists locally. Skipping download.",
            local_path.display()
        );
        return Ok(local_path.to_string_lossy().into_owned());
    }

    let url = gfs_filter_url(date_str, cycle_str, forecast_hour, region);

    println!("  Downloading '{}' from NOAA NOMADS...", url);
    let mut response = reqwest::blocking::get(&url).map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download GRIB file. HTTP Status: {}.\n\
             (Note: NOAA only stores the last 10 days of forecast data. Older dates will result in errors.)",
            response.status()
        ));
    }

    let mut dest = File::create(&local_path).map_err(|e| e.to_string())?;
    copy(&mut response, &mut dest).map_err(|e| e.to_string())?;
    println!("  Saved successfully to '{}'.", local_path.display());

    Ok(local_path.to_string_lossy().into_owned())
}

fn download_gfs_series(
    app: &AppHandle,
    work_dir: &Path,
    gfs_run_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    launch_lat: f64,
    launch_lon: f64,
) -> Result<Vec<String>, String> {
    let cycle_hour = (gfs_run_time.hour() / 6) * 6;
    let rounded_gfs_time = gfs_run_time
        .date_naive()
        .and_hms_opt(cycle_hour, 0, 0)
        .unwrap()
        .and_utc();

    let total_diff_seconds = launch_time
        .signed_duration_since(rounded_gfs_time)
        .num_seconds();
    if total_diff_seconds < 0 {
        return Err(
            "Error: Launch time cannot be before the GFS model initialization run time.".into(),
        );
    }

    let diff_hours = total_diff_seconds as f64 / 3600.0;
    let forecast_hour_low = ((diff_hours / 3.0).floor() as u32) * 3;
    let launch_offset_hours = diff_hours - (forecast_hour_low as f64);

    let date_str = rounded_gfs_time.format("%Y%m%d").to_string();
    let cycle_str = rounded_gfs_time.format("%H").to_string();
    let region = GfsRegion::around(launch_lat, launch_lon, REGION_MARGIN_DEG);

    println!(
        "Downloading GFS (f{:03}→f{:03}→f{:03}), offset {:.2}h, region lat[{}, {}] lon[{}, {}]",
        forecast_hour_low,
        forecast_hour_low + 3,
        forecast_hour_low + 6,
        launch_offset_hours,
        region.bottom_lat,
        region.top_lat,
        region.left_lon,
        region.right_lon
    );

    let forecast_hours = [
        forecast_hour_low,
        forecast_hour_low + 3,
        forecast_hour_low + 6,
    ];
    let total_files = forecast_hours.len() as u32;

    let mut paths = Vec::with_capacity(forecast_hours.len());
    for (i, forecast_hour) in forecast_hours.iter().enumerate() {
        let _ = app.emit(
            "progress",
            ProgressEvent::DownloadingGfs {
                current: (i + 1) as u32,
                total: total_files,
            },
        );
        paths.push(download_gfs_file(
            work_dir,
            &date_str,
            &cycle_str,
            *forecast_hour,
            &region,
        )?);
    }

    Ok(paths)
}

fn download_gefs_file(
    work_dir: &Path,
    date_str: &str,
    cycle_str: &str,
    forecast_hour: u32,
    member: GefsMember,
    region: &GfsRegion,
) -> Result<String, String> {
    let member_key = match member {
        GefsMember::Control => "c00".to_string(),
        GefsMember::Perturbed(n) => format!("p{:02}", n),
        GefsMember::Mean => "avg".to_string(),
        GefsMember::Spread => "spr".to_string(),
    };
    let local_path = work_dir.join(format!(
        "gefs_{}_{}_{}_f{:03}_{}.grib2",
        date_str,
        cycle_str,
        member_key,
        forecast_hour,
        region.cache_key()
    ));

    if local_path.exists() {
        println!(
            "  GEFS file '{}' already exists locally. Skipping download.",
            local_path.display()
        );
        return Ok(local_path.to_string_lossy().into_owned());
    }

    let url = gefs_filter_url(
        date_str,
        cycle_str,
        forecast_hour,
        member,
        region,
        GefsResolution::Primary0p5,
    );

    println!("  Downloading GEFS '{}' from NOAA NOMADS...", url);
    let mut response = reqwest::blocking::get(&url).map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download GEFS GRIB file. HTTP Status: {}.\n\
             (Note: NOAA only stores the last 10 days of forecast data. Older dates will result in errors.)",
            response.status()
        ));
    }

    let mut dest = File::create(&local_path).map_err(|e| e.to_string())?;
    copy(&mut response, &mut dest).map_err(|e| e.to_string())?;
    println!("  Saved GEFS to '{}'.", local_path.display());

    Ok(local_path.to_string_lossy().into_owned())
}

fn download_gefs_member_series(
    work_dir: &Path,
    gefs_run_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    launch_lat: f64,
    launch_lon: f64,
    member: GefsMember,
) -> Result<Vec<String>, String> {
    let cycle_hour = (gefs_run_time.hour() / 6) * 6;
    let rounded_time = gefs_run_time
        .date_naive()
        .and_hms_opt(cycle_hour, 0, 0)
        .unwrap()
        .and_utc();

    let total_diff_seconds = launch_time
        .signed_duration_since(rounded_time)
        .num_seconds();
    if total_diff_seconds < 0 {
        return Err(
            "Error: Launch time cannot be before the GEFS model initialization run time.".into(),
        );
    }

    let diff_hours = total_diff_seconds as f64 / 3600.0;
    let forecast_hour_low = ((diff_hours / 3.0).floor() as u32) * 3;

    let date_str = rounded_time.format("%Y%m%d").to_string();
    let cycle_str = rounded_time.format("%H").to_string();
    let region = GfsRegion::around(launch_lat, launch_lon, REGION_MARGIN_DEG);

    println!(
        "Downloading GEFS member {} (f{:03}→f{:03}→f{:03}), region lat[{}, {}] lon[{}, {}]",
        member,
        forecast_hour_low,
        forecast_hour_low + 3,
        forecast_hour_low + 6,
        region.bottom_lat,
        region.top_lat,
        region.left_lon,
        region.right_lon
    );

    let forecast_hours = [
        forecast_hour_low,
        forecast_hour_low + 3,
        forecast_hour_low + 6,
    ];

    let mut paths = Vec::with_capacity(3);
    for &fh in &forecast_hours {
        paths.push(download_gefs_file(
            work_dir,
            &date_str,
            &cycle_str,
            fh,
            member,
            &region,
        )?);
    }

    Ok(paths)
}

fn trajectory_to_result(trajectory: &Trajectory, launch_site: Geodetic) -> SimulationResult {
    let burst_idx = trajectory
        .states
        .iter()
        .position(|s| s.is_burst)
        .unwrap_or(trajectory.states.len());

    let ascent_path: Vec<TrajectoryPoint> = trajectory.states[..=burst_idx.min(trajectory.states.len() - 1)]
        .iter()
        .map(|s| TrajectoryPoint {
            lat: s.lat,
            lon: s.lon,
            alt: s.alt,
            time: s.time,
        })
        .collect();

    let descent_path: Vec<TrajectoryPoint> = if burst_idx < trajectory.states.len() {
        trajectory.states[burst_idx..]
            .iter()
            .map(|s| TrajectoryPoint {
                lat: s.lat,
                lon: s.lon,
                alt: s.alt,
                time: s.time,
            })
            .collect()
    } else {
        Vec::new()
    };

    let max_altitude = trajectory
        .states
        .iter()
        .map(|s| s.alt)
        .fold(0.0, f64::max);

    let stratosphere_duration = stratosphere_duration_s(trajectory);

    let (landing_lat, landing_lon, total_duration_s, drift_km) =
        if let Some(last) = trajectory.states.last() {
            let d_lat = (last.lat - launch_site.lat).to_radians();
            let d_lon = (last.lon - launch_site.lon).to_radians();
            let lat_avg = ((last.lat + launch_site.lat) / 2.0).to_radians();
            let drift = ((d_lat * EARTH_RADIUS).powi(2)
                + (d_lon * EARTH_RADIUS * lat_avg.cos()).powi(2))
                .sqrt()
                / 1000.0;
            (last.lat, last.lon, last.time, drift)
        } else {
            (0.0, 0.0, 0.0, 0.0)
        };

    SimulationResult {
        ascent_path,
        descent_path,
        stratosphere_duration_s: stratosphere_duration,
        max_altitude,
        landing_lat,
        landing_lon,
        drift_km,
        total_duration_s,
    }
}

/// 気球種別＋総重量＋目標上昇速度から上昇パラメータを組み立てる
fn ascent_params(
    ascent_rate: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
) -> Result<AscentParams, String> {
    let coeff_k = ascent_coeff_k(balloon_class_g).ok_or_else(|| {
        format!(
            "Unknown balloon class: {}g. Choose from 1000, 1500, 2000, 3000.",
            balloon_class_g
        )
    })?;
    if !(gross_mass_kg > 0.0) {
        return Err("gross_mass_kg must be positive".into());
    }
    if !(ascent_rate > 0.0) {
        return Err("ascent_rate must be positive".into());
    }
    Ok(AscentParams {
        gross_mass_kg,
        target_rate_m_s: ascent_rate,
        coeff_k,
        burst_volume_m3: None,
    })
}

#[tauri::command]
async fn run_simulation(
    app: AppHandle,
    launch_lat: f64,
    launch_lon: f64,
    launch_alt: f64,
    gfs_run_time: String,
    launch_time: String,
    ascent_rate: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    burst_altitude: f64,
) -> Result<SimulationResult, String> {
    let gfs_run: DateTime<Utc> = gfs_run_time.parse().map_err(|e| format!("Invalid gfs_run_time: {}", e))?;
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<SimulationResult, String> {
        let work_dir = Path::new(".").to_path_buf();

        let file_paths =
            download_gfs_series(&app, &work_dir, gfs_run, launch, launch_lat, launch_lon)?;

        let _ = app.emit("progress", ProgressEvent::DecodingGrib);

        let dataset = Dataset::from_grib_files(
            &file_paths,
            launch,
            PressureUnit::Pascal,
            HeightUnit::DeciMeters,
        )
        .map_err(|e| format!("Failed to load GRIB data: {}", e))?;

        let _ = app.emit("progress", ProgressEvent::RunningSimulation);

        let config = SimConfig {
            launch_site,
            ascent,
            ground_descend_rate_m_s: descent_rate,
            burst_altitude_m: burst_altitude,
            dt: 5.0,
        };

        println!("Running simulation...");
        let simulator = Simulator::new(config, dataset, launch);
        let trajectory = simulator.run();

        Ok(trajectory_to_result(&trajectory, launch_site))
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
async fn run_monte_carlo(
    app: AppHandle,
    launch_lat: f64,
    launch_lon: f64,
    launch_alt: f64,
    gfs_run_time: String,
    launch_time: String,
    ascent_rate: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    burst_altitude_mean: f64,
    burst_altitude_std: f64,
    num_samples: u32,
) -> Result<MonteCarloResult, String> {
    let gfs_run: DateTime<Utc> = gfs_run_time.parse().map_err(|e| format!("Invalid gfs_run_time: {}", e))?;
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<MonteCarloResult, String> {
        let work_dir = Path::new(".").to_path_buf();

        let file_paths =
            download_gfs_series(&app, &work_dir, gfs_run, launch, launch_lat, launch_lon)?;

        let _ = app.emit("progress", ProgressEvent::DecodingGrib);

        let dataset = Dataset::from_grib_files(
            &file_paths,
            launch,
            PressureUnit::Pascal,
            HeightUnit::DeciMeters,
        )
        .map_err(|e| format!("Failed to load GRIB data: {}", e))?;

        let use_scatter = burst_altitude_std > 0.0;

        let sample_count = if use_scatter { num_samples } else { 1 };

        let _ = app.emit(
            "progress",
            ProgressEvent::RunningMonteCarlo {
                current: 0,
                total: sample_count,
            },
        );

        // サンプルするバースト高度を事前に生成
        let burst_altitudes: Vec<f64> = if use_scatter {
            let normal = Normal::new(burst_altitude_mean, burst_altitude_std)
                .map_err(|e| format!("Invalid distribution parameters: {}", e))?;
            let mut rng = rand::thread_rng();
            (0..sample_count)
                .map(|_| rng.sample(normal).max(0.0))
                .collect()
        } else {
            vec![burst_altitude_mean]
        };

        // Rayon で並列シミュレーション
        let completed = Arc::new(AtomicUsize::new(0));
        let completed_inner = completed.clone();
        let app_inner = app.clone();

        let (points, trajectories): (Vec<MonteCarloPoint>, Vec<MonteCarloTrajectory>) = burst_altitudes
            .par_iter()
            .map(|&sampled_burst| {
                let deviation = if use_scatter {
                    (sampled_burst - burst_altitude_mean) / burst_altitude_std
                } else {
                    0.0
                };

                let config = SimConfig {
                    launch_site,
                    ascent,
                    ground_descend_rate_m_s: descent_rate,
                    burst_altitude_m: sampled_burst,
                    dt: 5.0,
                };

                let simulator = Simulator::new(config, dataset.clone(), launch);
                let trajectory = simulator.run();
                let result = trajectory_to_result(&trajectory, launch_site);

                let mc_point = MonteCarloPoint {
                    landing_lat: result.landing_lat,
                    landing_lon: result.landing_lon,
                    burst_altitude: sampled_burst,
                    deviation_sigma: Some(deviation),
                };

                let mc_traj = MonteCarloTrajectory {
                    ascent_path: result.ascent_path,
                    descent_path: result.descent_path,
                };

                let done = completed_inner.fetch_add(1, Ordering::Relaxed) + 1;
                let _ = app_inner.emit(
                    "progress",
                    ProgressEvent::RunningMonteCarlo {
                        current: done as u32,
                        total: sample_count,
                    },
                );

                (mc_point, mc_traj)
            })
            .unzip();

        let sum_lat: f64 = points.iter().map(|p| p.landing_lat).sum();
        let sum_lon: f64 = points.iter().map(|p| p.landing_lon).sum();

        // 平均バースト高度の経路を計算
        let mean_config = SimConfig {
            launch_site,
            ascent,
            ground_descend_rate_m_s: descent_rate,
            burst_altitude_m: burst_altitude_mean,
            dt: 5.0,
        };
        let mean_sim = Simulator::new(mean_config, dataset, launch);
        let mean_trajectory = mean_sim.run();
        let mean_result = trajectory_to_result(&mean_trajectory, launch_site);

        let n = sample_count as f64;
        Ok(MonteCarloResult {
            points,
            mean_landing_lat: if use_scatter { sum_lat / n } else { mean_result.landing_lat },
            mean_landing_lon: if use_scatter { sum_lon / n } else { mean_result.landing_lon },
            mean_ascent_path: mean_result.ascent_path,
            mean_descent_path: mean_result.descent_path,
            trajectories,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
async fn run_gefs_simulation(
    app: AppHandle,
    launch_lat: f64,
    launch_lon: f64,
    launch_alt: f64,
    gefs_run_time: String,
    launch_time: String,
    ascent_rate: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    burst_altitude_mean: f64,
    burst_altitude_std: f64,
    num_members: u32,
    num_samples: u32,
) -> Result<MonteCarloResult, String> {
    let gefs_run: DateTime<Utc> = gefs_run_time.parse().map_err(|e| format!("Invalid gefs_run_time: {}", e))?;
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<MonteCarloResult, String> {
        let work_dir = Path::new(".").to_path_buf();

        let num = num_members.min(31);
        let mut members_to_run = Vec::new();
        members_to_run.push(GefsMember::Control);
        for i in 1..num {
            members_to_run.push(GefsMember::Perturbed(i as u8));
        }

        let use_scatter = burst_altitude_std > 0.0;
        let sample_count = if use_scatter { num_samples.max(1) } else { 1 };

        let member_count = members_to_run.len() as u32;
        let total_sims = member_count * sample_count;

        let normal = if use_scatter {
            Some(
                Normal::new(burst_altitude_mean, burst_altitude_std)
                    .map_err(|e| format!("Invalid distribution parameters: {}", e))?,
            )
        } else {
            None
        };

        let completed = Arc::new(AtomicUsize::new(0));

        let _ = app.emit(
            "progress",
            ProgressEvent::RunningMonteCarlo {
                current: 0,
                total: total_sims,
            },
        );

        let mut points: Vec<MonteCarloPoint> = Vec::with_capacity(total_sims as usize);
        let mut trajectories: Vec<MonteCarloTrajectory> = Vec::with_capacity(total_sims as usize);
        let mut mean_ascent_path: Option<Vec<TrajectoryPoint>> = None;
        let mut mean_descent_path: Option<Vec<TrajectoryPoint>> = None;

        for (idx, &member) in members_to_run.iter().enumerate() {
            let _ = app.emit(
                "progress",
                ProgressEvent::DownloadingGefs {
                    current: (idx + 1) as u32,
                    total: member_count,
                    member: member.to_string(),
                },
            );

            let file_paths = download_gefs_member_series(
                &work_dir,
                gefs_run,
                launch,
                launch_lat,
                launch_lon,
                member,
            )?;

            let _ = app.emit("progress", ProgressEvent::DecodingGrib);

            let dataset = Dataset::from_grib_files(
                &file_paths,
                launch,
                PressureUnit::Pascal,
                HeightUnit::DeciMeters,
            )
            .map_err(|e| format!("Failed to load GEFS GRIB data for member {}: {}", member, e))?;

            let burst_altitudes: Vec<f64> = if use_scatter {
                let mut rng = rand::thread_rng();
                (0..sample_count)
                    .map(|_| rng.sample(normal.as_ref().unwrap()).max(0.0))
                    .collect()
            } else {
                vec![burst_altitude_mean]
            };

            let completed_inner = completed.clone();
            let app_inner = app.clone();

            let (member_points, member_trajectories): (
                Vec<MonteCarloPoint>,
                Vec<MonteCarloTrajectory>,
            ) = burst_altitudes
                .par_iter()
                .map(|&sampled_burst| {
                    let config = SimConfig {
                        launch_site,
                        ascent,
                        ground_descend_rate_m_s: descent_rate,
                        burst_altitude_m: sampled_burst,
                        dt: 5.0,
                    };

                    let simulator = Simulator::new(config, dataset.clone(), launch);
                    let trajectory = simulator.run();
                    let result = trajectory_to_result(&trajectory, launch_site);

                    let mc_point = MonteCarloPoint {
                        landing_lat: result.landing_lat,
                        landing_lon: result.landing_lon,
                        burst_altitude: sampled_burst,
                        deviation_sigma: None,
                    };

                    let mc_traj = MonteCarloTrajectory {
                        ascent_path: result.ascent_path,
                        descent_path: result.descent_path,
                    };

                    let done = completed_inner.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app_inner.emit(
                        "progress",
                        ProgressEvent::RunningMonteCarlo {
                            current: done as u32,
                            total: total_sims,
                        },
                    );

                    (mc_point, mc_traj)
                })
                .unzip();

            if idx == 0 {
                if use_scatter {
                    let mean_config = SimConfig {
                        launch_site,
                        ascent,
                        ground_descend_rate_m_s: descent_rate,
                        burst_altitude_m: burst_altitude_mean,
                        dt: 5.0,
                    };
                    let mean_sim = Simulator::new(mean_config, dataset.clone(), launch);
                    let mean_result = trajectory_to_result(&mean_sim.run(), launch_site);
                    mean_ascent_path = Some(mean_result.ascent_path);
                    mean_descent_path = Some(mean_result.descent_path);
                } else {
                    mean_ascent_path = Some(member_trajectories[0].ascent_path.clone());
                    mean_descent_path = Some(member_trajectories[0].descent_path.clone());
                }
            }

            points.extend(member_points);
            trajectories.extend(member_trajectories);
        }

        let n = points.len() as f64;
        let mean_lat = points.iter().map(|p| p.landing_lat).sum::<f64>() / n;
        let mean_lon = points.iter().map(|p| p.landing_lon).sum::<f64>() / n;

        Ok(MonteCarloResult {
            points,
            mean_landing_lat: mean_lat,
            mean_landing_lon: mean_lon,
            mean_ascent_path: mean_ascent_path.unwrap_or_default(),
            mean_descent_path: mean_descent_path.unwrap_or_default(),
            trajectories,
        })
    })
    .await
    .map_err(|e| format!("Task failed: {}", e))?
}

#[tauri::command]
fn save_kml(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| format!("Failed to write KML file: {}", e))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![run_simulation, run_monte_carlo, run_gefs_simulation, save_kml])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
