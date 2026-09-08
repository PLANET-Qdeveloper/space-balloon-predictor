use chrono::{DateTime, Utc};
use clap::{Parser, Subcommand};
use log::info;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use space_balloon_predictor_rs::geo::coords::{EARTH_RADIUS, Geodetic};
use space_balloon_predictor_rs::{dataset, dataset::Dataset};
use space_balloon_predictor_rs::grib::{HeightUnit, PressureUnit};
use space_balloon_predictor_rs::export::kml;
use space_balloon_predictor_rs::engine::simulation::{AscentParams, SimConfig, Simulator, Trajectory};
use space_balloon_predictor_rs::engine::physics::ascent_coeff_k;

fn build_ascent(
    ascent: f64,
    balloon_class: u32,
    gross_mass: f64,
    coeff_k: Option<f64>,
    burst_volume: Option<f64>,
) -> Result<AscentParams, String> {
    let coeff_k = match coeff_k {
        Some(k) if k > 0.0 => k,
        Some(_) => return Err("coeff-k must be positive".into()),
        None => ascent_coeff_k(balloon_class).ok_or_else(|| {
            format!(
                "Unknown balloon class: {}g. Choose from 1000, 1500, 2000, 3000 (or pass --coeff-k).",
                balloon_class
            )
        })?,
    };
    if !(gross_mass > 0.0) {
        return Err("gross-mass must be positive".into());
    }
    if !(ascent > 0.0) {
        return Err("ascent must be positive".into());
    }
    Ok(AscentParams {
        gross_mass_kg: gross_mass,
        target_rate_m_s: ascent,
        coeff_k,
        burst_volume_m3: burst_volume,
    })
}
fn parse_pressure_unit(s: &str) -> Result<PressureUnit, String> {
    match s.to_lowercase().as_str() {
        "hpa" | "hectopascal" => Ok(PressureUnit::HectoPascal),
        "pa" | "pascal" => Ok(PressureUnit::Pascal),
        _ => Err(format!("unknown pressure unit: {s}. Expected 'hpa' or 'pa'")),
    }
}

fn parse_height_unit(s: &str) -> Result<HeightUnit, String> {
    match s.to_lowercase().as_str() {
        "decimeters" | "msm" => Ok(HeightUnit::DeciMeters),
        "geopotential" | "era5" => Ok(HeightUnit::Geopotential),
        _ => Err(format!(
            "unknown height unit: {s}. Expected 'decimeters' (MSM) or 'geopotential' (ERA5)"
        )),
    }
}

/// フライト統計情報
struct FlightStats {
    max_altitude_m: f64,
    duration_secs: f64,
    landing_lat: f64,
    landing_lon: f64,
    drift_km: f64,
}

/// 軌道データからフライト統計を計算して返す
fn compute_flight_stats(trajectory: &Trajectory, launch_site: Geodetic) -> FlightStats {
    let max_alt = trajectory.states.iter().map(|s| s.alt).fold(0.0, f64::max);

    let last = trajectory.states.last().unwrap();
    let d_lat = (last.lat - launch_site.lat).to_radians();
    let d_lon = (last.lon - launch_site.lon).to_radians();
    let lat_avg = ((last.lat + launch_site.lat) / 2.0).to_radians();
    let drift = ((d_lat * EARTH_RADIUS).powi(2)
        + (d_lon * EARTH_RADIUS * lat_avg.cos()).powi(2))
    .sqrt()
        / 1000.0;

    FlightStats {
        max_altitude_m: max_alt,
        duration_secs: last.time,
        landing_lat: last.lat,
        landing_lon: last.lon,
        drift_km: drift,
    }
}

#[derive(Parser)]
#[command(name = "space-balloon-predictor")]
#[command(about = "Predicts high-altitude balloon trajectory using weather data")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download GFS data from NOAA NOMADS and predict trajectory
    Gfs {
        /// GFS model initialization run time (e.g. 2026-06-13T12:00:00Z)
        gfs_run_time: DateTime<Utc>,
        /// Launch time (e.g. 2026-06-13T15:00:00Z)
        launch_time: DateTime<Utc>,
        /// Output KML file path
        output: String,
        /// Launch site latitude (degrees)
        lat: f64,
        /// Launch site longitude (degrees)
        lon: f64,
        /// Launch site altitude in meters (default: 10.0)
        #[arg(default_value_t = 10.0)]
        alt: f64,
        /// Balloon ascent rate in m/s
        #[arg(long, default_value_t = 5.0)]
        ascent: f64,
        /// Balloon class weight in grams (selects ascent coefficient k: 1000, 1500, 2000, 3000)
        #[arg(long, default_value_t = 2000)]
        balloon_class: u32,
        /// Total suspended weight W in kg
        #[arg(long, default_value_t = 6.0)]
        gross_mass: f64,
        /// Custom ascent coefficient k (overrides balloon class lookup)
        #[arg(long)]
        coeff_k: Option<f64>,
        /// Burst volume in m^3 (optional; altitude trigger always applies)
        #[arg(long)]
        burst_volume: Option<f64>,
        /// Parachute descent rate at ground level in m/s
        #[arg(long, default_value_t = 5.0)]
        descent: f64,
        /// Burst altitude in meters
        #[arg(long, default_value_t = 30000.0)]
        burst: f64,
        /// Pressure unit in GRIB data
        #[arg(long, value_parser = parse_pressure_unit, default_value = "pa")]
        pressure_unit: PressureUnit,
    },
    /// Use local GRIB1/GRIB2 files to predict trajectory
    Grib {
        /// Primary GRIB file (e.g. MSM GRIB1)
        #[arg(long, required = true)]
        grib1: Vec<String>,
        /// Enable ensemble mode: merge primary with time-interpolated GFS files
        #[arg(long)]
        ensemble: Vec<String>,
        /// Launch time (e.g. 2026-06-13T15:00:00Z)
        launch_time: DateTime<Utc>,
        /// Output KML file path
        output: String,
        /// Launch site latitude (degrees)
        lat: f64,
        /// Launch site longitude (degrees)
        lon: f64,
        /// Launch site altitude in meters (default: 10.0)
        #[arg(default_value_t = 10.0)]
        alt: f64,
        /// Balloon ascent rate in m/s
        #[arg(long, default_value_t = 5.0)]
        ascent: f64,
        /// Balloon class weight in grams (selects ascent coefficient k: 1000, 1500, 2000, 3000)
        #[arg(long, default_value_t = 2000)]
        balloon_class: u32,
        /// Total suspended weight W in kg
        #[arg(long, default_value_t = 6.0)]
        gross_mass: f64,
        /// Custom ascent coefficient k (overrides balloon class lookup)
        #[arg(long)]
        coeff_k: Option<f64>,
        /// Burst volume in m^3 (optional; altitude trigger always applies)
        #[arg(long)]
        burst_volume: Option<f64>,
        /// Parachute descent rate at ground level in m/s
        #[arg(long, default_value_t = 5.0)]
        descent: f64,
        /// Burst altitude in meters
        #[arg(long, default_value_t = 30000.0)]
        burst: f64,
        /// Pressure unit in GRIB data
        #[arg(long, value_parser = parse_pressure_unit, default_value = "hpa")]
        pressure_unit: PressureUnit,
        /// Height unit for GRIB1 param 129 (decimeters=MSM GH, geopotential=ERA5 z)
        #[arg(long, value_parser = parse_height_unit, default_value = "decimeters")]
        height_unit: HeightUnit,
    },
}

fn download_to_file(url: &str, local_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    info!("Downloading '{}'...", url);
    let mut response = reqwest::blocking::get(url)?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download GRIB file. HTTP Status: {}.\n\
             (Note: NOAA only stores the last 10 days of forecast data. Older dates will result in errors.)",
            response.status()
        ).into());
    }
    let mut dest = File::create(local_path)?;
    std::io::copy(&mut response, &mut dest)?;
    info!("Saved to '{}'.", local_path);
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Command::Gfs {
            gfs_run_time,
            launch_time,
            output,
            lat,
            lon,
            alt,
            ascent,
            balloon_class,
            gross_mass,
            coeff_k,
            burst_volume,
            descent,
            burst,
            pressure_unit,
        } => {
            let launch_site = Geodetic { lat, lon, alt };
            let forecasts = dataset::resolve_gfs_forecasts(gfs_run_time, launch_time, lat, lon)?;

            let mut local_paths: Vec<String> = Vec::new();
            for f in &forecasts.forecasts {
                if Path::new(&f.local_path).exists() {
                    info!("File '{}' already exists locally. Skipping download.", f.local_path);
                } else {
                    download_to_file(&f.url, &f.local_path)?;
                }
                local_paths.push(f.local_path.clone());
            }

            let dataset = Dataset::from_grib_files(&local_paths, launch_time, pressure_unit, HeightUnit::DeciMeters)?;

            let config = SimConfig {
                launch_site,
                ascent: build_ascent(ascent, balloon_class, gross_mass, coeff_k, burst_volume)
                    .map_err(|e| format!("Invalid ascent parameters: {e}"))?,
                ground_descend_rate_m_s: descent,
                burst_altitude_m: burst,
                dt: 5.0,
            };
            let trajectory = Simulator::new(config, dataset, launch_time).run();

            let stats = compute_flight_stats(&trajectory, launch_site);
            println!("\nResults:");
            println!("  Launch:     {} UTC", launch_time);
            println!(
                "  Duration:   {:.1} min ({:.1} hr)",
                stats.duration_secs / 60.0,
                stats.duration_secs / 3600.0
            );
            println!(
                "  Max Alt:    {:.0} m ({:.1} km)",
                stats.max_altitude_m,
                stats.max_altitude_m / 1000.0
            );
            println!("  Landing:    {:.5}, {:.5}", stats.landing_lat, stats.landing_lon);
            println!("  Drift:      {:.1} km\n", stats.drift_km);

            info!("Writing output KML trajectory to '{}'...", output);
            let kml_content = kml::trajectory_to_kml(&trajectory, launch_time);
            let mut file = File::create(&output)?;
            file.write_all(kml_content.as_bytes())?;
            info!("Done!");
        }
        Command::Grib {
            grib1,
            ensemble,
            launch_time,
            output,
            lat,
            lon,
            alt,
            ascent,
            balloon_class,
            gross_mass,
            coeff_k,
            burst_volume,
            descent,
            burst,
            pressure_unit,
            height_unit,
        } => {
            let launch_site = Geodetic { lat, lon, alt };
            let dataset = if !ensemble.is_empty() {
                let primary = grib1
                    .first()
                    .ok_or("At least one primary file required for ensemble mode")?;
                Dataset::from_ensemble(
                    primary,
                    &ensemble,
                    launch_time,
                    pressure_unit,
                    height_unit,
                    lat,
                    lon,
                )?
            } else {
                Dataset::from_grib_files(&grib1, launch_time, pressure_unit, height_unit)?
            };

            let config = SimConfig {
                launch_site,
                ascent: build_ascent(ascent, balloon_class, gross_mass, coeff_k, burst_volume)
                    .map_err(|e| format!("Invalid ascent parameters: {e}"))?,
                ground_descend_rate_m_s: descent,
                burst_altitude_m: burst,
                dt: 5.0,
            };
            let trajectory = Simulator::new(config, dataset, launch_time).run();

            let stats = compute_flight_stats(&trajectory, launch_site);
            println!("\nResults:");
            println!("  Launch:     {} UTC", launch_time);
            println!(
                "  Duration:   {:.1} min ({:.1} hr)",
                stats.duration_secs / 60.0,
                stats.duration_secs / 3600.0
            );
            println!(
                "  Max Alt:    {:.0} m ({:.1} km)",
                stats.max_altitude_m,
                stats.max_altitude_m / 1000.0
            );
            println!("  Landing:    {:.5}, {:.5}", stats.landing_lat, stats.landing_lon);
            println!("  Drift:      {:.1} km\n", stats.drift_km);

            info!("Writing output KML trajectory to '{}'...", output);
            let kml_content = kml::trajectory_to_kml(&trajectory, launch_time);
            let mut file = File::create(&output)?;
            file.write_all(kml_content.as_bytes())?;
            info!("Done!");
        }
    }

    Ok(())
}
