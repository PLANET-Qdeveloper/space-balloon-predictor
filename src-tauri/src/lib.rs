use chrono::{DateTime, Duration, NaiveDate, Timelike, Utc};
use serde::Serialize;
use std::fs::File;
use std::io::{copy, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;
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
    terrain_fallback_used: bool,
    model: String,
    model_run_time_utc: String,
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
    terrain_fallback_used: bool,
    landing_lat: f64,
    landing_lon: f64,
    ascent_rate_m_s: f64,
    descent_rate_m_s: f64,
    burst_altitude: f64,
    /// バースト高度の標準偏差で算出した偏差 (σ)。高度をサンプリングしない場合は None
    deviation_sigma: Option<f64>,
}

#[derive(Serialize)]
struct MonteCarloTrajectory {
    ascent_path: Vec<TrajectoryPoint>,
    descent_path: Vec<TrajectoryPoint>,
}

#[derive(Serialize)]
struct MonteCarloResult {
    model: String,
    model_run_time_utc: String,
    points: Vec<MonteCarloPoint>,
    mean_landing_lat: f64,
    mean_landing_lon: f64,
    mean_ascent_path: Vec<TrajectoryPoint>,
    mean_descent_path: Vec<TrajectoryPoint>,
    trajectories: Vec<MonteCarloTrajectory>,
}

#[derive(Clone, Copy)]
struct MonteCarloSample {
    ascent_rate_m_s: f64,
    descent_rate_m_s: f64,
    burst_altitude_m: f64,
}

fn normal_distribution(
    mean: f64,
    standard_deviation: f64,
    label: &str,
) -> Result<Option<Normal<f64>>, String> {
    if !mean.is_finite() {
        return Err(format!("{label} mean must be finite"));
    }
    if !standard_deviation.is_finite() || standard_deviation < 0.0 {
        return Err(format!("{label} standard deviation must be non-negative"));
    }
    if standard_deviation == 0.0 {
        return Ok(None);
    }

    Normal::new(mean, standard_deviation)
        .map(Some)
        .map_err(|e| format!("Invalid {label} distribution: {e}"))
}

fn sample_parameter<R: Rng + ?Sized>(
    rng: &mut R,
    mean: f64,
    distribution: Option<&Normal<f64>>,
    minimum: f64,
) -> f64 {
    let Some(normal) = distribution else {
        return mean;
    };

    loop {
        let sample: f64 = rng.sample(normal);
        if sample.is_finite() && sample >= minimum {
            return sample;
        }
    }
}

#[cfg(test)]
mod monte_carlo_parameter_tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    fn zero_standard_deviation_keeps_the_mean() {
        let distribution = normal_distribution(6.0, 0.0, "ascent rate").unwrap();
        let mut rng = StdRng::seed_from_u64(1);

        for _ in 0..4 {
            assert_eq!(
                sample_parameter(&mut rng, 6.0, distribution.as_ref(), f64::EPSILON),
                6.0
            );
        }
    }

    #[test]
    fn negative_standard_deviation_is_rejected() {
        assert!(normal_distribution(6.0, -0.1, "ascent rate").is_err());
    }

    #[test]
    fn rate_samples_stay_positive() {
        let distribution = normal_distribution(1.0, 2.0, "ascent rate").unwrap();
        let mut rng = StdRng::seed_from_u64(42);

        for _ in 0..32 {
            let sample = sample_parameter(&mut rng, 1.0, distribution.as_ref(), 0.1);
            assert!(sample >= 0.1);
        }
    }
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

const MODEL_CYCLE_HOURS: u32 = 6;
const FALLBACK_MODEL_RUN_AVAILABILITY_LAG_HOURS: i64 = 6;
const MAX_GFS_RUN_BACKTRACKS: usize = 8;
const GFS_AVAILABILITY_URL: &str = "https://nomads.ncep.noaa.gov/gribfilter.php?ds=gfs_0p25";

/// 指定時刻以前で最も近いモデルサイクルに丸める。
fn floor_model_cycle(time: DateTime<Utc>) -> DateTime<Utc> {
    let cycle_hour = (time.hour() / MODEL_CYCLE_HOURS) * MODEL_CYCLE_HOURS;
    time.date_naive()
        .and_hms_opt(cycle_hour, 0, 0)
        .expect("model cycle hour must be valid")
        .and_utc()
}

/// NOMADSのavailable一覧を取得できない場合の保守的なフォールバック。
fn select_model_run_time(now: DateTime<Utc>, launch_time: DateTime<Utc>) -> DateTime<Utc> {
    let latest_completed = floor_model_cycle(
        now - Duration::hours(FALLBACK_MODEL_RUN_AVAILABILITY_LAG_HOURS),
    );
    let launch_cycle = floor_model_cycle(launch_time);

    if launch_cycle < latest_completed {
        launch_cycle
    } else {
        latest_completed
    }
}

/// NOMADSのGFSページに掲載されている日付を抽出する。
fn parse_available_gfs_dates(html: &str) -> Vec<DateTime<Utc>> {
    let mut dates = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = html[cursor..].find("gfs.") {
        let date_start = cursor + relative_start + 4;
        let date_end = date_start + 8;
        if date_end <= html.len() {
            let date_text = &html[date_start..date_end];
            if date_text.bytes().all(|byte| byte.is_ascii_digit()) {
                if let Ok(date) = NaiveDate::parse_from_str(date_text, "%Y%m%d") {
                    if let Some(datetime) = date.and_hms_opt(0, 0, 0) {
                        dates.push(datetime.and_utc());
                    }
                }
            }
        }
        cursor = date_start;
    }

    dates.sort();
    dates.dedup();
    dates
}

fn parse_available_gfs_cycles(html: &str) -> Vec<u32> {
    let cycle_start = html.find("Cycle").unwrap_or(0);
    let cycle_end = html[cycle_start..]
        .find("Subdirectory")
        .map(|offset| cycle_start + offset)
        .unwrap_or(html.len());
    let section = &html[cycle_start..cycle_end];

    let mut cycles = Vec::new();
    let mut values = extract_quoted_values(section, "click_subdir(1,");
    values.extend(extract_quoted_values(section, "value="));

    for value in values {
        if let Ok(cycle) = value.parse::<u32>() {
            if cycle < 24 && cycle % MODEL_CYCLE_HOURS == 0 {
                cycles.push(cycle);
            }
        }
    }

    cycles.sort();
    cycles.dedup();
    cycles
}

fn extract_quoted_values(text: &str, marker: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find(marker) {
        let mut value_start = cursor + relative_start + marker.len();
        while let Some(byte) = text.as_bytes().get(value_start) {
            if !byte.is_ascii_whitespace() {
                break;
            }
            value_start += 1;
        }

        let Some(quote) = text[value_start..].chars().next() else {
            break;
        };
        if quote != '\'' && quote != '"' {
            cursor = value_start;
            continue;
        }

        let text_start = value_start + quote.len_utf8();
        let Some(relative_end) = text[text_start..].find(quote) else {
            break;
        };
        let text_end = text_start + relative_end;
        values.push(text[text_start..text_end].to_string());
        cursor = text_end + quote.len_utf8();
    }

    values
}

fn available_gfs_runs(html: &str) -> Vec<DateTime<Utc>> {
    let dates = parse_available_gfs_dates(html);
    let cycles = parse_available_gfs_cycles(html);

    dates
        .into_iter()
        .flat_map(|date| {
            cycles
                .iter()
                .map(move |&cycle| date + Duration::hours(cycle as i64))
        })
        .collect()
}

fn select_available_gfs_run_time(now: DateTime<Utc>, launch_time: DateTime<Utc>) -> DateTime<Utc> {
    let fallback = select_model_run_time(now, launch_time);
    let response = reqwest::blocking::Client::builder()
        .timeout(StdDuration::from_secs(15))
        .user_agent("space-balloon-predictor")
        .build()
        .and_then(|client| client.get(GFS_AVAILABILITY_URL).send())
        .and_then(|response| response.error_for_status())
        .and_then(|response| response.text());

    let html = match response {
        Ok(html) => html,
        Err(error) => {
            eprintln!(
                "Could not query GFS availability from NOMADS: {}. Falling back to the conservative run selection.",
                error
            );
            return fallback;
        }
    };

    let latest_allowed = floor_model_cycle(if launch_time < now { launch_time } else { now });
    let selected = available_gfs_runs(&html)
        .into_iter()
        .filter(|run| *run <= latest_allowed)
        .max();

    match selected {
        Some(run) => {
            println!(
                "NOMADS reports latest usable GFS model run {} for launch {}",
                run.to_rfc3339(),
                launch_time.to_rfc3339()
            );
            run
        }
        None => {
            eprintln!(
                "Could not find a usable GFS run in the NOMADS availability page. Falling back to {}.",
                fallback.to_rfc3339()
            );
            fallback
        }
    }
}

#[cfg(test)]
mod model_run_tests {
    use super::*;

    fn utc(value: &str) -> DateTime<Utc> {
        value.parse().expect("valid UTC timestamp")
    }

    #[test]
    fn future_launch_uses_a_run_that_already_exists() {
        let now = utc("2026-09-09T05:00:00Z");
        let launch = utc("2026-09-16T04:00:00Z");

        assert_eq!(
            select_model_run_time(now, launch),
            utc("2026-09-08T18:00:00Z")
        );
    }

    #[test]
    fn past_launch_does_not_use_a_run_after_launch() {
        let now = utc("2026-09-09T14:00:00Z");
        let launch = utc("2026-09-09T04:00:00Z");

        assert_eq!(
            select_model_run_time(now, launch),
            utc("2026-09-09T00:00:00Z")
        );
    }

    #[test]
    fn parses_available_gfs_dates_and_cycles() {
        let html = r#"
            <span onClick="click_subdir(0, 'gfs.20260909', this)">gfs.20260909</span>
            <span onClick="click_subdir(0, 'gfs.20260908', this)">gfs.20260908</span>
            <h2>Cycle:</h2>
            <span onClick="click_subdir(1, '18', this)">18</span>
            <span onClick="click_subdir(1, '12', this)">12</span>
            <span onClick="click_subdir(1, '06', this)">06</span>
            <span onClick="click_subdir(1, '00', this)">00</span>
            <h2>Subdirectory:</h2>
        "#;

        assert_eq!(
            parse_available_gfs_dates(html),
            vec![utc("2026-09-08T00:00:00Z"), utc("2026-09-09T00:00:00Z")]
        );
        assert_eq!(parse_available_gfs_cycles(html), vec![0, 6, 12, 18]);
    }
}

fn is_valid_grib_cache_file(path: &Path) -> bool {
    let metadata = match path.metadata() {
        Ok(metadata) => metadata,
        Err(_) => return false,
    };
    if metadata.len() < 4 {
        return false;
    }

    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic).is_ok() && magic == *b"GRIB"
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

    if local_path.exists() && is_valid_grib_cache_file(&local_path) {
        println!(
            "  File '{}' already exists locally. Skipping download.",
            local_path.display()
        );
        return Ok(local_path.to_string_lossy().into_owned());
    }
    if local_path.exists() {
        println!(
            "  Cached file '{}' is not a valid GRIB file. Refreshing it.",
            local_path.display()
        );
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

    let mut magic = [0u8; 4];
    response
        .read_exact(&mut magic)
        .map_err(|e| format!("Failed to read GFS response: {}", e))?;
    if magic != *b"GRIB" {
        let mut preview = [0u8; 4092];
        let preview_len = response.read(&mut preview).unwrap_or(0);
        let mut response_preview = String::from_utf8_lossy(&magic).to_string();
        response_preview.push_str(&String::from_utf8_lossy(&preview[..preview_len]));
        let detail = response_preview
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("NOMADS returned a non-GRIB response")
            .trim();
        return Err(format!(
            "GFS forecast f{:03} is not available: {}",
            forecast_hour, detail
        ));
    }

    let mut dest = File::create(&local_path).map_err(|e| e.to_string())?;
    dest.write_all(&magic).map_err(|e| e.to_string())?;
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
) -> Result<(DateTime<Utc>, Vec<String>), String> {
    let initial_run_time = floor_model_cycle(gfs_run_time);
    let mut candidate_run_time = initial_run_time;
    let mut last_error = None;

    for attempt in 0..=MAX_GFS_RUN_BACKTRACKS {
        match download_gfs_series_for_run(
            app,
            work_dir,
            candidate_run_time,
            launch_time,
            launch_lat,
            launch_lon,
        ) {
            Ok(paths) => {
                if candidate_run_time != initial_run_time {
                    println!(
                        "GFS run {} was not usable; using previous available run {}",
                        initial_run_time.to_rfc3339(),
                        candidate_run_time.to_rfc3339()
                    );
                }
                return Ok((candidate_run_time, paths));
            }
            Err(error) => {
                println!(
                    "GFS run {} is not usable: {}",
                    candidate_run_time.to_rfc3339(),
                    error
                );
                last_error = Some(error);
            }
        }

        if attempt < MAX_GFS_RUN_BACKTRACKS {
            candidate_run_time -= Duration::hours(MODEL_CYCLE_HOURS as i64);
            println!(
                "Trying previous GFS run {}...",
                candidate_run_time.to_rfc3339()
            );
        }
    }

    Err(format!(
        "Could not find a usable GFS run after checking up to {} previous cycles. Last error: {}",
        MAX_GFS_RUN_BACKTRACKS,
        last_error.unwrap_or_else(|| "unknown download error".to_string())
    ))
}

fn download_gfs_series_for_run(
    app: &AppHandle,
    work_dir: &Path,
    rounded_gfs_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    launch_lat: f64,
    launch_lon: f64,
) -> Result<Vec<String>, String> {

    let total_diff_seconds = launch_time
        .signed_duration_since(rounded_gfs_time)
        .num_seconds();
    if total_diff_seconds < 0 {
        return Err(
            "Error: Launch time cannot be before the GFS model initialization run time.".into(),
        );
    }

    let diff_hours = total_diff_seconds as f64 / 3600.0;
    if diff_hours > 384.0 {
        return Err(format!(
            "Launch time is {:.1} hours after the GFS run; GFS forecasts are available for up to 384 hours.",
            diff_hours
        ));
    }

    // f384が最終時刻なので、終端付近でも3本の時刻を取得できるようにする。
    let forecast_hour_low = (((diff_hours / 3.0).floor() as u32) * 3).min(378);
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
    let rounded_time = floor_model_cycle(gefs_run_time);

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

fn trajectory_to_result(
    trajectory: &Trajectory,
    launch_site: Geodetic,
    model: &str,
    model_run_time_utc: &str,
) -> SimulationResult {
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
        terrain_fallback_used: trajectory.terrain_fallback_used,
        model: model.to_string(),
        model_run_time_utc: model_run_time_utc.to_string(),
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
    launch_time: String,
    ascent_rate: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    burst_altitude: f64,
) -> Result<SimulationResult, String> {
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<SimulationResult, String> {
        let gfs_run = select_available_gfs_run_time(Utc::now(), launch);
        let work_dir = Path::new(".").to_path_buf();

        let (gfs_run, file_paths) =
            download_gfs_series(&app, &work_dir, gfs_run, launch, launch_lat, launch_lon)?;
        println!(
            "Using GFS model run {} for launch {}",
            gfs_run.to_rfc3339(),
            launch.to_rfc3339()
        );
        let model_run_time_utc = gfs_run.to_rfc3339();

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

        Ok(trajectory_to_result(
            &trajectory,
            launch_site,
            "GFS",
            &model_run_time_utc,
        ))
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
    launch_time: String,
    ascent_rate: f64,
    ascent_rate_std: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    descent_rate_std: f64,
    burst_altitude_mean: f64,
    burst_altitude_std: f64,
    num_samples: u32,
) -> Result<MonteCarloResult, String> {
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<MonteCarloResult, String> {
        let gfs_run = select_available_gfs_run_time(Utc::now(), launch);
        let work_dir = Path::new(".").to_path_buf();

        let (gfs_run, file_paths) =
            download_gfs_series(&app, &work_dir, gfs_run, launch, launch_lat, launch_lon)?;
        println!(
            "Using GFS model run {} for Monte Carlo launch {}",
            gfs_run.to_rfc3339(),
            launch.to_rfc3339()
        );
        let model_run_time_utc = gfs_run.to_rfc3339();

        let _ = app.emit("progress", ProgressEvent::DecodingGrib);

        let dataset = Dataset::from_grib_files(
            &file_paths,
            launch,
            PressureUnit::Pascal,
            HeightUnit::DeciMeters,
        )
        .map_err(|e| format!("Failed to load GRIB data: {}", e))?;

        let ascent_rate_distribution =
            normal_distribution(ascent_rate, ascent_rate_std, "ascent rate")?;
        let descent_rate_distribution =
            normal_distribution(descent_rate, descent_rate_std, "descent rate")?;
        let burst_altitude_distribution =
            normal_distribution(burst_altitude_mean, burst_altitude_std, "burst altitude")?;
        let use_scatter =
            ascent_rate_std > 0.0 || descent_rate_std > 0.0 || burst_altitude_std > 0.0;

        let sample_count = if use_scatter { num_samples.max(1) } else { 1 };

        let _ = app.emit(
            "progress",
            ProgressEvent::RunningMonteCarlo {
                current: 0,
                total: sample_count,
            },
        );

        // 各物理パラメータを独立にサンプリング
        let samples: Vec<MonteCarloSample> = {
            let mut rng = rand::thread_rng();
            (0..sample_count)
                .map(|_| MonteCarloSample {
                    ascent_rate_m_s: sample_parameter(
                        &mut rng,
                        ascent_rate,
                        ascent_rate_distribution.as_ref(),
                        f64::EPSILON,
                    ),
                    descent_rate_m_s: sample_parameter(
                        &mut rng,
                        descent_rate,
                        descent_rate_distribution.as_ref(),
                        f64::EPSILON,
                    ),
                    burst_altitude_m: sample_parameter(
                        &mut rng,
                        burst_altitude_mean,
                        burst_altitude_distribution.as_ref(),
                        0.0,
                    ),
                })
                .collect()
        };

        // Rayon で並列シミュレーション
        let completed = Arc::new(AtomicUsize::new(0));
        let completed_inner = completed.clone();
        let app_inner = app.clone();

        let (points, trajectories): (Vec<MonteCarloPoint>, Vec<MonteCarloTrajectory>) = samples
            .par_iter()
            .map(|sample| {
                let deviation = if ascent_rate_std == 0.0
                    && descent_rate_std == 0.0
                    && burst_altitude_std > 0.0
                {
                    Some((sample.burst_altitude_m - burst_altitude_mean) / burst_altitude_std)
                } else {
                    None
                };

                let config = SimConfig {
                    launch_site,
                    ascent: AscentParams {
                        target_rate_m_s: sample.ascent_rate_m_s,
                        ..ascent
                    },
                    ground_descend_rate_m_s: sample.descent_rate_m_s,
                    burst_altitude_m: sample.burst_altitude_m,
                    dt: 5.0,
                };

                let simulator = Simulator::new(config, dataset.clone(), launch);
                let trajectory = simulator.run();
                let result = trajectory_to_result(
                    &trajectory,
                    launch_site,
                    "GFS",
                    &model_run_time_utc,
                );

                let mc_point = MonteCarloPoint {
                    terrain_fallback_used: result.terrain_fallback_used,
                    landing_lat: result.landing_lat,
                    landing_lon: result.landing_lon,
                    ascent_rate_m_s: sample.ascent_rate_m_s,
                    descent_rate_m_s: sample.descent_rate_m_s,
                    burst_altitude: sample.burst_altitude_m,
                    deviation_sigma: deviation,
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
        let mean_result = trajectory_to_result(
            &mean_trajectory,
            launch_site,
            "GFS",
            &model_run_time_utc,
        );

        let n = sample_count as f64;
        Ok(MonteCarloResult {
            model: "GFS".to_string(),
            model_run_time_utc,
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
    launch_time: String,
    ascent_rate: f64,
    ascent_rate_std: f64,
    gross_mass_kg: f64,
    balloon_class_g: u32,
    descent_rate: f64,
    descent_rate_std: f64,
    burst_altitude_mean: f64,
    burst_altitude_std: f64,
    num_members: u32,
    num_samples: u32,
) -> Result<MonteCarloResult, String> {
    let launch: DateTime<Utc> = launch_time.parse().map_err(|e| format!("Invalid launch_time: {}", e))?;
    let gefs_run = select_model_run_time(Utc::now(), launch);
    println!(
        "Using GEFS model run {} for launch {}",
        gefs_run.to_rfc3339(),
        launch.to_rfc3339()
    );
    let ascent = ascent_params(ascent_rate, gross_mass_kg, balloon_class_g)?;

    let launch_site = Geodetic {
        lat: launch_lat,
        lon: launch_lon,
        alt: launch_alt,
    };

    tokio::task::spawn_blocking(move || -> Result<MonteCarloResult, String> {
        let work_dir = Path::new(".").to_path_buf();
        let model_run_time_utc = gefs_run.to_rfc3339();

        let num = num_members.min(31);
        let mut members_to_run = Vec::new();
        members_to_run.push(GefsMember::Control);
        for i in 1..num {
            members_to_run.push(GefsMember::Perturbed(i as u8));
        }

        let ascent_rate_distribution =
            normal_distribution(ascent_rate, ascent_rate_std, "ascent rate")?;
        let descent_rate_distribution =
            normal_distribution(descent_rate, descent_rate_std, "descent rate")?;
        let burst_altitude_distribution =
            normal_distribution(burst_altitude_mean, burst_altitude_std, "burst altitude")?;
        let use_scatter =
            ascent_rate_std > 0.0 || descent_rate_std > 0.0 || burst_altitude_std > 0.0;
        let sample_count = if use_scatter { num_samples.max(1) } else { 1 };

        let member_count = members_to_run.len() as u32;
        let total_sims = member_count * sample_count;

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

            let samples: Vec<MonteCarloSample> = {
                let mut rng = rand::thread_rng();
                (0..sample_count)
                    .map(|_| MonteCarloSample {
                        ascent_rate_m_s: sample_parameter(
                            &mut rng,
                            ascent_rate,
                            ascent_rate_distribution.as_ref(),
                            f64::EPSILON,
                        ),
                        descent_rate_m_s: sample_parameter(
                            &mut rng,
                            descent_rate,
                            descent_rate_distribution.as_ref(),
                            f64::EPSILON,
                        ),
                        burst_altitude_m: sample_parameter(
                            &mut rng,
                            burst_altitude_mean,
                            burst_altitude_distribution.as_ref(),
                            0.0,
                        ),
                    })
                    .collect()
            };

            let completed_inner = completed.clone();
            let app_inner = app.clone();

            let (member_points, member_trajectories): (
                Vec<MonteCarloPoint>,
                Vec<MonteCarloTrajectory>,
            ) = samples
                .par_iter()
                .map(|sample| {
                    let config = SimConfig {
                        launch_site,
                        ascent: AscentParams {
                            target_rate_m_s: sample.ascent_rate_m_s,
                            ..ascent
                        },
                        ground_descend_rate_m_s: sample.descent_rate_m_s,
                        burst_altitude_m: sample.burst_altitude_m,
                        dt: 5.0,
                    };

                    let simulator = Simulator::new(config, dataset.clone(), launch);
                    let trajectory = simulator.run();
                    let result = trajectory_to_result(
                        &trajectory,
                        launch_site,
                        "GEFS",
                        &model_run_time_utc,
                    );

                    let mc_point = MonteCarloPoint {
                        terrain_fallback_used: result.terrain_fallback_used,
                        landing_lat: result.landing_lat,
                        landing_lon: result.landing_lon,
                        ascent_rate_m_s: sample.ascent_rate_m_s,
                        descent_rate_m_s: sample.descent_rate_m_s,
                        burst_altitude: sample.burst_altitude_m,
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
                    let mean_result = trajectory_to_result(
                        &mean_sim.run(),
                        launch_site,
                        "GEFS",
                        &model_run_time_utc,
                    );
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
            model: "GEFS".to_string(),
            model_run_time_utc,
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
