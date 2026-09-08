// 乾燥空気の比気体定数R_d
// 気体定数をR, 気体の分子量をMとおくと，R_d=R/Mと表される
const GAS_CONSTANT_DRY_AIR: f64 = 287.058;

use std::f64::consts::PI;

// 重力加速度 (m/s^2)
pub const GRAVITY: f64 = 9.80665;
// 標準状態における大気とヘリウムの密度差 (kg/m^3)。
// Excel シート「浮力(fix)」の採用値 1.115（一般値。今井論文値は 1.20249）。
pub const RHO_DIFF_STD: f64 = 1.115;
const PRESSURE_STD_PA: f64 = 101325.0;
const TEMP_STD_K: f64 = 273.15;
// 製作所の上昇速度係数を本式の k に変換する除数（Excel メモより 23.6）
const K_DIVISOR: f64 = 23.6;

// 赤道半径（地球長半径a）
pub const WGS84_A: f64 = 6_378_137.0;
// 第一離心率の2乗
pub const WGS84_E2: f64 = 0.00669437999014;

/// 風速ベクトル (東西成分u, 南北成分v)
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub struct WindVector {
    pub u: f64, // 東西風 (正の値が東向き、負の値が西向き)
    pub v: f64, // 南北風 (正の値が北向き、負の値が南向き)
}

/// 特定の気圧と気温から乾燥空気の密度を計算する
/// PM = dRTより, d = (PM)/(RT) = P/(R_d*T)
/// pressure_pa: 気圧 (Pa)
/// temperature_k: 気温 (K)
pub fn air_density(pressure_pa: f64, temperature_k: f64) -> f64 {
    if temperature_k <= 0.0 {
        return 0.0;
    }
    pressure_pa / (GAS_CONSTANT_DRY_AIR * temperature_k)
}

/// 空気密度の比率に基づいて、ある高度での下降終端速度を推定する
/// velocity_0: 地上での終端速度 (m/s)
/// density_0: 地上での空気密度 (kg/m^3)
/// density: 推定したい高度における空気密度 (kg/m^3)
pub fn terminal_velocity(velocity_0: f64, density_0: f64, density_z: f64) -> f64 {
    if density_z <= 0.0 {
        return velocity_0;
    }
    velocity_0 * (density_0 / density_z).sqrt()
}

/// 理想大気モデル（国際標準大気：ISA / 米国標準大気1976モデル）に基づいて
/// 任意の海抜高度(m)における標準的な気圧(Pa)と気温(K)を算出する
/// 気象データの範囲外に出たときにフォールバックとして利用する
pub fn standard_atmosphere_pt(altitude_m: f64) -> (f64, f64) {
    let h = altitude_m.max(0.0);

    if h <= 11000.0 {
        // 対流圏: 地上〜高度11km
        let temp = 288.15 - 0.0065 * h;
        let press = 101325.0 * (temp / 288.15).powf(5.25588);
        (press, temp)
    } else if h <= 20000.0 {
        // 成層圏下部: 高度11km〜20km
        let temp = 216.65;
        let press = 22632.1 * (-0.00015769 * (h - 11000.0)).exp();
        (press, temp)
    } else if h <= 32000.0 {
        // 成層圏中部: 高度20km〜32km
        let temp = 216.65 + 0.001 * (h - 20000.0);
        let press = 5474.89 * (216.65 / temp).powf(34.1631);
        (press, temp)
    } else {
        // 32km以上
        let temp = 228.65;
        let press = 868.0 * (-0.00014 * (h - 32000.0)).exp();
        (press, temp)
    }
}

/// 理想大気モデルに基づいて任意の海抜高度(m)における
/// 標準的な空気密度(kg/m^3)を算出する
pub fn standard_atmosphere_density(altitude_m: f64) -> f64 {
    let (pressure_pa, temperature_k) = standard_atmosphere_pt(altitude_m);
    air_density(pressure_pa, temperature_k)
}

pub fn ascent_coeff_k(balloon_class_g: u32) -> Option<f64> {
    let raw = match balloon_class_g {
        1000 => 132.0,
        1500 => 138.0,
        2000 => 148.0,
        3000 => 158.0,
        _ => return None,
    };
    Some(raw / K_DIVISOR)
}

pub fn forward_ascent_rate(coeff_k: f64, gross_mass_kg: f64, net_lift_kg: f64) -> f64 {
    if !(coeff_k > 0.0) || !(gross_mass_kg > 0.0) || !(net_lift_kg > 0.0) {
        return 0.0;
    }
    coeff_k * net_lift_kg.sqrt() / (gross_mass_kg + net_lift_kg).cbrt()
}

pub fn solve_net_lift(target_rate_m_s: f64, gross_mass_kg: f64, coeff_k: f64) -> Option<f64> {
    if !(target_rate_m_s > 0.0) || !(gross_mass_kg > 0.0) || !(coeff_k > 0.0) {
        return None;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    while forward_ascent_rate(coeff_k, gross_mass_kg, hi) < target_rate_m_s {
        hi *= 2.0;
        if !hi.is_finite() || hi > 1e9 {
            return None;
        }
    }
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if forward_ascent_rate(coeff_k, gross_mass_kg, mid) < target_rate_m_s {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let lift = 0.5 * (lo + hi);
    if lift.is_finite() && lift > 0.0 {
        Some(lift)
    } else {
        None
    }
}

pub fn sphere_cross_section(volume_m3: f64) -> f64 {
    if !(volume_m3 > 0.0) {
        return 0.0;
    }
    let radius = (3.0 * volume_m3 / (4.0 * PI)).cbrt();
    PI * radius * radius
}

#[derive(Debug, Clone, Copy)]
pub struct AscentCalibration {
    pub target_rate_m_s: f64,
    pub net_lift_kg: f64,
    pub total_mass_kg: f64,
    pub volume0_m3: f64,
    pub area0_m2: f64,
    pub cd_area_m2: f64,
    pub pressure0_pa: f64,
    pub temp0_k: f64,
    pub density0: f64,
    pub burst_volume_m3: Option<f64>,
}

pub fn calibrate_ascent(
    coeff_k: f64,
    gross_mass_kg: f64,
    target_rate_m_s: f64,
    launch_alt_m: f64,
    burst_volume_m3: Option<f64>,
) -> Option<AscentCalibration> {
    let lift = solve_net_lift(target_rate_m_s, gross_mass_kg, coeff_k)?;
    let (p0, t0) = standard_atmosphere_pt(launch_alt_m);
    let rho0 = air_density(p0, t0);
    if !(rho0 > 0.0) {
        return None;
    }
    // 充填高度における大気-He 密度差（Excel C16 式の裏返し）
    let rho_diff0 = RHO_DIFF_STD * (p0 / PRESSURE_STD_PA) * (TEMP_STD_K / t0);
    if !(rho_diff0 > 0.0) || rho_diff0 >= rho0 {
        return None;
    }
    let volume0 = (gross_mass_kg + lift) / rho_diff0;
    if !(volume0 > 0.0) {
        return None;
    }
    let area0 = sphere_cross_section(volume0);
    if !(area0 > 0.0) {
        return None;
    }
    // ガス質量を込めた全質量。rho0*V0 - m_total = L となる
    let gas_mass = (rho0 - rho_diff0) * volume0;
    let total_mass = gross_mass_kg + gas_mass;
    // 地上の釣り合いから Cd*A0 を逆算（Cd 単独の入力は不要）
    let cd_area = 2.0 * lift * GRAVITY / (rho0 * target_rate_m_s * target_rate_m_s);
    if !(cd_area > 0.0) || !cd_area.is_finite() {
        return None;
    }
    Some(AscentCalibration {
        target_rate_m_s,
        net_lift_kg: lift,
        total_mass_kg: total_mass,
        volume0_m3: volume0,
        area0_m2: area0,
        cd_area_m2: cd_area,
        pressure0_pa: p0,
        temp0_k: t0,
        density0: rho0,
        burst_volume_m3,
    })
}

pub fn ascent_velocity(cal: &AscentCalibration, pressure_pa: f64, temperature_k: f64) -> f64 {
    if !(pressure_pa > 0.0) || !(temperature_k > 0.0) {
        return cal.target_rate_m_s;
    }
    let rho = air_density(pressure_pa, temperature_k);
    if !(rho > 0.0) || !rho.is_finite() {
        return cal.target_rate_m_s;
    }
    let mut volume = cal.volume0_m3 * (cal.pressure0_pa / pressure_pa) * (temperature_k / cal.temp0_k);
    if !volume.is_finite() || volume <= 0.0 {
        return cal.target_rate_m_s;
    }
    if let Some(vb) = cal.burst_volume_m3 {
        volume = volume.min(vb);
    }
    let linear = (volume / cal.volume0_m3).cbrt();
    if !linear.is_finite() || linear <= 0.0 {
        return cal.target_rate_m_s;
    }
    let net_kg = rho * volume - cal.total_mass_kg;
    if !(net_kg > 0.0) {
        return 0.0;
    }
    let drag_area = cal.cd_area_m2 * linear * linear;
    if !(drag_area > 0.0) {
        return 0.0;
    }
    let v2 = 2.0 * net_kg * GRAVITY / (rho * drag_area);
    if !(v2 > 0.0) || !v2.is_finite() {
        return 0.0;
    }
    v2.sqrt()
}

pub fn volume_burst_reached(cal: &AscentCalibration, pressure_pa: f64, temperature_k: f64) -> bool {
    match cal.burst_volume_m3 {
        Some(vb) => {
            if !(pressure_pa > 0.0) || !(temperature_k > 0.0) {
                return false;
            }
            let volume =
                cal.volume0_m3 * (cal.pressure0_pa / pressure_pa) * (temperature_k / cal.temp0_k);
            volume.is_finite() && volume >= vb
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_air_density() {
        // 標準大気圧 101325 Pa, 気温 288.15 K の時の密度は約 1.225 kg/m^3
        // https://pigeon-poppo.com/standard-atmosphere/
        let rho = air_density(101325.0, 288.15);
        assert!((rho - 1.225).abs() < 0.01);
    }

    #[test]
    fn test_terminal_velocity() {
        let velocity_0 = 5.0; // 地上での終端速度 5 m/s
        let density_0 = 1.225; // 地上空気密度
        let density = 0.30625; // 密度の薄い高度（地上の4分の1）

        // 密度が4分の1になると、終端速度は2倍（10.0 m/s）になるはず
        let velocity = terminal_velocity(velocity_0, density_0, density);
        assert!((velocity - 10.0).abs() < 1e-5);
    }

    #[test]
    fn test_ascent_coeff_k_table() {
        // Excel「浮力(fix)」現行表: 製作所係数 / 23.6
        assert!((ascent_coeff_k(2000).unwrap() - 148.0 / 23.6).abs() < 1e-9);
        assert!((ascent_coeff_k(1000).unwrap() - 132.0 / 23.6).abs() < 1e-9);
        assert!((ascent_coeff_k(1500).unwrap() - 138.0 / 23.6).abs() < 1e-9);
        assert!((ascent_coeff_k(3000).unwrap() - 158.0 / 23.6).abs() < 1e-9);
        // 欠番
        assert!(ascent_coeff_k(1200).is_none());
        assert!(ascent_coeff_k(4200).is_none());
    }

    #[test]
    fn test_solve_net_lift_matches_excel() {
        // Excel デフォルト入力 (W=6kg, v=7m/s, k=148/23.6) → L=6.827409726kg
        let k = 148.0 / 23.6;
        let lift = solve_net_lift(7.0, 6.0, k).unwrap();
        assert!((lift - 6.827409726).abs() < 1e-4, "lift = {}", lift);
        // 順方向で v に戻る
        let v = forward_ascent_rate(k, 6.0, lift);
        assert!((v - 7.0).abs() < 1e-6, "v = {}", v);
    }

    #[test]
    fn test_ascent_velocity_increases_with_altitude() {
        let k = 148.0 / 23.6;
        let cal = calibrate_ascent(k, 6.0, 7.0, 10.0, None).unwrap();
        // 地上では目標速度に一致
        let v0 = ascent_velocity(&cal, cal.pressure0_pa, cal.temp0_k);
        assert!((v0 - 7.0).abs() < 1e-6, "v0 = {}", v0);
        // 30km 標準大気では約2倍（1/6乗則: 67.97^(1/6) ≒ 2.02）
        let (p30, t30) = standard_atmosphere_pt(30000.0);
        let v30 = ascent_velocity(&cal, p30, t30);
        assert!(v30 > v0, "v30 = {} should exceed v0 = {}", v30, v0);
        assert!((v30 - 14.1).abs() < 0.6, "v30 = {}", v30);
        // 落下の鏡写し (10倍) にはならない
        assert!(v30 < 7.0 * 3.0, "v30 = {}", v30);
    }

    #[test]
    fn test_calibrate_ascent_rejects_invalid() {
        assert!(solve_net_lift(0.0, 6.0, 6.27).is_none());
        assert!(solve_net_lift(7.0, 0.0, 6.27).is_none());
        assert!(solve_net_lift(7.0, 6.0, 0.0).is_none());
    }
}
