use crate::dataset::Dataset;
use crate::grib::Atmosphere;
use crate::geo::coords::Geodetic;
use crate::geo::interpolation::lerp;
use crate::engine::physics::{WindVector, air_density, ascent_velocity, calibrate_ascent, standard_atmosphere_density, standard_atmosphere_pt, terminal_velocity, volume_burst_reached, AscentCalibration, WGS84_E2, WGS84_A};
use chrono::{DateTime, Utc};
use std::sync::Arc;

const SEA_LEVEL_DENSITY: f64 = 1.225;

/// 位置の変化率（度/秒、メートル/秒）
#[derive(Default, Debug, Clone, Copy)]
struct StateRate {
    dlat: f64,
    dlon: f64,
    dalt: f64,
}

impl std::ops::Add for StateRate {
    type Output = Self;
    fn add(self, other: Self) -> Self {
        Self {
            dlat: self.dlat + other.dlat,
            dlon: self.dlon + other.dlon,
            dalt: self.dalt + other.dalt,
        }
    }
}

impl std::ops::Mul<f64> for StateRate {
    type Output = Self;
    fn mul(self, scalar: f64) -> Self {
        Self {
            dlat: self.dlat * scalar,
            dlon: self.dlon * scalar,
            dalt: self.dalt * scalar,
        }
    }
}

/// バルーンの現在の状態
#[derive(Debug, Clone, Copy)]
pub struct BalloonState {
    pub lat: f64,
    pub lon: f64,
    pub alt: f64,
    pub time: f64,
    pub is_burst: bool,
}

impl Default for BalloonState {
    fn default() -> Self {
        Self {
            lat: 0.0,
            lon: 0.0,
            alt: 0.0,
            time: 0.0,
            is_burst: false,
        }
    }
}

/// シミュレーションの結果を保持する
#[derive(Debug, Clone)]
pub struct Trajectory {
    pub states: Vec<BalloonState>,
}

impl Trajectory {
    pub fn new(states: Vec<BalloonState>) -> Self {
        Self { states }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct AscentParams {
    /// 総重量 W [kg]（気球＋ペイロード等すべて）
    pub gross_mass_kg: f64,
    /// 地上での目標上昇速度 v0 [m/s]
    pub target_rate_m_s: f64,
    /// 上昇速度係数 k（気球種別から決定。`physics::ascent_coeff_k` 参照）
    pub coeff_k: f64,
    /// バースト体積 [m^3]。None の場合は高度のみでバースト判定
    pub burst_volume_m3: Option<f64>,
}

/// シミュレーションの設定項目
#[derive(Debug, Clone, Copy)]
pub struct SimConfig {
    pub launch_site: Geodetic,
    pub ascent: AscentParams,
    pub ground_descend_rate_m_s: f64,
    pub burst_altitude_m: f64,
    pub dt: f64,
}

/// N時刻のGRIBデータと座標変換を保持し、RK4で軌道を計算するシミュレータ
pub struct Simulator {
    config: SimConfig,
    /// ソート済みの(時刻, Atmosphere)ペア
    atmospheres: Arc<Vec<(DateTime<Utc>, Atmosphere)>>,
    /// 各時刻間の累積時間（時間単位）。最初の要素は0.0
    cumulative_hours: Vec<f64>,
    /// 発射時刻 - 最初の気象データ時刻（時間単位）
    launch_offset_hours: f64,
    /// 地上較正済みの上昇パラメータ。較正失敗時は None（目標速度で定速上昇）
    ascent_cal: Option<AscentCalibration>,
}

/// StateRate に基づいてバルーン状態を dt 分だけ進める
fn apply_rate(state: &BalloonState, rate: &StateRate, dt: f64) -> BalloonState {
    BalloonState {
        lat: state.lat + rate.dlat * dt,
        lon: state.lon + rate.dlon * dt,
        alt: state.alt + rate.dalt * dt,
        time: state.time + dt,
        is_burst: state.is_burst,
    }
}

impl Simulator {
    pub fn new(
        config: SimConfig,
        dataset: Dataset,
        launch_time: DateTime<Utc>,
    ) -> Self {
        let atmospheres = dataset.clone_inner();

        // 発射時刻 - データベース時刻 のオフセットを計算
        let launch_offset_hours =
            launch_time.signed_duration_since(dataset.base_time()).num_seconds() as f64 / 3600.0;

        // 累積時間を計算
        let mut cumulative_hours = Vec::with_capacity(atmospheres.len());
        cumulative_hours.push(0.0);
        for i in 1..atmospheres.len() {
            let diff = atmospheres[i]
                .0
                .signed_duration_since(atmospheres[i - 1].0)
                .num_seconds() as f64
                / 3600.0;
            let prev = *cumulative_hours.last().unwrap();
            cumulative_hours.push(prev + diff);
        }

        Self {
            config,
            atmospheres,
            cumulative_hours,
            launch_offset_hours,
            ascent_cal: calibrate_ascent(
                config.ascent.coeff_k,
                config.ascent.gross_mass_kg,
                config.ascent.target_rate_m_s,
                config.launch_site.alt,
                config.ascent.burst_volume_m3,
            ),
        }
    }

    /// 現在の状態から緯度・経度・高度の変化率 (deg/s, m/s) を計算する
    fn dynamics(&self, state: &BalloonState) -> StateRate {
        // 極付近で cos(緯度)→0 による発散を防ぐため計算用緯度をクランプする
        let lat = state.lat.clamp(-89.9999, 89.9999);
        let geo = Geodetic {
            lat,
            lon: state.lon,
            alt: state.alt,
        };

        let current_elapsed_hours = self.launch_offset_hours + (state.time / 3600.0);

        // 累積時間から該当区間を2分探索し、2つのAtmosphereで補間
        let n = self.atmospheres.len();
        let (prev_idx, next_idx, time_ratio) = if n == 0 {
            return StateRate::default();
        } else if n == 1 {
            (0, 0, 0.0)
        } else {
            let partition = self
                .cumulative_hours
                .partition_point(|&h| h <= current_elapsed_hours);
            if partition == 0 {
                (0, 1, 0.0)
            } else if partition >= n {
                (n - 2, n - 1, 1.0)
            } else {
                let t_prev = self.cumulative_hours[partition - 1];
                let t_next = self.cumulative_hours[partition];
                let ratio = if (t_next - t_prev).abs() < f64::EPSILON {
                    0.0
                } else {
                    ((current_elapsed_hours - t_prev) / (t_next - t_prev)).clamp(0.0, 1.0)
                };
                (partition - 1, partition, ratio)
            }
        };

        let env_prev = &self.atmospheres[prev_idx].1;
        let env_next = &self.atmospheres[next_idx].1;

        let key = match env_prev.compute_interpolation_key(geo.lat, geo.lon) {
            Some(k) => k,
            None => return StateRate::default(),
        };

        // それぞれの気圧面データを取得
        let alt_map_prev = env_prev.build_altitude_map_with_key(&key);
        let alt_map_next = env_next.build_altitude_map_with_key(&key);
        let prev_state = env_prev.get_atmospheric_state_with_map(&geo, &alt_map_prev, &key);
        let next_state = env_next.get_atmospheric_state_with_map(&geo, &alt_map_next, &key);

        // 水平風速の決定
        // 前後の時刻の気圧面データの有無に応じて、風速を補間する
        let wind = match (&prev_state, &next_state) {
            // 両方の時刻にデータがある場合は線形補間
            (Some(prev), Some(next)) => {
                let u = lerp(prev.wind.u, next.wind.u, time_ratio);
                let v = lerp(prev.wind.v, next.wind.v, time_ratio);
                WindVector { u, v }
            }
            (Some(prev), None) => prev.wind,
            (None, Some(next)) => next.wind,
            // データがない場合は無風を仮定
            _ => WindVector::default(),
        };
        // 欠損グリッド値のNaN/Infを無風として扱う
        let wind = if wind.u.is_finite() && wind.v.is_finite() {
            wind
        } else {
            WindVector::default()
        };

        let (pressure_pa, temperature_k) = match (&prev_state, &next_state) {
            (Some(prev), Some(next)) => (
                lerp(prev.pressure_pa, next.pressure_pa, time_ratio),
                lerp(prev.temperature_k, next.temperature_k, time_ratio),
            ),
            (Some(prev), None) => (prev.pressure_pa, prev.temperature_k),
            (None, Some(next)) => (next.pressure_pa, next.temperature_k),
            _ => standard_atmosphere_pt(state.alt),
        };
        let (pressure_pa, temperature_k) =
            if pressure_pa.is_finite() && pressure_pa > 0.0
                && temperature_k.is_finite() && temperature_k > 0.0
            {
                (pressure_pa, temperature_k)
            } else {
                standard_atmosphere_pt(state.alt)
            };

        // 垂直速度の決定
        let vertical_velocity = if !state.is_burst {
            match &self.ascent_cal {
                Some(cal) => ascent_velocity(cal, pressure_pa, temperature_k),
                None => self.config.ascent.target_rate_m_s,
            }
        } else {
            let density_at_altitude = air_density(pressure_pa, temperature_k);
            let density_at_altitude =
                if density_at_altitude.is_finite() && density_at_altitude > 0.0 {
                    density_at_altitude
                } else {
                    standard_atmosphere_density(state.alt)
                };

            let speed = terminal_velocity(
                self.config.ground_descend_rate_m_s,
                SEA_LEVEL_DENSITY,
                density_at_altitude,
            );
            -speed
        };

        let lat_rad = lat.to_radians();
        let sin_lat = lat_rad.sin();

        let w = (1.0 - WGS84_E2 * sin_lat * sin_lat).sqrt();
        let m = WGS84_A * (1.0 - WGS84_E2) / (w * w * w); // 子午線曲率半径 (南北方向)
        let n = WGS84_A / w;                             // 卯酉線曲率半径 (東西方向)

        let deg_per_rad = 180.0 / std::f64::consts::PI;

        let dlat_dt = wind.v / (m + state.alt) * deg_per_rad;
        let dlon_dt = wind.u / ((n + state.alt) * lat_rad.cos()) * deg_per_rad;

        StateRate {
            dlat: if dlat_dt.is_finite() { dlat_dt } else { 0.0 },
            dlon: if dlon_dt.is_finite() { dlon_dt } else { 0.0 },
            dalt: if vertical_velocity.is_finite() {
                vertical_velocity
            } else {
                0.0
            },
        }
    }

    /// 4次のルンゲクッタ法で1ステップ進める
    fn rk4_step(&self, state: &BalloonState, dt: f64) -> BalloonState {
        let k1 = self.dynamics(state);
        let k2 = self.dynamics(&apply_rate(state, &k1, dt * 0.5));
        let k3 = self.dynamics(&apply_rate(state, &k2, dt * 0.5));
        let k4 = self.dynamics(&apply_rate(state, &k3, dt));

        let delta = (k1 + k2 * 2.0 + k3 * 2.0 + k4) * (dt / 6.0);

        BalloonState {
            lat: (state.lat + delta.dlat).clamp(-90.0, 90.0),
            lon: (state.lon + delta.dlon).rem_euclid(360.0),
            alt: state.alt + delta.dalt,
            time: state.time + dt,
            is_burst: state.is_burst,
        }
    }

    pub fn run(&self) -> Trajectory {
        let mut trajectory = Vec::new();

        let mut state = BalloonState {
            lat: self.config.launch_site.lat,
            lon: self.config.launch_site.lon,
            alt: self.config.launch_site.alt,
            time: 0.0,
            is_burst: false,
        };

        trajectory.push(state);

        while !(state.is_burst && state.alt <= self.config.launch_site.alt) {
            let mut next_state = self.rk4_step(&state, self.config.dt);

            if !state.is_burst {
                let alt_burst = next_state.alt >= self.config.burst_altitude_m;
                let vol_burst = match &self.ascent_cal {
                    Some(cal) => {
                        let (p, t) = standard_atmosphere_pt(next_state.alt.max(0.0));
                        volume_burst_reached(cal, p, t)
                    }
                    None => false,
                };
                next_state.is_burst = alt_burst || vol_burst;
            } else {
                next_state.is_burst = true;
            }

            state = next_state;
            trajectory.push(state);
        }

        Trajectory::new(trajectory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dataset::Dataset;
    use crate::geo::grid::LatLonGrid;
    use crate::grib::types::AtmosphereLayer;
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    /// 指定値で埋めた 2x2 グリッドを構築する
    fn grid(fill: f32) -> LatLonGrid {
        LatLonGrid {
            values: vec![fill; 4],
            lon_coords: Arc::new(vec![0.0f32, 1.0]),
            lat_coords: Arc::new(vec![0.0f32, 1.0]),
            width: 2,
            height: 2,
        }
    }

    /// 全変数がNaNのAtmosphereを構築する（欠損データを再現）
    fn nan_atmosphere() -> crate::grib::Atmosphere {
        let layer = AtmosphereLayer {
            u_wind: grid(f32::NAN),
            v_wind: grid(f32::NAN),
            temp_k: grid(f32::NAN),
            height_gpm: grid(1000.0),
        };
        let mut levels = BTreeMap::new();
        levels.insert(100_000, layer);
        crate::grib::Atmosphere::for_test(levels)
    }

    fn test_ascent() -> AscentParams {
        AscentParams {
            gross_mass_kg: 6.0,
            target_rate_m_s: 5.0,
            coeff_k: 148.0 / 23.6,
            burst_volume_m3: None,
        }
    }

    #[test]
    fn dynamics_is_finite_with_nan_grid() {
        let dataset = Dataset::from_atmospheres(vec![(Utc::now(), nan_atmosphere())]).unwrap();
        let config = SimConfig {
            launch_site: Geodetic {
                lat: 35.0,
                lon: 139.0,
                alt: 10.0,
            },
            ascent: test_ascent(),
            ground_descend_rate_m_s: 5.0,
            burst_altitude_m: 10_000.0,
            dt: 5.0,
        };
        let simulator = Simulator::new(config, dataset, Utc::now());
        let state = BalloonState {
            lat: 35.0,
            lon: 139.0,
            alt: 10.0,
            time: 0.0,
            is_burst: false,
        };

        let rate = simulator.dynamics(&state);
        assert!(rate.dlat.is_finite(), "dlat must be finite");
        assert!(rate.dlon.is_finite(), "dlon must be finite");
        assert!(rate.dalt.is_finite(), "dalt must be finite");
    }

    #[test]
    fn run_produces_valid_coordinates_with_nan_grid() {
        let dataset = Dataset::from_atmospheres(vec![(Utc::now(), nan_atmosphere())]).unwrap();
        let config = SimConfig {
            launch_site: Geodetic {
                lat: 35.0,
                lon: 139.0,
                alt: 10.0,
            },
            ascent: test_ascent(),
            ground_descend_rate_m_s: 5.0,
            burst_altitude_m: 10_000.0,
            dt: 5.0,
        };
        let simulator = Simulator::new(config, dataset, Utc::now());
        let trajectory = simulator.run();

        assert!(!trajectory.states.is_empty());
        for s in &trajectory.states {
            assert!(s.lat.is_finite(), "lat must be finite");
            assert!(s.lon.is_finite(), "lon must be finite");
            assert!(s.alt.is_finite(), "alt must be finite");
            assert!((-90.0..=90.0).contains(&s.lat), "lat out of range: {}", s.lat);
            assert!((0.0..360.0).contains(&s.lon), "lon out of range: {}", s.lon);
        }
    }

    #[test]
    fn ascent_rate_grows_with_altitude_in_dynamics() {
        let dataset = Dataset::from_atmospheres(vec![(Utc::now(), nan_atmosphere())]).unwrap();
        let config = SimConfig {
            launch_site: Geodetic {
                lat: 35.0,
                lon: 139.0,
                alt: 10.0,
            },
            ascent: AscentParams {
                gross_mass_kg: 6.0,
                target_rate_m_s: 7.0,
                coeff_k: 148.0 / 23.6,
                burst_volume_m3: None,
            },
            ground_descend_rate_m_s: 5.0,
            burst_altitude_m: 35_000.0,
            dt: 5.0,
        };
        let simulator = Simulator::new(config, dataset, Utc::now());
        let low = BalloonState {
            lat: 35.0,
            lon: 139.0,
            alt: 10.0,
            time: 0.0,
            is_burst: false,
        };
        let high = BalloonState {
            alt: 20_000.0,
            ..low
        };
        let v_low = simulator.dynamics(&low).dalt;
        let v_high = simulator.dynamics(&high).dalt;
        assert!((v_low - 7.0).abs() < 0.05, "v_low = {}", v_low);
        assert!(v_high > v_low, "v_high = {} should exceed v_low = {}", v_high, v_low);
    }
}
