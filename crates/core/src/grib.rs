mod decode;
pub(crate) mod grib1;
pub(crate) mod grib2;
pub mod parameter;
pub mod types;

use log::debug;
use std::collections::{BTreeMap, HashSet};

use crate::geo::coords::Geodetic;
use crate::geo::grid::GridInterpolationKey;
use crate::geo::interpolation::lerp;

pub use grib1::Grib1File;
pub use grib2::Grib2File;
pub use types::{AnyGribFile, HeightUnit, PressureUnit, open_grib};

use types::{
    AtmosphereLayer, AtmospherePoint, PressureHeightPair, interpolate_grids,
};

/// GRIB ファイルから解析された全等圧面の気象データ
/// キーは気圧値 (Pa)、値は対応するAtmosphereLayer
#[derive(Clone)]
pub struct Atmosphere {
    levels: BTreeMap<i32, AtmosphereLayer>,
    /// アンサンブル時にセカンダリグリッドの等圧面に適用する補間キー
    secondary_key: Option<GridInterpolationKey>,
    /// セカンダリグリッドに属する等圧面の集合
    secondary_levels: HashSet<i32>,
}

impl Atmosphere {
    /// プライマリグリッドの等圧面を使って緯度・経度の補間キーを計算する
    /// アンサンブル時はセカンダリ(GFS)グリッドをスキップし、プライマリ(MSM)グリッドから計算する
    pub fn compute_interpolation_key(&self, lat: f64, lon: f64) -> Option<GridInterpolationKey> {
        self.levels
            .iter()
            .find(|(press, _)| !self.secondary_levels.contains(press))
            .map(|(_, level)| level.height_gpm.compute_interpolation_key(lat, lon))
    }

    /// 全等圧面の高度を2次元補間して高度順にソートしたマップを返す
    #[allow(dead_code)]
    pub fn build_altitude_map(&self, lat: f64, lon: f64) -> Vec<PressureHeightPair> {
        let mut map: Vec<PressureHeightPair> = self
            .levels
            .iter()
            .map(|(&press, level_data)| PressureHeightPair {
                pressure_pa: press,
                altitude_m: level_data.height_gpm.interpolate_at(lat, lon),
            })
            .collect();
        map.sort_by(|a, b| {
            a.altitude_m
                .partial_cmp(&b.altitude_m)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        map
    }

    /// 事前計算済みの補間キーを使って全等圧面の高度マップを構築する
    pub fn build_altitude_map_with_key(
        &self,
        key: &GridInterpolationKey,
    ) -> Vec<PressureHeightPair> {
        let mut map: Vec<PressureHeightPair> = self
            .levels
            .iter()
            .map(|(&press, level_data)| {
                let level_key = self.level_key(press, key);
                PressureHeightPair {
                    pressure_pa: press,
                    altitude_m: level_data.height_gpm.interpolate_with_key(level_key),
                }
            })
            .collect();
        map.sort_by(|a, b| {
            a.altitude_m
                .partial_cmp(&b.altitude_m)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        map
    }

    /// 事前構築済みの高度マップと補間キーを使って大気状態を取得する
    pub fn get_atmospheric_state_with_map(
        &self,
        geo: &Geodetic,
        alt_map: &[PressureHeightPair],
        key: &GridInterpolationKey,
    ) -> Option<AtmospherePoint> {
        if alt_map.is_empty() {
            return None;
        }

        let (p_lower, p_upper, vertical_ratio) = Self::find_vertical_bracket(alt_map, geo.alt);

        let data_low = self.levels.get(&p_lower)?;
        let data_high = self.levels.get(&p_upper)?;

        let key_low = self.level_key(p_lower, key);
        let key_high = self.level_key(p_upper, key);

        let u = lerp(
            data_low.u_wind.interpolate_with_key(key_low),
            data_high.u_wind.interpolate_with_key(key_high),
            vertical_ratio,
        );
        let v = lerp(
            data_low.v_wind.interpolate_with_key(key_low),
            data_high.v_wind.interpolate_with_key(key_high),
            vertical_ratio,
        );
        let temp = lerp(
            data_low.temp_k.interpolate_with_key(key_low),
            data_high.temp_k.interpolate_with_key(key_high),
            vertical_ratio,
        );
        let pressure_pa = lerp(p_lower as f64, p_upper as f64, vertical_ratio);

        Some(AtmospherePoint {
            wind: crate::engine::physics::WindVector { u, v },
            temperature_k: temp,
            pressure_pa,
        })
    }

    /// ソート済み高度マップに対して2分探索で等圧面ペア (lower, upper, 補間比率) を返す
    fn find_vertical_bracket(alt_map: &[PressureHeightPair], altitude: f64) -> (i32, i32, f64) {
        let partition = alt_map.partition_point(|a| a.altitude_m < altitude);
        match partition {
            0 => {
                let pressure = alt_map[0].pressure_pa;
                (pressure, pressure, 0.0)
            }
            i if i >= alt_map.len() => {
                let last = alt_map.len() - 1;
                let pressure = alt_map[last].pressure_pa;
                (pressure, pressure, 1.0)
            }
            i => {
                let alt_lower = alt_map[i - 1].altitude_m;
                let alt_upper = alt_map[i].altitude_m;
                let ratio = (altitude - alt_lower) / (alt_upper - alt_lower);
                (alt_map[i - 1].pressure_pa, alt_map[i].pressure_pa, ratio)
            }
        }
    }

    /// 任意の緯度・経度・高度における気象環境を取得（Trilinear補間）
    #[allow(dead_code)]
    pub fn get_atmospheric_state_at(&self, geo: &Geodetic) -> Option<AtmospherePoint> {
        if self.levels.is_empty() {
            return None;
        }

        let alt_map = self.build_altitude_map(geo.lat, geo.lon);
        let (p_lower, p_upper, vertical_ratio) = Self::find_vertical_bracket(&alt_map, geo.alt);

        let data_low = self.levels.get(&p_lower)?;
        let data_high = self.levels.get(&p_upper)?;

        let u = lerp(
            data_low.u_wind.interpolate_at(geo.lat, geo.lon),
            data_high.u_wind.interpolate_at(geo.lat, geo.lon),
            vertical_ratio,
        );
        let v = lerp(
            data_low.v_wind.interpolate_at(geo.lat, geo.lon),
            data_high.v_wind.interpolate_at(geo.lat, geo.lon),
            vertical_ratio,
        );
        let temp = lerp(
            data_low.temp_k.interpolate_at(geo.lat, geo.lon),
            data_high.temp_k.interpolate_at(geo.lat, geo.lon),
            vertical_ratio,
        );
        let pressure_pa = lerp(p_lower as f64, p_upper as f64, vertical_ratio);

        Some(AtmospherePoint {
            wind: crate::engine::physics::WindVector { u, v },
            temperature_k: temp,
            pressure_pa,
        })
    }

    /// 他のAtmosphereの等圧面を結合する。
    /// selfの最小圧力（最高高度）より低い圧力のレベルのみを追加する。
    /// selfの範囲内の等圧面は污染しない。
    /// lat/lonはセカンダリグリッドの補間キー計算に使用する。
    pub fn merge_with(&mut self, other: &Atmosphere, lat: f64, lon: f64) {
        // selfの最小圧力（=最高高度の等圧面）を取得
        let min_pressure_self = self.levels.keys().min().copied().unwrap_or(i32::MAX);

        let mut new_levels = Vec::new();
        for (&pressure, layer) in &other.levels {
            // selfに既にあるレベルはスキップ
            if self.levels.contains_key(&pressure) {
                continue;
            }
            // selfの最小圧力以上のレベル（=selfの高度範囲内のレベル）は追加しない
            if pressure >= min_pressure_self {
                continue;
            }
            self.secondary_levels.insert(pressure);
            new_levels.push((pressure, layer.clone()));
        }
        for (pressure, layer) in new_levels {
            self.levels.insert(pressure, layer);
        }
        // セカンダリグリッドのキーを計算（最初の追加レベルのグリッドから）
        if self.secondary_key.is_none() {
            if let Some(&pressure) = self.secondary_levels.iter().next() {
                if let Some(level) = self.levels.get(&pressure) {
                    self.secondary_key = Some(level.height_gpm.compute_interpolation_key(lat, lon));
                }
            }
        }
    }

    /// 等圧面の気圧値に対応する補間キーを返す
    fn level_key<'a>(
        &'a self,
        pressure_pa: i32,
        primary_key: &'a GridInterpolationKey,
    ) -> &'a GridInterpolationKey {
        if self.secondary_levels.contains(&pressure_pa) {
            self.secondary_key.as_ref().unwrap_or(primary_key)
        } else {
            primary_key
        }
    }

    /// デバッグ用: 指定位置の各等圧面における風速・風向・高度を出力する
    pub fn debug_print_winds_at(&self, lat: f64, lon: f64) {
        let alt_map = self.build_altitude_map(lat, lon);
        let key = match self.compute_interpolation_key(lat, lon) {
            Some(k) => k,
            None => return,
        };
        debug!("  Pressure(Pa) | Alt(m)   | U(m/s)  | V(m/s)  | source");
        debug!("  -------------|----------|---------|---------|--------");
        for pair in &alt_map {
            let level = match self.levels.get(&pair.pressure_pa) {
                Some(l) => l,
                None => continue,
            };
            let lk = self.level_key(pair.pressure_pa, &key);
            let u = level.u_wind.interpolate_with_key(lk);
            let v = level.v_wind.interpolate_with_key(lk);
            let src = if self.secondary_levels.contains(&pair.pressure_pa) {
                "GFS"
            } else {
                "MSM"
            };
            debug!(
                "  {:>12} | {:>7.0} | {:>7.2} | {:>7.2} | {}",
                pair.pressure_pa, pair.altitude_m, u, v, src
            );
        }
    }

    /// 2つのAtmosphere間で線形時間補間を行い、補間結果を返す。
    /// ratio=0.0 → self, ratio=1.0 → other
    /// 共通する圧力レベルのみを補間する。
    pub fn interpolate_between(&self, other: &Atmosphere, ratio: f64) -> Option<Atmosphere> {
        if ratio <= 0.0 {
            return Some(self.clone());
        }
        if ratio >= 1.0 {
            return Some(other.clone());
        }

        let mut levels = BTreeMap::new();
        for (&pressure, self_layer) in &self.levels {
            if let Some(other_layer) = other.levels.get(&pressure) {
                levels.insert(
                    pressure,
                    AtmosphereLayer {
                        u_wind: interpolate_grids(&self_layer.u_wind, &other_layer.u_wind, ratio),
                        v_wind: interpolate_grids(&self_layer.v_wind, &other_layer.v_wind, ratio),
                        temp_k: interpolate_grids(&self_layer.temp_k, &other_layer.temp_k, ratio),
                        height_gpm: interpolate_grids(
                            &self_layer.height_gpm,
                            &other_layer.height_gpm,
                            ratio,
                        ),
                    },
                );
            }
        }

        if levels.is_empty() {
            None
        } else {
            Some(Atmosphere {
                levels,
                secondary_key: None,
                secondary_levels: HashSet::new(),
            })
        }
    }

    /// 等圧面の気圧値リストを返す
    pub fn pressure_levels(&self) -> Vec<i32> {
        self.levels.keys().copied().collect()
    }

    /// セカンダリグリッドに属する等圧面の集合を返す
    pub fn secondary_level_set(&self) -> Vec<i32> {
        self.secondary_levels.iter().copied().collect()
    }
}

#[cfg(test)]
impl Atmosphere {
    /// テスト用: 任意の等圧面セットからAtmosphereを構築する
    pub(crate) fn for_test(levels: BTreeMap<i32, AtmosphereLayer>) -> Self {
        Self {
            levels,
            secondary_key: None,
            secondary_levels: HashSet::new(),
        }
    }
}
