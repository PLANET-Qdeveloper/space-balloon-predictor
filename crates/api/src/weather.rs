use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Timelike, Utc};
use log::info;
use space_balloon_predictor_rs::dataset::{self, Dataset};
use space_balloon_predictor_rs::grib::{HeightUnit, PressureUnit};

/// GFS は 00/06/12/18 UTC 初期化。NOMADS 反映待ちを見込んで少し過去のサイクルを選ぶ。
const GFS_AVAILABILITY_LAG_HOURS: i64 = 5;

/// `launch_time` と現在時刻から、使えそうな最新の GFS 初期化時刻を決める。
pub fn resolve_gfs_run_time(launch_time: DateTime<Utc>) -> DateTime<Utc> {
    let available_before = Utc::now() - Duration::hours(GFS_AVAILABILITY_LAG_HOURS);
    let limit = available_before.min(launch_time);
    floor_to_gfs_cycle(limit)
}

fn floor_to_gfs_cycle(dt: DateTime<Utc>) -> DateTime<Utc> {
    let hour = dt.hour();
    let cycle_hour = (hour / 6) * 6;
    dt.date_naive()
        .and_hms_opt(cycle_hour, 0, 0)
        .expect("valid GFS cycle")
        .and_utc()
}

/// NOAA NOMADS から GFS GRIB を取得（キャッシュ済みなら再利用）し Dataset を返す。
pub fn fetch_gfs_dataset(
    gfs_run_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    lat: f64,
    lon: f64,
    cache_dir: &Path,
) -> Result<Dataset, Box<dyn std::error::Error + Send + Sync>> {
    fs::create_dir_all(cache_dir)?;

    let forecasts = dataset::resolve_gfs_forecasts(gfs_run_time, launch_time, lat, lon)
        .map_err(|err| err.to_string())?;
    let mut local_paths: Vec<String> = Vec::with_capacity(forecasts.forecasts.len());

    for forecast in &forecasts.forecasts {
        let cached = cache_path(cache_dir, &forecast.local_path);
        if cached.exists() {
            info!("Using cached GRIB: {}", cached.display());
        } else {
            info!("Downloading GRIB: {}", forecast.url);
            download_to_file(&forecast.url, &cached)?;
            info!("Saved: {}", cached.display());
        }
        local_paths.push(cached.to_string_lossy().into_owned());
    }

    // GFS GRIB2: 気圧は Pa。height_unit は GRIB1 専用なのでダミー値で可。
    Dataset::from_grib_files(
        &local_paths,
        launch_time,
        PressureUnit::Pascal,
        HeightUnit::DeciMeters,
    )
    .map_err(|err| err.to_string().into())
}

fn cache_path(cache_dir: &Path, recommended_local: &str) -> PathBuf {
    let name = Path::new(recommended_local)
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("forecast.grib2"));
    cache_dir.join(name)
}

fn download_to_file(url: &str, path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut response = reqwest::blocking::get(url)?;
    if !response.status().is_success() {
        return Err(format!(
            "Failed to download GRIB (HTTP {}). NOAA keeps roughly the last 10 days of GFS data.",
            response.status()
        )
        .into());
    }

    let tmp = path.with_extension("grib2.partial");
    {
        let mut file = File::create(&tmp)?;
        response.copy_to(&mut file)?;
        file.flush()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}
