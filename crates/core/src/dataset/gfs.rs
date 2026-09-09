use chrono::{DateTime, Utc};

/// GFSサブリージョンの切り出し範囲（NOMADS filterスクリプト用）
#[derive(Debug, Clone, Copy)]
pub struct GfsRegion {
    pub top_lat: f64,
    pub bottom_lat: f64,
    pub left_lon: f64,
    pub right_lon: f64,
}

impl GfsRegion {
    /// 中心点からマージン（度）だけ広げた領域を計算する
    pub fn around(lat: f64, lon: f64, margin_deg: f64) -> Self {
        let top_lat = (lat + margin_deg).min(90.0);
        let bottom_lat = (lat - margin_deg).max(-90.0);
        let mut left_lon = lon - margin_deg;
        let mut right_lon = lon + margin_deg;
        // 経度範囲が±180を超える場合は日付変更線越えとして負値表現に変換する
        // (right_lon < left_lon が日付変更線を跨ぐ領域の意味になる)
        if right_lon > 180.0 {
            right_lon -= 360.0;
        } else if left_lon < -180.0 {
            left_lon += 360.0;
        }
        Self {
            top_lat,
            bottom_lat,
            left_lon,
            right_lon,
        }
    }

    pub fn cache_key(&self) -> String {
        let r = |x: f64| (x / REGION_CACHE_GRID_DEG).round() * REGION_CACHE_GRID_DEG;
        format!(
            "t{}_b{}_l{}_r{}",
            r(self.top_lat),
            r(self.bottom_lat),
            r(self.left_lon),
            r(self.right_lon)
        )
    }
}

/// NOMADS filterスクリプトでsubregionに分割したGFS 0.25°ファイルのURLを構築する
pub fn gfs_filter_url(
    date_str: &str,
    cycle_str: &str,
    forecast_hour: u32,
    region: &GfsRegion,
) -> String {
    let file = if forecast_hour == 0 {
        format!("gfs.t{}z.pgrb2.0p25.anl", cycle_str)
    } else {
        format!("gfs.t{}z.pgrb2.0p25.f{:03}", cycle_str, forecast_hour)
    };
    format!(
        "https://nomads.ncep.noaa.gov/cgi-bin/filter_gfs_0p25.pl?dir=/gfs.{}/{}/atmos&file={}&var_HGT=on&var_TMP=on&var_UGRD=on&var_VGRD=on&all_lev=on&subregion=&toplat={}&leftlon={}&rightlon={}&bottomlat={}",
        date_str,
        cycle_str,
        file,
        region.top_lat,
        region.left_lon,
        region.right_lon,
        region.bottom_lat
    )
}

/// 放球点を中心に切り出す領域のマージン（度）
pub const REGION_MARGIN_DEG: f64 = 10.0;

pub const REGION_CACHE_GRID_DEG: f64 = 4.0;

pub struct GfsForecast {
    /// ダウンロード先URL
    pub url: String,
    /// 予報の有効時刻
    pub time: DateTime<Utc>,
    /// 推奨ローカル保存パス
    pub local_path: String,
    /// 切り出し領域
    pub region: GfsRegion,
}

pub struct GfsForecastSet {
    /// 3時間間隔で発射時刻を挟む予報 (通常3件)
    pub forecasts: Vec<GfsForecast>,
    /// 最初の予報時刻からの発射オフセット (時間単位)
    pub launch_offset_hours: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_reuses_small_position_shift() {
        let base = GfsRegion::around(35.0, 139.0, REGION_MARGIN_DEG);
        let nearby = GfsRegion::around(35.4, 139.4, REGION_MARGIN_DEG);

        assert_eq!(base.cache_key(), nearby.cache_key());
    }

    #[test]
    fn cache_key_changes_after_crossing_cache_grid_boundary() {
        let base = GfsRegion::around(35.0, 139.0, REGION_MARGIN_DEG);
        let shifted = GfsRegion::around(35.6, 139.0, REGION_MARGIN_DEG);

        assert_ne!(base.cache_key(), shifted.cache_key());
    }
}

/// 発射時刻に基づき、必要なGFS予報のURL一覧を解決する
/// GFSは3時間ごとの予報を提供するため、発射時刻を挟む3時間区間の
/// 前後2件 + 中間1件 = 3件の予報URLを返す。
/// lat/lonを中心に ±REGION_MARGIN_DEG のsubregionで切り出したファイルをダウンロードする。
pub fn resolve_gfs_forecasts(
    gfs_run_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    lat: f64,
    lon: f64,
) -> Result<GfsForecastSet, Box<dyn std::error::Error>> {
    let total_diff_seconds = launch_time
        .signed_duration_since(gfs_run_time)
        .num_seconds();
    if total_diff_seconds < 0 {
        return Err(
            "Error: Launch time cannot be before the GFS model initialization run time.".into(),
        );
    }

    let diff_hours = total_diff_seconds as f64 / 3600.0;
    let forecast_hour_low = ((diff_hours / 3.0).floor() as u32) * 3;
    let launch_offset_hours = diff_hours - (forecast_hour_low as f64);

    let date_str = gfs_run_time.format("%Y%m%d").to_string();
    let cycle_str = gfs_run_time.format("%H").to_string();
    let region = GfsRegion::around(lat, lon, REGION_MARGIN_DEG);

    let forecasts: Vec<GfsForecast> = [0u32, 3, 6]
        .iter()
        .map(|&offset| {
            let fh = forecast_hour_low + offset;
            let time = gfs_run_time + chrono::Duration::hours(fh as i64);
            GfsForecast {
                url: gfs_filter_url(&date_str, &cycle_str, fh, &region),
                time,
                local_path: format!(
                    "./gfs_{}_{}_f{:03}_{}.grib2",
                    date_str,
                    cycle_str,
                    fh,
                    region.cache_key()
                ),
                region,
            }
        })
        .collect();

    Ok(GfsForecastSet {
        forecasts,
        launch_offset_hours,
    })
}
