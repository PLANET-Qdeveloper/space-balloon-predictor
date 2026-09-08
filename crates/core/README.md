# space-balloon-predictor-rs

スペースバルーンの軌道予測ライブラリ

## 使い方

```rust
use space_balloon_predictor_rs::{Dataset, Simulator};
use space_balloon_predictor_rs::engine::simulation::SimConfig;
use space_balloon_predictor_rs::geo::coords::Geodetic;
use space_balloon_predictor_rs::grib::PressureUnit;
use space_balloon_predictor_rs::export::kml;

// 気象データを読み込む
let dataset = Dataset::from_grib_files(
    &["weather.grib2".to_string()],
    launch_time,
    PressureUnit::HectoPascal,
    HeightUnit::DeciMeters,
)?;

// シミュレーション設定
let launch_site = Geodetic { lat: 35.0, lon: 139.0, alt: 10.0 };
let config = SimConfig {
    launch_site,
    ascent: AscentParams {
        gross_mass_kg: 6.0,
        target_rate_m_s: 5.0,
        coeff_k: ascent_coeff_k(2000).unwrap(),
        burst_volume_m3: None,
    },
    ground_descend_rate_m_s: 5.0,
    burst_altitude_m: 30000.0,
    dt: 5.0,
};

// シミュレーションを実行
let trajectory = Simulator::new(config, dataset, launch_time).run();

// KML にエクスポート
let kml = kml::trajectory_to_kml(&trajectory, launch_time);
std::fs::write("output.kml", kml)?;
```

## CLI

```bash
cargo run --bin space-balloon-predictor
```

```bash
# GFS データをダウンロードして予測
space-balloon-predictor gfs 2026-06-13T12:00:00Z 2026-06-13T15:00:00Z output.kml 35.0 139.0

# ローカル GRIB ファイルから予測
space-balloon-predictor grib --grib1 msm.grib2 launch_time output.kml 35.0 139.0

# アンサンブルモード（MSM + GFS マージ）
space-balloon-predictor grib --grib1 msm.grib2 --ensemble gfs1.grib2 gfs2.grib2 launch_time output.kml 35.0 139.0
```

## モンテカルロシミュレーション

```rust
use rayon::prelude::*;
use rand::Rng;
use space_balloon_predictor_rs::{Dataset, Simulator};
use space_balloon_predictor_rs::engine::simulation::SimConfig;

let dataset = Dataset::from_grib_files(&paths, launch_time, PressureUnit::HectoPascal, HeightUnit::DeciMeters)?;
let mut rng = rand::thread_rng();

// 並列シミュレーション
let trajectories: Vec<_> = (0..1000)
    .into_par_iter()
    .map(|_| {
        let config = SimConfig {
            launch_site,
            ascent: AscentParams {
                gross_mass_kg: 6.0,
                target_rate_m_s: rng.gen_range(4.5..5.5),
                coeff_k: ascent_coeff_k(2000).unwrap(),
                burst_volume_m3: None,
            },
            ground_descend_rate_m_s: 5.0,
            burst_altitude_m: rng.gen_range(28000.0..32000.0),
            dt: 5.0,
        };
        // dataset.clone() は Arc のクローンでコスト很小
        Simulator::new(config, dataset.clone(), launch_time).run()
    })
    .collect();
```