//! 国土地理院 DEM10B 標高タイル。欠測・通信障害時は海抜0 mの近似を使う。
//! 出典: https://maps.gsi.go.jp/development/ichiran.html
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

type TileCache = HashMap<(u32, u32), Option<Vec<f64>>>;
static CACHE: OnceLock<Mutex<TileCache>> = OnceLock::new();

pub(super) struct Terrain {
    client: Option<reqwest::blocking::Client>,
    offline: bool,
    warned: bool,
}

fn pixel(lat: f64, lon: f64) -> Option<(u32, u32)> {
    if !lat.is_finite() || !lon.is_finite() || lat.abs() > 85.05112878 {
        return None;
    }
    let size = (256 * (1 << 14)) as f64;
    let x = (lon + 180.0).rem_euclid(360.0) / 360.0 * size;
    let y = (1.0 - lat.to_radians().tan().asinh() / std::f64::consts::PI) * 0.5 * size;
    Some((x.floor() as u32, y.clamp(0.0, size - 1.0).floor() as u32))
}

fn parse_tile(text: &str) -> Option<Vec<f64>> {
    let values: Vec<_> = text.trim().split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<f64>().unwrap_or(f64::NAN)).collect();
    (values.len() == 256 * 256).then_some(values)
}

impl Terrain {
    pub(super) fn used_fallback(&self) -> bool { self.warned }

    pub(super) fn new() -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5)).build().ok(),
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
        let (x, y) = pixel(lat, lon)?;
        let key = (x / 256, y / 256);
        let mut cache = CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock().ok()?;
        if !cache.contains_key(&key) {
            if self.offline { return None; }
            let url = format!("https://cyberjapandata.gsi.go.jp/xyz/dem/14/{}/{}.txt", key.0, key.1);
            let result = self.client.as_ref()?.get(url).send();
            let tile = match result {
                Ok(response) if response.status() == reqwest::StatusCode::NOT_FOUND => None,
                Ok(response) => match response.error_for_status().and_then(|r| r.text()) {
                    Ok(text) => parse_tile(&text),
                    Err(_) => { self.offline = true; return None; }
                },
                Err(_) => { self.offline = true; return None; }
            };
            // アンサンブル間で共有し、メモリ使用量を制限する。
            if cache.len() >= 256 { cache.clear(); }
            cache.insert(key, tile);
        }
        cache.get(&key)?.as_ref()?.get(((y % 256) * 256 + x % 256) as usize)
            .copied().filter(|v| v.is_finite())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coordinates_wrap_and_reject_poles() {
        assert_eq!(pixel(35.0, 139.0), pixel(35.0, 499.0));
        assert!(pixel(90.0, 0.0).is_none());
        assert!(pixel(f64::NAN, 0.0).is_none());
    }
    #[test]
    fn missing_pixels_and_invalid_tiles() {
        let mut entries = vec!["12.5"; 65536];
        entries[42] = "e";
        let values = parse_tile(&entries.join(",")).unwrap();
        assert_eq!(values[0], 12.5);
        assert!(values[42].is_nan());
        assert!(parse_tile("12,13").is_none());
    }
}
