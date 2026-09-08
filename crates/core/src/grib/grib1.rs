use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::geo::grid::LatLonGrid;

use super::decode::{build_levels, store_grib2_param};
use super::parameter::{GribParameter, GRIB1_ISOBARIC_SURFACE, PARAM_H};
use super::types::{GridMetadata, HeightUnit, PressureLevelBuilder, PressureUnit};
use super::Atmosphere;

pub struct Grib1File {
    pub(crate) path: PathBuf,
}

impl Grib1File {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref().to_path_buf();
        if !path.exists() {
            return Err(format!("File not found: {}", path.display()).into());
        }
        Ok(Self { path })
    }
}

struct Grib1Message<'a> {
    msg: &'a grib_reader::Message<'a>,
}

impl<'a> Grib1Message<'a> {
    fn new(msg: &'a grib_reader::Message<'a>) -> Option<Self> {
        let pd = msg.grib1_product_definition()?;
        if pd.level_type != GRIB1_ISOBARIC_SURFACE {
            return None;
        }
        Some(Self { msg })
    }

    // 予測が行われた時刻を取得
    fn reference_time(&self) -> Option<(u16, u8, u8, u8, u8, u8)> {
        let pd = self.msg.grib1_product_definition()?;
        let rt = pd.reference_time;
        Some((rt.year, rt.month, rt.day, rt.hour, rt.minute, rt.second))
    }

    fn param_and_scale(&self) -> Option<(GribParameter, f64)> {
        let pd = self.msg.grib1_product_definition()?;
        GribParameter::from_grib1_parameter_number(pd.parameter_number)
    }

    fn pressure_pa(&self, unit: PressureUnit) -> Option<i32> {
        let pd = self.msg.grib1_product_definition()?;
        // GRIB1 の等圧面レベル値は hPa（ERA5・MSM）。Pa 換算する
        Some(unit.to_pa(pd.level_value as f64) as i32)
    }

    fn grid_metadata(&self) -> Option<GridMetadata> {
        let (width, height) = self.msg.grid_shape();
        let (lats, lons) = match (self.msg.latitudes(), self.msg.longitudes()) {
            (Ok(Some(lats)), Ok(Some(lons))) => (lats, lons),
            _ => return None,
        };
        let lat_coords: Vec<f32> = lats.iter().map(|&v| v as f32).collect();
        let lon_coords: Vec<f32> = lons.iter().map(|v| (*v as f32).rem_euclid(360.0)).collect();
        Some(GridMetadata {
            lon_coords: Arc::new(lon_coords),
            lat_coords: Arc::new(lat_coords),
            width,
            height,
        })
    }

    fn decode_grid(&self, metadata: &GridMetadata, unit_scale: f64) -> Option<LatLonGrid> {
        let raw_values = self.msg.read_flat_data_as_f32().ok()?;
        let values: Vec<f32> = if (unit_scale - 1.0_f64).abs() > f64::EPSILON {
            let s = unit_scale as f32;
            raw_values.iter().map(|v| *v * s).collect()
        } else {
            raw_values
        };
        Some(LatLonGrid {
            values,
            lon_coords: Arc::clone(&metadata.lon_coords),
            lat_coords: Arc::clone(&metadata.lat_coords),
            width: metadata.width,
            height: metadata.height,
        })
    }
}

impl Grib1File {
    pub fn to_atmosphere(
        &self,
        unit: PressureUnit,
        height_unit: HeightUnit,
    ) -> Result<Vec<(chrono::DateTime<chrono::Utc>, Atmosphere)>, Box<dyn std::error::Error>> {
        let file = grib_reader::GribFile::open(&self.path)?;

        let mut time_groups: BTreeMap<(u16, u8, u8, u8, u8, u8), Vec<grib_reader::Message<'_>>> =
            BTreeMap::new();

        for msg in file.messages() {
            let grib = Grib1Message::new(&msg);
            let key = match grib.and_then(|g| g.reference_time()) {
                Some(k) => k,
                None => continue,
            };
            time_groups.entry(key).or_default().push(msg);
        }

        let mut result = Vec::new();

        for ((year, month, day, hour, minute, _), messages) in &time_groups {
            let dt = chrono::NaiveDate::from_ymd_opt(*year as i32, *month as u32, *day as u32)
                .and_then(|d| d.and_hms_opt(*hour as u32, *minute as u32, 0))
                .map(|dt| chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));

            let dt = match dt {
                Some(dt) => dt,
                None => continue,
            };

            let mut temp_storage: BTreeMap<i32, PressureLevelBuilder> = BTreeMap::new();
            let mut grid_metadata: Option<GridMetadata> = None;

            for msg in messages {
                let grib = match Grib1Message::new(msg) {
                    Some(g) => g,
                    None => continue,
                };
                let (param, unit_scale) = match grib.param_and_scale() {
                    Some(p) => p,
                    None => continue,
                };
                let unit_scale = if param == PARAM_H {
                    height_unit.to_meters_scale()
                } else {
                    unit_scale
                };
                let pressure_pa = match grib.pressure_pa(unit) {
                    Some(p) => p,
                    None => continue,
                };

                if grid_metadata.is_none() {
                    grid_metadata = grib.grid_metadata();
                }
                let metadata = match grid_metadata.as_ref() {
                    Some(m) => m,
                    None => continue,
                };
                let grid = match grib.decode_grid(metadata, unit_scale) {
                    Some(g) => g,
                    None => continue,
                };

                let entry = temp_storage.entry(pressure_pa).or_default();
                store_grib2_param(entry, param, grid);
            }

            let levels = build_levels(temp_storage);
            if !levels.is_empty() {
                result.push((
                    dt,
                    Atmosphere {
                        levels,
                        secondary_key: None,
                        secondary_levels: HashSet::new(),
                    },
                ));
            }
        }

        result.sort_by_key(|(dt, _)| *dt);
        Ok(result)
    }
}
