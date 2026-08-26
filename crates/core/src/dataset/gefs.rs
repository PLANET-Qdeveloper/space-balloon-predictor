use chrono::{DateTime, Utc};

use super::gfs::GfsRegion;

/// GEFSアンサンブルメンバー識別子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GefsMember {
    /// 制御ラン (gec00)
    Control,
    /// 摂動メンバー (gep01..gep30)
    Perturbed(u8),
    /// アンサンブル平均 (geavg)
    Mean,
    /// アンサンブルスプレッド (gespr)
    Spread,
}

impl GefsMember {
    /// GRIBファイル名のプレフィックスを返す
    pub fn file_prefix(&self) -> String {
        match self {
            GefsMember::Control => "gec00".to_string(),
            GefsMember::Perturbed(n) => format!("gep{:02}", n),
            GefsMember::Mean => "geavg".to_string(),
            GefsMember::Spread => "gespr".to_string(),
        }
    }
}

impl std::fmt::Display for GefsMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GefsMember::Control => write!(f, "control (c00)"),
            GefsMember::Perturbed(n) => write!(f, "perturbed ({:02})", n),
            GefsMember::Mean => write!(f, "mean"),
            GefsMember::Spread => write!(f, "spread"),
        }
    }
}

/// GEFS解像度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GefsResolution {
    /// 0.5度プライマリ (pgrb2ap5) - 最長384時間
    Primary0p5,
    /// 0.25度セレクト (pgrb2sp25) - 最長240時間
    Select0p25,
}

impl GefsResolution {
    /// NOMADSフィルタースクリプト名
    pub fn filter_script(&self) -> &str {
        match self {
            GefsResolution::Primary0p5 => "filter_gefs_atmos_0p50a.pl",
            GefsResolution::Select0p25 => "filter_gefs_atmos_0p25s.pl",
        }
    }

    /// ディレクトリパス
    pub fn dir_path(&self) -> &str {
        match self {
            GefsResolution::Primary0p5 => "pgrb2ap5",
            GefsResolution::Select0p25 => "pgrb2sp25",
        }
    }

    /// グリッド解像度のファイル名サフィックス
    pub fn grid_suffix(&self) -> &str {
        match self {
            GefsResolution::Primary0p5 => "0p50",
            GefsResolution::Select0p25 => "0p25",
        }
    }

    /// ファイルタイプ (pgrb2a / pgrb2s)
    pub fn file_type(&self) -> &str {
        match self {
            GefsResolution::Primary0p5 => "pgrb2a",
            GefsResolution::Select0p25 => "pgrb2s",
        }
    }

    /// 予報時間の最大値
    pub fn max_forecast_hour(&self) -> u32 {
        match self {
            GefsResolution::Primary0p5 => 384,
            GefsResolution::Select0p25 => 240,
        }
    }
}

/// NOMADS filterスクリプトでsubregionにしたGEFSファイルのURLを構築する
pub fn gefs_filter_url(
    date_str: &str,
    cycle_str: &str,
    forecast_hour: u32,
    member: GefsMember,
    region: &GfsRegion,
    resolution: GefsResolution,
) -> String {
    let prefix = member.file_prefix();
    let fh = if forecast_hour == 0 {
        "anl".to_string()
    } else {
        format!("f{:03}", forecast_hour)
    };

    let file = format!(
        "{}.t{}z.{}.{}.{}",
        prefix,
        cycle_str,
        resolution.file_type(),
        resolution.grid_suffix(),
        fh
    );

    format!(
        "https://nomads.ncep.noaa.gov/cgi-bin/{}?\
         dir=%2Fgefs.{}%2F{}%2Fatmos%2F{}&\
         file={}&var_HGT=on&var_TMP=on&var_UGRD=on&var_VGRD=on&all_lev=on&\
         subregion=&toplat={}&leftlon={}&rightlon={}&bottomlat={}",
        resolution.filter_script(),
        date_str,
        cycle_str,
        resolution.dir_path(),
        file,
        region.top_lat,
        region.left_lon,
        region.right_lon,
        region.bottom_lat
    )
}

/// 予報時間一覧を生成する
/// GEFSは3時間刻み～192時間、6時間刻み～384時間
pub fn gefs_forecast_hours(max_fh: u32) -> Vec<u32> {
    let mut hours = Vec::new();
    let mut fh = 0u32;
    while fh <= max_fh {
        hours.push(fh);
        if fh < 192 {
            fh += 3;
        } else {
            fh += 6;
        }
    }
    hours
}

pub struct GefsForecast {
    /// ダウンロード先URL
    pub url: String,
    /// 予報の有効時刻
    pub time: DateTime<Utc>,
    /// 推奨ローカル保存パス
    pub local_path: String,
    /// 切り出し領域
    pub region: GfsRegion,
    /// アンサンブルメンバー
    pub member: GefsMember,
}

pub struct GefsForecastSet {
    /// 各メンバーの予報ファイル群
    pub forecasts: Vec<GefsForecast>,
    /// 最初の予報時刻からの発射オフセット (時間単位)
    pub launch_offset_hours: f64,
}

/// 発射時刻に基づき、必要なGEFS予報のURL一覧を解決する
pub fn resolve_gefs_forecasts(
    gefs_run_time: DateTime<Utc>,
    launch_time: DateTime<Utc>,
    lat: f64,
    lon: f64,
    members: &[GefsMember],
    resolution: GefsResolution,
) -> Result<GefsForecastSet, Box<dyn std::error::Error>> {
    let total_diff_seconds = launch_time
        .signed_duration_since(gefs_run_time)
        .num_seconds();
    if total_diff_seconds < 0 {
        return Err(
            "Error: Launch time cannot be before the GEFS model initialization run time.".into(),
        );
    }

    let diff_hours = total_diff_seconds as f64 / 3600.0;
    let forecast_hour_low = ((diff_hours / 3.0).floor() as u32) * 3;
    let launch_offset_hours = diff_hours - (forecast_hour_low as f64);

    let date_str = gefs_run_time.format("%Y%m%d").to_string();
    let cycle_str = gefs_run_time.format("%H").to_string();
    let region = GfsRegion::around(lat, lon, super::gfs::REGION_MARGIN_DEG);

    let mut forecasts = Vec::new();

    for &member in members {
        for &offset in &[0u32, 3, 6] {
            let fh = forecast_hour_low + offset;
            let time = gefs_run_time + chrono::Duration::hours(fh as i64);
            let url = gefs_filter_url(
                &date_str,
                &cycle_str,
                fh,
                member,
                &region,
                resolution,
            );
            let member_key = match member {
                GefsMember::Control => "c00".to_string(),
                GefsMember::Perturbed(n) => format!("p{:02}", n),
                GefsMember::Mean => "avg".to_string(),
                GefsMember::Spread => "spr".to_string(),
            };
            let local_path = format!(
                "./gefs_{}_{}_{}_f{:03}_{}.grib2",
                date_str,
                cycle_str,
                member_key,
                fh,
                region.cache_key()
            );
            forecasts.push(GefsForecast {
                url,
                time,
                local_path,
                region,
                member,
            });
        }
    }

    Ok(GefsForecastSet {
        forecasts,
        launch_offset_hours,
    })
}
