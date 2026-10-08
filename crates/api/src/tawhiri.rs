use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use space_balloon_predictor_rs::engine::simulation::Trajectory;
use space_balloon_predictor_rs::geo::coords::Geodetic;

use crate::error::ApiError;
use crate::predict::{predict, PredictRequest, PredictResult};

pub const API_VERSION: u32 = 1;
pub const PROFILE_STANDARD: &str = "standard_profile";
pub const PROFILE_FLOAT: &str = "float_profile";
pub const PROFILE_REVERSE: &str = "reverse_profile";
const MIN_RATE_M_S: f64 = 0.2;
const DEFAULT_BALLOON_CLASS_G: u32 = 2000;
const DEFAULT_GROSS_MASS_KG: f64 = 6.0;
const DEFAULT_DT_S: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Csv,
    Kml,
}

impl OutputFormat {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "json" => Some(Self::Json),
            "csv" => Some(Self::Csv),
            "kml" => Some(Self::Kml),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Csv => "csv",
            Self::Kml => "kml",
        }
    }
}

#[derive(Debug, Clone)]
pub enum FlightProfile {
    Standard {
        ascent_rate: f64,
        burst_altitude: f64,
        descent_rate: f64,
    },
    Float {
        ascent_rate: f64,
        float_altitude: f64,
        stop_datetime: DateTime<Utc>,
    },
    Reverse {
        ascent_rate: f64,
    },
}

impl FlightProfile {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Standard { .. } => PROFILE_STANDARD,
            Self::Float { .. } => PROFILE_FLOAT,
            Self::Reverse { .. } => PROFILE_REVERSE,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParsedRequest {
    pub launch_latitude: f64,
    pub launch_longitude: f64,
    pub launch_datetime: DateTime<Utc>,
    pub launch_altitude: f64,
    pub profile: FlightProfile,
    pub dataset: Option<DateTime<Utc>>,
    pub format: OutputFormat,
    pub balloon_class_g: u32,
    pub gross_mass_kg: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Point {
    pub altitude: f64,
    pub datetime: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stage {
    pub stage: String,
    pub trajectory: Vec<Point>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Metadata {
    pub complete_datetime: String,
    pub start_datetime: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EchoedRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ascent_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub burst_altitude: Option<f64>,
    pub dataset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descent_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub float_altitude: Option<f64>,
    pub format: String,
    pub launch_altitude: f64,
    pub launch_datetime: String,
    pub launch_latitude: f64,
    pub launch_longitude: f64,
    pub profile: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_datetime: Option<String>,
    pub version: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuccessResponse {
    pub metadata: Metadata,
    pub prediction: Vec<Stage>,
    pub request: EchoedRequest,
    pub warnings: EmptyWarnings,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct EmptyWarnings {}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub description: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
    pub metadata: Metadata,
}

pub fn rfc3339(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

pub fn parse_balloon_class(raw: &str, start: DateTime<Utc>) -> Result<u32, ApiError> {
    let class = raw.parse::<u32>().map_err(|_| {
        ApiError::request(format!("Invalid balloon class '{raw}'."), start)
    })?;
    if !matches!(class, 1000 | 1500 | 2000 | 3000) {
        return Err(ApiError::request(
            format!("Unknown balloon class: {class}g (expected 1000, 1500, 2000, or 3000)"),
            start,
        ));
    }
    Ok(class)
}

pub fn parse_request(
    params: &HashMap<String, String>,
    start: DateTime<Utc>,
) -> Result<ParsedRequest, ApiError> {
    parse_request_with_balloon_class(params, start, None)
}

pub fn parse_request_with_balloon_class(
    params: &HashMap<String, String>,
    start: DateTime<Utc>,
    balloon_class_g: Option<u32>,
) -> Result<ParsedRequest, ApiError> {
    let launch_latitude = extract_f64(params, "launch_latitude", start, |x| (-90.0..=90.0).contains(&x))?;
    let launch_longitude = extract_f64(params, "launch_longitude", start, |x| (0.0..360.0).contains(&x))?;
    let launch_datetime = extract_datetime(params, "launch_datetime", start)?;
    let launch_altitude = match params.get("launch_altitude") {
        None => 0.0,
        Some(_) => extract_f64(params, "launch_altitude", start, |x| x.is_finite())?,
    };
    let format = match params.get("format") {
        None => OutputFormat::Json,
        Some(value) => OutputFormat::parse(value).ok_or_else(|| {
            ApiError::internal(format!("Format not supported: {value}"), start)
        })?,
    };

    let profile_name = params
        .get("profile")
        .map(String::as_str)
        .unwrap_or(PROFILE_STANDARD);

    let profile = match profile_name {
        PROFILE_STANDARD => {
            let ascent_rate = rate_clip(extract_f64(params, "ascent_rate", start, |x| x > 0.0)?);
            let burst_altitude =
                extract_f64(params, "burst_altitude", start, |x| x > launch_altitude)?;
            let descent_rate = rate_clip(extract_f64(params, "descent_rate", start, |x| x > 0.0)?);
            FlightProfile::Standard {
                ascent_rate,
                burst_altitude,
                descent_rate,
            }
        }
        PROFILE_FLOAT => {
            let ascent_rate = rate_clip(extract_f64(params, "ascent_rate", start, |x| x > 0.0)?);
            let float_altitude =
                extract_f64(params, "float_altitude", start, |x| x > launch_altitude)?;
            let stop_datetime = extract_datetime(params, "stop_datetime", start)?;
            if stop_datetime <= launch_datetime {
                return Err(ApiError::request(
                    format!(
                        "Invalid value for parameter 'stop_datetime': {}.",
                        params.get("stop_datetime").map(String::as_str).unwrap_or("")
                    ),
                    start,
                ));
            }
            FlightProfile::Float {
                ascent_rate,
                float_altitude,
                stop_datetime,
            }
        }
        PROFILE_REVERSE => FlightProfile::Reverse {
            ascent_rate: rate_clip(extract_f64(params, "ascent_rate", start, |x| x > 0.0)?),
        },
        other => {
            return Err(ApiError::request(
                format!("Unknown profile '{other}'."),
                start,
            ));
        }
    };

    let dataset = match params.get("dataset") {
        None => None,
        Some(value) if value == "latest" => None,
        Some(_) => Some(extract_datetime(params, "dataset", start)?),
    };

    let balloon_class_g = match balloon_class_g {
        Some(class) => parse_balloon_class(&class.to_string(), start)?,
        None => match params.get("balloon_class") {
            None => DEFAULT_BALLOON_CLASS_G,
            Some(raw) => parse_balloon_class(raw, start)?,
        },
    };

    let gross_mass_kg = match params.get("gross_mass") {
        None => DEFAULT_GROSS_MASS_KG,
        Some(_) => extract_f64(params, "gross_mass", start, |x| x > 0.0)?,
    };

    Ok(ParsedRequest {
        launch_latitude,
        launch_longitude,
        launch_datetime,
        launch_altitude,
        profile,
        dataset,
        format,
        balloon_class_g,
        gross_mass_kg,
    })
}

pub fn run_prediction(
    parsed: &ParsedRequest,
    cache_dir: &Path,
    start: DateTime<Utc>,
) -> Result<SuccessResponse, ApiError> {
    match &parsed.profile {
        FlightProfile::Standard { .. } => {}
        FlightProfile::Float { .. } | FlightProfile::Reverse { .. } => {
            return Err(ApiError::not_implemented(
                format!(
                    "Profile '{}' is not yet implemented.",
                    parsed.profile.name()
                ),
                start,
            ));
        }
    }

    let request = predict_request(parsed);
    let result = predict(&request, cache_dir)
        .map_err(|err| map_predict_error(&err.to_string(), start))?;
    Ok(success_from_result(parsed, &result, start, Utc::now()))
}

pub fn stages_from_trajectory(trajectory: &Trajectory, launch_time: DateTime<Utc>) -> Vec<Stage> {
    let mut ascent = Vec::new();
    let mut descent = Vec::new();

    for state in &trajectory.states {
        let point = Point {
            altitude: state.alt,
            datetime: rfc3339(datetime_at(launch_time, state.time)),
            latitude: state.lat,
            longitude: wrap_lon_0_360(state.lon),
        };
        if state.is_burst {
            descent.push(point);
        } else {
            ascent.push(point);
        }
    }

    if let Some(burst) = descent.first().cloned() {
        match ascent.last() {
            Some(last) if last.datetime == burst.datetime => {}
            _ => ascent.push(burst),
        }
    }

    vec![
        Stage {
            stage: "ascent".to_string(),
            trajectory: ascent,
        },
        Stage {
            stage: "descent".to_string(),
            trajectory: descent,
        },
    ]
}

pub fn format_csv(response: &SuccessResponse) -> (String, String) {
    let mut output = String::from("datetime,latitude,longitude,altitude\n");
    for stage in &response.prediction {
        for point in &stage.trajectory {
            let lon = wrap_lon_180(point.longitude);
            output.push_str(&format!(
                "{},{:.5},{:.5},{:.1}\n",
                point.datetime, point.latitude, lon, point.altitude
            ));
        }
    }
    (attachment_filename(&response.request, "csv"), output)
}

pub fn format_kml(response: &SuccessResponse) -> Result<(String, String), ApiError> {
    let start = Utc::now();
    let req = &response.request;
    let (linestr_description, flight_info) = match req.profile.as_str() {
        PROFILE_STANDARD => (
            format!(
                "Ascent rate: {:.1}, descent rate: {:.1}, with burst at {:.1}m.",
                req.ascent_rate.unwrap_or(0.0),
                req.descent_rate.unwrap_or(0.0),
                req.burst_altitude.unwrap_or(0.0)
            ),
            format!(
                "Flight Data for start time {}, at site: {:.4},{:.4}, standard flight profile.",
                req.launch_datetime, req.launch_latitude, req.launch_longitude
            ),
        ),
        other => {
            return Err(ApiError::internal(
                format!("Unknown Flight Profile for KML export: {other}."),
                start,
            ));
        }
    };

    let mut linestring_coords = String::new();
    for stage in &response.prediction {
        for point in &stage.trajectory {
            let lon = wrap_lon_180(point.longitude);
            linestring_coords.push_str(&format!(
                "{:.5},{:.5},{:.1}\n",
                lon, point.latitude, point.altitude
            ));
        }
    }

    let placemarks = standard_kml_placemarks(&response.prediction);
    let output = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<kml xmlns="http://www.opengis.net/kml/2.2">
<Document>
<name>Flight Path</name>
<description>{flight_info}</description>
<Style id="yellowPoly">
<LineStyle>
<color>7f00ffff</color>
<width>4</width>
</LineStyle>
<PolyStyle>
<color>7f00ff00</color>
</PolyStyle>
</Style>
<Placemark>
<name>Flight path</name>
<description>{linestr_description}</description>
<styleUrl>#yellowPoly</styleUrl>
<LineString>
<extrude>1</extrude>
<tesselate>1</tesselate>
<altitudeMode>absolute</altitudeMode>
<coordinates>
{linestring_coords}</coordinates>
</LineString></Placemark>
{placemarks}
</Document></kml>
"#
    );

    Ok((attachment_filename(req, "kml"), output))
}

fn success_from_result(
    parsed: &ParsedRequest,
    result: &PredictResult,
    start: DateTime<Utc>,
    complete: DateTime<Utc>,
) -> SuccessResponse {
    SuccessResponse {
        metadata: Metadata {
            complete_datetime: rfc3339(complete),
            start_datetime: rfc3339(start),
        },
        prediction: stages_from_trajectory(&result.trajectory, parsed.launch_datetime),
        request: echo_request(parsed, result.gfs_run_time),
        warnings: EmptyWarnings {},
    }
}

fn echo_request(parsed: &ParsedRequest, dataset: DateTime<Utc>) -> EchoedRequest {
    let mut echoed = EchoedRequest {
        ascent_rate: None,
        burst_altitude: None,
        dataset: rfc3339(dataset),
        descent_rate: None,
        float_altitude: None,
        format: parsed.format.as_str().to_string(),
        launch_altitude: parsed.launch_altitude,
        launch_datetime: rfc3339(parsed.launch_datetime),
        launch_latitude: parsed.launch_latitude,
        launch_longitude: parsed.launch_longitude,
        profile: parsed.profile.name().to_string(),
        stop_datetime: None,
        version: API_VERSION,
    };

    match &parsed.profile {
        FlightProfile::Standard {
            ascent_rate,
            burst_altitude,
            descent_rate,
        } => {
            echoed.ascent_rate = Some(*ascent_rate);
            echoed.burst_altitude = Some(*burst_altitude);
            echoed.descent_rate = Some(*descent_rate);
        }
        FlightProfile::Float {
            ascent_rate,
            float_altitude,
            stop_datetime,
        } => {
            echoed.ascent_rate = Some(*ascent_rate);
            echoed.float_altitude = Some(*float_altitude);
            echoed.stop_datetime = Some(rfc3339(*stop_datetime));
        }
        FlightProfile::Reverse { ascent_rate } => {
            echoed.ascent_rate = Some(*ascent_rate);
        }
    }

    echoed
}

fn predict_request(parsed: &ParsedRequest) -> PredictRequest {
    let (ascent_rate, burst_altitude, descent_rate) = match parsed.profile {
        FlightProfile::Standard {
            ascent_rate,
            burst_altitude,
            descent_rate,
        } => (ascent_rate, burst_altitude, descent_rate),
        FlightProfile::Float { .. } | FlightProfile::Reverse { .. } => {
            unreachable!("unsupported profiles are rejected before predict")
        }
    };

    PredictRequest {
        launch_time: parsed.launch_datetime,
        gfs_run_time: parsed.dataset,
        launch_site: Geodetic {
            lat: parsed.launch_latitude,
            lon: parsed.launch_longitude,
            alt: parsed.launch_altitude,
        },
        start_in_descent: false,
        ascent_rate_m_s: ascent_rate,
        balloon_class_g: parsed.balloon_class_g,
        gross_mass_kg: parsed.gross_mass_kg,
        ground_descend_rate_m_s: descent_rate,
        burst_altitude_m: burst_altitude,
        dt: DEFAULT_DT_S,
    }
}

fn map_predict_error(description: &str, start: DateTime<Utc>) -> ApiError {
    let lower = description.to_ascii_lowercase();
    if lower.contains("http") || lower.contains("download") || lower.contains("gfs") {
        ApiError::invalid_dataset(
            format!("No matching dataset found: {description}"),
            start,
        )
    } else if lower.contains("balloon class") {
        ApiError::request(description, start)
    } else {
        ApiError::prediction(format!("Prediction did not complete: '{description}'."), start)
    }
}

fn extract_f64(
    params: &HashMap<String, String>,
    name: &str,
    start: DateTime<Utc>,
    validator: impl Fn(f64) -> bool,
) -> Result<f64, ApiError> {
    let raw = require(params, name, start)?;
    let value = raw.parse::<f64>().map_err(|_| parse_error(name, raw, start))?;
    if !value.is_finite() || !validator(value) {
        return Err(invalid_error(name, raw, start));
    }
    Ok(value)
}

fn extract_datetime(
    params: &HashMap<String, String>,
    name: &str,
    start: DateTime<Utc>,
) -> Result<DateTime<Utc>, ApiError> {
    let raw = require(params, name, start)?;
    DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| parse_error(name, raw, start))
}

fn require<'a>(
    params: &'a HashMap<String, String>,
    name: &str,
    start: DateTime<Utc>,
) -> Result<&'a str, ApiError> {
    params.get(name).map(String::as_str).ok_or_else(|| {
        ApiError::request(format!("Parameter '{name}' not provided in request."), start)
    })
}

fn parse_error(name: &str, raw: &str, start: DateTime<Utc>) -> ApiError {
    ApiError::request(format!("Unable to parse parameter '{name}': {raw}."), start)
}

fn invalid_error(name: &str, raw: &str, start: DateTime<Utc>) -> ApiError {
    ApiError::request(format!("Invalid value for parameter '{name}': {raw}."), start)
}

fn rate_clip(rate: f64) -> f64 {
    rate.max(MIN_RATE_M_S)
}

fn datetime_at(launch_time: DateTime<Utc>, elapsed_s: f64) -> DateTime<Utc> {
    let millis = (elapsed_s * 1000.0).round() as i64;
    Utc.timestamp_millis_opt(launch_time.timestamp_millis() + millis)
        .single()
        .unwrap_or(launch_time)
}

fn wrap_lon_0_360(lon: f64) -> f64 {
    lon.rem_euclid(360.0)
}

fn wrap_lon_180(lon: f64) -> f64 {
    if lon > 180.0 {
        lon - 360.0
    } else {
        lon
    }
}

fn attachment_filename(req: &EchoedRequest, ext: &str) -> String {
    let launch = DateTime::parse_from_rfc3339(&req.launch_datetime)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let dataset = DateTime::parse_from_rfc3339(&req.dataset)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or(launch);
    format!(
        "{}_{:.4}_{:.4}_{}_{}.{}",
        launch.format("%Y%m%d-%H%M%SZ"),
        req.launch_latitude,
        req.launch_longitude,
        req.profile,
        dataset.format("%Y%m%d%HZ"),
        ext
    )
}

fn standard_kml_placemarks(prediction: &[Stage]) -> String {
    let Some(ascent) = prediction.first().and_then(|s| s.trajectory.first()) else {
        return String::new();
    };
    let burst = prediction
        .first()
        .and_then(|s| s.trajectory.last())
        .unwrap_or(ascent);
    let landing = prediction
        .last()
        .and_then(|s| s.trajectory.last())
        .unwrap_or(burst);

    format!(
        r#"
<Placemark>
<name>Balloon Launch</name>
<description>Balloon launch at {lat:.5},{lon:.5}, at {dt}.</description>
<Point><altitudeMode>absolute</altitudeMode><coordinates>{clon:.5},{clat:.5},{calt:.1}</coordinates></Point>
</Placemark>

<Placemark>
<name>Balloon Burst</name>
<description>Balloon burst at {blat:.5},{blon:.5}, at {bdt}.</description>
<Point><altitudeMode>absolute</altitudeMode><coordinates>{bclon:.5},{bclat:.5},{bcalt:.1}</coordinates></Point>
</Placemark>

<Placemark>
<name>Balloon Landing</name>
<description>Balloon landing at {llat:.5},{llon:.5}, at {ldt}.</description>
<Point><altitudeMode>absolute</altitudeMode><coordinates>{lclon:.5},{lclat:.5},{lcalt:.1}</coordinates></Point>
</Placemark>
"#,
        lat = ascent.latitude,
        lon = wrap_lon_180(ascent.longitude),
        dt = ascent.datetime,
        clon = wrap_lon_180(ascent.longitude),
        clat = ascent.latitude,
        calt = ascent.altitude,
        blat = burst.latitude,
        blon = wrap_lon_180(burst.longitude),
        bdt = burst.datetime,
        bclon = wrap_lon_180(burst.longitude),
        bclat = burst.latitude,
        bcalt = burst.altitude,
        llat = landing.latitude,
        llon = wrap_lon_180(landing.longitude),
        ldt = landing.datetime,
        lclon = wrap_lon_180(landing.longitude),
        lclat = landing.latitude,
        lcalt = landing.altitude,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ApiErrorKind;
    use space_balloon_predictor_rs::engine::simulation::{BalloonState, Trajectory};

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 16, 0, 0, 0).unwrap()
    }

    fn params(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    fn standard_params() -> HashMap<String, String> {
        params(&[
            ("launch_latitude", "35.0"),
            ("launch_longitude", "139.0"),
            ("launch_datetime", "2026-09-16T12:00:00Z"),
            ("ascent_rate", "5"),
            ("burst_altitude", "30000"),
            ("descent_rate", "6"),
        ])
    }

    #[test]
    fn missing_launch_datetime_is_request_exception() {
        let mut p = standard_params();
        p.remove("launch_datetime");
        let err = parse_request(&p, start()).unwrap_err();
        assert_eq!(err.kind, ApiErrorKind::RequestException);
        assert!(err.description.contains("launch_datetime"));
    }

    #[test]
    fn offset_datetime_is_echoed_in_utc() {
        let mut p = standard_params();
        p.insert(
            "launch_datetime".to_string(),
            "2014-08-20T00:00:00+01:00".to_string(),
        );
        let parsed = parse_request(&p, start()).unwrap();
        assert_eq!(rfc3339(parsed.launch_datetime), "2014-08-19T23:00:00Z");
    }

    #[test]
    fn rejects_longitude_outside_tawhiri_range() {
        let mut p = standard_params();
        p.insert("launch_longitude".to_string(), "-122.0".to_string());
        let err = parse_request(&p, start()).unwrap_err();
        assert_eq!(err.kind, ApiErrorKind::RequestException);
        assert!(err.description.contains("launch_longitude"));
    }

    #[test]
    fn clips_tiny_ascent_rate_like_tawhiri() {
        let mut p = standard_params();
        p.insert("ascent_rate".to_string(), "0.05".to_string());
        let parsed = parse_request(&p, start()).unwrap();
        match parsed.profile {
            FlightProfile::Standard { ascent_rate, .. } => {
                assert!((ascent_rate - 0.2).abs() < 1e-12);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_profile_is_request_exception() {
        let mut p = standard_params();
        p.insert("profile".to_string(), "nope".to_string());
        let err = parse_request(&p, start()).unwrap_err();
        assert!(err.description.contains("Unknown profile"));
    }

    #[test]
    fn unsupported_format_is_internal_exception() {
        let mut p = standard_params();
        p.insert("format".to_string(), "xml".to_string());
        let err = parse_request(&p, start()).unwrap_err();
        assert_eq!(err.kind, ApiErrorKind::InternalException);
        assert_eq!(err.description, "Format not supported: xml");
    }

    #[test]
    fn float_profile_parses_then_run_is_not_implemented() {
        let p = params(&[
            ("launch_latitude", "35.0"),
            ("launch_longitude", "139.0"),
            ("launch_datetime", "2026-09-16T12:00:00Z"),
            ("profile", "float_profile"),
            ("ascent_rate", "5"),
            ("float_altitude", "18000"),
            ("stop_datetime", "2026-09-16T18:00:00Z"),
        ]);
        let parsed = parse_request(&p, start()).unwrap();
        let err = run_prediction(&parsed, Path::new("/tmp"), start()).unwrap_err();
        assert_eq!(err.kind, ApiErrorKind::NotYetImplementedException);
    }

    #[test]
    fn path_balloon_class_overrides_default() {
        let parsed = parse_request_with_balloon_class(&standard_params(), start(), Some(1000)).unwrap();
        assert_eq!(parsed.balloon_class_g, 1000);
    }

    #[test]
    fn path_balloon_class_overrides_query_param() {
        let mut p = standard_params();
        p.insert("balloon_class".to_string(), "3000".to_string());
        let parsed = parse_request_with_balloon_class(&p, start(), Some(1500)).unwrap();
        assert_eq!(parsed.balloon_class_g, 1500);
    }

    #[test]
    fn rejects_unknown_path_balloon_class() {
        let err = parse_balloon_class("2500", start()).unwrap_err();
        assert_eq!(err.kind, ApiErrorKind::RequestException);
        assert!(err.description.contains("2500"));
    }

    #[test]
    fn stages_share_burst_point() {
        let launch = Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0).unwrap();
        let trajectory = Trajectory::new(vec![
            BalloonState {
                lat: 35.0,
                lon: 139.0,
                alt: 10.0,
                time: 0.0,
                is_burst: false,
            },
            BalloonState {
                lat: 35.1,
                lon: 139.2,
                alt: 30000.0,
                time: 3600.0,
                is_burst: true,
            },
            BalloonState {
                lat: 35.2,
                lon: 139.4,
                alt: 50.0,
                time: 5400.0,
                is_burst: true,
            },
        ]);
        let stages = stages_from_trajectory(&trajectory, launch);
        assert_eq!(stages[0].stage, "ascent");
        assert_eq!(stages[1].stage, "descent");
        assert_eq!(stages[0].trajectory.len(), 2);
        assert_eq!(stages[1].trajectory.len(), 2);
        assert_eq!(
            stages[0].trajectory.last().unwrap(),
            stages[1].trajectory.first().unwrap()
        );
        assert_eq!(stages[0].trajectory[0].datetime, "2026-09-16T12:00:00Z");
        assert_eq!(stages[1].trajectory[0].datetime, "2026-09-16T13:00:00Z");
        assert_eq!(stages[1].trajectory[1].datetime, "2026-09-16T13:30:00Z");
    }

    #[test]
    fn csv_wraps_longitude_past_180() {
        let response = SuccessResponse {
            metadata: Metadata {
                start_datetime: "2026-09-16T00:00:00Z".into(),
                complete_datetime: "2026-09-16T00:00:01Z".into(),
            },
            prediction: vec![Stage {
                stage: "ascent".into(),
                trajectory: vec![Point {
                    altitude: 10.0,
                    datetime: "2026-09-16T12:00:00Z".into(),
                    latitude: 35.0,
                    longitude: 190.0,
                }],
            }],
            request: EchoedRequest {
                ascent_rate: Some(5.0),
                burst_altitude: Some(30000.0),
                dataset: "2026-09-16T06:00:00Z".into(),
                descent_rate: Some(6.0),
                float_altitude: None,
                format: "csv".into(),
                launch_altitude: 0.0,
                launch_datetime: "2026-09-16T12:00:00Z".into(),
                launch_latitude: 35.0,
                launch_longitude: 190.0,
                profile: PROFILE_STANDARD.into(),
                stop_datetime: None,
                version: 1,
            },
            warnings: EmptyWarnings {},
        };
        let (filename, csv) = format_csv(&response);
        assert!(csv.contains("2026-09-16T12:00:00Z,35.00000,-170.00000,10.0"));
        assert!(filename.ends_with(".csv"));
        assert!(filename.contains("standard_profile"));
    }

    #[test]
    fn success_json_has_tawhiri_fragments() {
        let launch = Utc.with_ymd_and_hms(2026, 9, 16, 12, 0, 0).unwrap();
        let parsed = ParsedRequest {
            launch_latitude: 50.0,
            launch_longitude: 0.01,
            launch_datetime: launch,
            launch_altitude: 0.0,
            profile: FlightProfile::Standard {
                ascent_rate: 5.0,
                burst_altitude: 30000.0,
                descent_rate: 10.0,
            },
            dataset: None,
            format: OutputFormat::Json,
            balloon_class_g: 2000,
            gross_mass_kg: 6.0,
        };
        let result = PredictResult {
            gfs_run_time: Utc.with_ymd_and_hms(2026, 9, 16, 6, 0, 0).unwrap(),
            launch_time: launch,
            trajectory: Trajectory::new(vec![
                BalloonState {
                    lat: 50.0,
                    lon: 0.01,
                    alt: 0.0,
                    time: 0.0,
                    is_burst: false,
                },
                BalloonState {
                    lat: 50.1,
                    lon: 0.2,
                    alt: 30000.0,
                    time: 100.0,
                    is_burst: true,
                },
            ]),
            kml: String::new(),
        };
        let body = success_from_result(&parsed, &result, start(), start());
        let value = serde_json::to_value(&body).unwrap();
        assert!(value.get("request").is_some());
        assert!(value.get("prediction").is_some());
        assert!(value.get("metadata").is_some());
        assert_eq!(value["request"]["profile"], "standard_profile");
        assert_eq!(value["request"]["version"], 1);
        assert_eq!(value["prediction"][0]["stage"], "ascent");
        assert_eq!(value["prediction"][1]["stage"], "descent");
        assert_eq!(value["warnings"], serde_json::json!({}));
    }
}
