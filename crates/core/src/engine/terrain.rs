//! 着地判定用の標高データ。
//! - 国土地理院 DEM10B（日本）: https://maps.gsi.go.jp/development/ichiran.html
//! - OpenTopoData SRTM30m（全球 / セルフホスト可）: https://www.opentopodata.org/
//! 欠測・通信障害時は海抜 0 m の近似を使う。
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;

/// 公開 OpenTopoData API のデフォルトベース URL（末尾スラッシュなし）。
pub const DEFAULT_OPENTOPO_BASE_URL: &str = "https://api.opentopodata.org";

/// 着地判定に使う標高データソース。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DemSource {
    /// 国土地理院 DEM10B（日本向け・高解像度）
    #[default]
    GsiDem10b,
    /// OpenTopoData（SRTM 30m・全球）。セルフホストも可。
    OpenTopoData,
}

impl DemSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GsiDem10b => "gsi",
            Self::OpenTopoData => "opentopodata",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "gsi" => Some(Self::GsiDem10b),
            "opentopodata" => Some(Self::OpenTopoData),
            _ => None,
        }
    }
}

/// 空文字や末尾スラッシュを正規化し、空なら公開 API を使う。
pub fn normalize_opentopo_base_url(raw: &str) -> String {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        DEFAULT_OPENTOPO_BASE_URL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn should_throttle_opentopo(base_url: &str) -> bool {
    base_url.contains("api.opentopodata.org")
}

const GSI_ZOOM: u32 = 14;
/// 公開 API はおおよそ 1 リクエスト/秒。
const OPENTOPO_MIN_INTERVAL: Duration = Duration::from_millis(1100);
const OPENTOPO_DATASET: &str = "srtm30m";

type GsiCache = HashMap<(u32, u32), Option<Vec<f64>>>;
type OpenTopoCache = HashMap<(i32, i32), Option<f64>>;

static GSI_CACHE: OnceLock<Mutex<GsiCache>> = OnceLock::new();
static OPENTOPO_CACHE: OnceLock<Mutex<OpenTopoCache>> = OnceLock::new();
static OPENTOPO_LAST_REQUEST: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

pub(super) struct Terrain {
    source: DemSource,
    opentopo_base_url: String,
    throttle_opentopo: bool,
    client: Option<reqwest::blocking::Client>,
    offline: bool,
    warned: bool,
}

#[derive(Debug, Deserialize)]
struct OpenTopoResponse {
    results: Vec<OpenTopoResult>,
    status: String,
}

#[derive(Debug, Deserialize)]
struct OpenTopoResult {
    elevation: Option<f64>,
}

fn pixel(lat: f64, lon: f64, zoom: u32) -> Option<(u32, u32)> {
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 85.05112878 {
        return None;
    }
    let size = (256u64 * (1u64 << zoom)) as f64;
    let x = (lon + 180.0).rem_euclid(360.0) / 360.0 * size;
    let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * size;
    Some((x.floor() as u32, y.clamp(0.0, size - 1.0).floor() as u32))
}

fn parse_gsi_tile(text: &str) -> Option<Vec<f64>> {
    let values: Vec<_> = text
        .trim()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<f64>().unwrap_or(f64::NAN))
        .collect();
    (values.len() == 256 * 256).then_some(values)
}

/// SRTM 1″ 格子に丸める。
fn open_topo_key(lat: f64, lon: f64) -> Option<(i32, i32)> {
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) {
        return None;
    }
    let lon = ((lon + 180.0).rem_euclid(360.0)) - 180.0;
    Some(((lat * 3600.0).round() as i32, (lon * 3600.0).round() as i32))
}

fn lon_for_opentopo(lon: f64) -> f64 {
    ((lon + 180.0).rem_euclid(360.0)) - 180.0
}

fn throttle_opentopo() {
    let Ok(mut last) = OPENTOPO_LAST_REQUEST
        .get_or_init(|| Mutex::new(None))
        .lock()
    else {
        return;
    };
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < OPENTOPO_MIN_INTERVAL {
            thread::sleep(OPENTOPO_MIN_INTERVAL - elapsed);
        }
    }
    *last = Some(Instant::now());
}

impl Terrain {
    pub(super) fn used_fallback(&self) -> bool {
        self.warned
    }

    pub(super) fn new(source: DemSource, opentopo_base_url: impl Into<String>) -> Self {
        let opentopo_base_url = normalize_opentopo_base_url(&opentopo_base_url.into());
        let throttle_opentopo = should_throttle_opentopo(&opentopo_base_url);
        Self {
            source,
            opentopo_base_url,
            throttle_opentopo,
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .ok(),
            offline: false,
            warned: false,
        }
    }

    pub(super) fn elevation(&mut self, lat: f64, lon: f64) -> f64 {
        let value = self.lookup(lat, lon);
        if value.is_none() && !self.warned {
            log::warn!("標高データなし: 該当地点では海抜0 mを着地面として近似します");
            self.warned = true;
        }
        value.unwrap_or(0.0)
    }

    fn lookup(&mut self, lat: f64, lon: f64) -> Option<f64> {
        match self.source {
            DemSource::GsiDem10b => self.lookup_gsi(lat, lon),
            DemSource::OpenTopoData => self.lookup_opentopo(lat, lon),
        }
    }

    fn lookup_gsi(&mut self, lat: f64, lon: f64) -> Option<f64> {
        let (x, y) = pixel(lat, lon, GSI_ZOOM)?;
        let key = (x / 256, y / 256);
        let mut cache = GSI_CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
        if !cache.contains_key(&key) {
            if self.offline {
                return None;
            }
            let tile = self.fetch_gsi_tile(key.0, key.1);
            if tile.is_none() && self.offline {
                return None;
            }
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(key, tile);
        }
        cache
            .get(&key)?
            .as_ref()?
            .get(((y % 256) * 256 + x % 256) as usize)
            .copied()
            .filter(|v| v.is_finite())
    }

    fn lookup_opentopo(&mut self, lat: f64, lon: f64) -> Option<f64> {
        let key = open_topo_key(lat, lon)?;
        {
            let cache = OPENTOPO_CACHE
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .ok()?;
            if let Some(cached) = cache.get(&key) {
                return *cached;
            }
            if self.offline {
                return None;
            }
        }

        let elevation = self.fetch_opentopo(lat, lon);
        let mut cache = OPENTOPO_CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .ok()?;
        if cache.len() >= 4096 {
            cache.clear();
        }
        cache.insert(key, elevation);
        elevation
    }

    fn fetch_gsi_tile(&mut self, tx: u32, ty: u32) -> Option<Vec<f64>> {
        let client = self.client.as_ref()?;
        let url = format!("https://cyberjapandata.gsi.go.jp/xyz/dem/{GSI_ZOOM}/{tx}/{ty}.txt");
        match client.get(&url).send() {
            Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => None,
            Ok(response) => match response.error_for_status().and_then(|r| r.text()) {
                Ok(text) => parse_gsi_tile(&text),
                Err(_) => {
                    self.offline = true;
                    None
                }
            },
            Err(_) => {
                self.offline = true;
                None
            }
        }
    }

    fn fetch_opentopo(&mut self, lat: f64, lon: f64) -> Option<f64> {
        let client = self.client.as_ref()?;
        let lon = lon_for_opentopo(lon);
        let url = format!(
            "{}/v1/{OPENTOPO_DATASET}?locations={lat:.6},{lon:.6}",
            self.opentopo_base_url
        );
        if self.throttle_opentopo {
            throttle_opentopo();
        }
        let response = match client.get(&url).send() {
            Ok(response) if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
                thread::sleep(OPENTOPO_MIN_INTERVAL);
                if self.throttle_opentopo {
                    throttle_opentopo();
                }
                client.get(&url).send()
            }
            other => other,
        };
        match response {
            Ok(response) => match response.error_for_status().and_then(|r| r.text()) {
                Ok(text) => match serde_json::from_str::<OpenTopoResponse>(&text) {
                    Ok(body) if body.status.eq_ignore_ascii_case("OK") => {
                        body.results.first().and_then(|r| r.elevation)
                    }
                    _ => None,
                },
                Err(_) => {
                    self.offline = true;
                    None
                }
            },
            Err(_) => {
                self.offline = true;
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_wrap_and_reject_poles() {
        assert_eq!(pixel(35.0, 139.0, 14), pixel(35.0, 499.0, 14));
        assert!(pixel(90.0, 0.0, 14).is_none());
        assert!(pixel(f64::NAN, 0.0, 14).is_none());
    }

    #[test]
    fn missing_pixels_and_invalid_tiles() {
        let mut entries = vec!["12.5"; 65536];
        entries[42] = "e";
        let values = parse_gsi_tile(&entries.join(",")).unwrap();
        assert_eq!(values[0], 12.5);
        assert!(values[42].is_nan());
        assert!(parse_gsi_tile("12,13").is_none());
    }

    #[test]
    fn dem_source_roundtrip() {
        assert_eq!(DemSource::parse("gsi"), Some(DemSource::GsiDem10b));
        assert_eq!(DemSource::parse("opentopodata"), Some(DemSource::OpenTopoData));
        assert_eq!(DemSource::parse("terrarium"), None);
        assert_eq!(DemSource::parse("nope"), None);
    }

    #[test]
    fn normalize_opentopo_base_url_defaults_and_trims() {
        assert_eq!(
            normalize_opentopo_base_url(""),
            DEFAULT_OPENTOPO_BASE_URL
        );
        assert_eq!(
            normalize_opentopo_base_url("https://api.opentopodata.org/"),
            "https://api.opentopodata.org"
        );
        assert_eq!(
            normalize_opentopo_base_url(" http://127.0.0.1:5000/ "),
            "http://127.0.0.1:5000"
        );
        assert!(should_throttle_opentopo(DEFAULT_OPENTOPO_BASE_URL));
        assert!(!should_throttle_opentopo("http://127.0.0.1:5000"));
    }
}
