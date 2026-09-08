use std::path::Path;
use std::sync::Arc;

use crate::geo::grid::LatLonGrid;
use crate::engine::physics::WindVector;

/// GRIBファイル内の気圧値の単位
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureUnit {
    /// NOAA GFS, JMA MSM: hPa
    HectoPascal,
    /// ECMWF: Pa
    Pascal,
}

impl PressureUnit {
    /// raw_value を Pa に変換する
    pub(crate) fn to_pa(self, raw: f64) -> f64 {
        match self {
            PressureUnit::HectoPascal => raw * 100.0,
            PressureUnit::Pascal => raw,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeightUnit {
    DeciMeters,
    Geopotential,
}

impl HeightUnit {
    /// デコード済み生値を高度 [m] に変換する係数
    pub(crate) fn to_meters_scale(self) -> f64 {
        match self {
            HeightUnit::DeciMeters => 0.1,
            HeightUnit::Geopotential => 1.0 / 9.80665,
        }
    }
}

#[derive(Clone)]
pub(crate) struct GridMetadata {
    pub(crate) lon_coords: Arc<Vec<f32>>,
    pub(crate) lat_coords: Arc<Vec<f32>>,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

#[derive(Default)]
pub(crate) struct PressureLevelBuilder {
    pub(crate) u_wind: Option<LatLonGrid>,
    pub(crate) v_wind: Option<LatLonGrid>,
    pub(crate) temp_k: Option<LatLonGrid>,
    pub(crate) height_gpm: Option<LatLonGrid>,
}

/// 1等圧面の気圧値 (Pa) と、補間された位勢高度 (m) のペア
pub struct PressureHeightPair {
    pub pressure_pa: i32,
    pub altitude_m: f64,
}

#[derive(Clone)]
pub(crate) struct AtmosphereLayer {
    pub(crate) u_wind: LatLonGrid,
    pub(crate) v_wind: LatLonGrid,
    pub(crate) temp_k: LatLonGrid,
    pub(crate) height_gpm: LatLonGrid,
}

/// 1地点の大気状態 (風速・気温・気圧)
pub struct AtmospherePoint {
    pub wind: WindVector,
    pub temperature_k: f64,
    pub pressure_pa: f64,
}

/// 2つのLatLonGrid間で線形補間を行い、新しいLatLonGridを返す
pub(crate) fn interpolate_grids(a: &LatLonGrid, b: &LatLonGrid, ratio: f64) -> LatLonGrid {
    use crate::geo::interpolation::lerp;
    use std::sync::Arc;

    let values: Vec<f32> = a
        .values
        .iter()
        .zip(b.values.iter())
        .map(|(va, vb)| lerp(*va as f64, *vb as f64, ratio) as f32)
        .collect();
    LatLonGrid {
        values,
        lon_coords: Arc::clone(&a.lon_coords),
        lat_coords: Arc::clone(&a.lat_coords),
        width: a.width,
        height: a.height,
    }
}

use super::grib1::Grib1File;
use super::grib2::Grib2File;

pub enum AnyGribFile {
    V1(Grib1File),
    V2(Grib2File),
}

pub fn open_grib(path: impl AsRef<Path>) -> Result<AnyGribFile, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut f = std::fs::File::open(path.as_ref())?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic)?;
    if &magic[0..4] != b"GRIB" {
        return Err(format!("Not a GRIB file: {}", path.as_ref().display()).into());
    }
    match magic[7] {
        1 => Ok(AnyGribFile::V1(Grib1File::open(path)?)),
        2 => Ok(AnyGribFile::V2(Grib2File::open(path)?)),
        e => Err(format!("Unknown GRIB edition byte: {}", e).into()),
    }
}
