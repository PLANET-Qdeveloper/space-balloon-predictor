use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use log::info;
use space_balloon_predictor_rs_server::ApiError;
use space_balloon_predictor_rs_server::tawhiri::{
    OutputFormat, ParsedRequest, SuccessResponse, format_csv, format_kml, parse_balloon_class,
    parse_request_with_balloon_class, run_prediction,
};
use tokio::net::TcpListener;
use tower_http::cors::CorsLayer;

#[derive(Clone)]
struct AppState {
    cache_dir: Arc<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cache_dir = std::env::var("CACHE_DIR").unwrap_or_else(|_| "./cache/gfs".to_string());
    let host = std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".to_string());
    let addr: SocketAddr = format!("{host}:{port}").parse()?;

    let state = AppState {
        cache_dir: Arc::new(PathBuf::from(cache_dir)),
    };

    let app = Router::new()
        .route("/{balloon_class}", get(pqruntime_handler))
        .route("/{balloon_class}/", get(pqruntime_handler))
        .layer(CorsLayer::permissive())
        .with_state(state);

    info!("Tawhiri-compatible predictor listening on http://{addr}");

    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn predict_handler(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    predict_with_class(state, params, None).await
}

async fn pqruntime_handler(
    Path(balloon_class): Path<String>,
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let start = Utc::now();
    match parse_balloon_class(&balloon_class, start) {
        Ok(class) => predict_with_class_at(state, params, Some(class), start).await,
        Err(err) => err.into_response(),
    }
}

async fn predict_with_class(
    state: AppState,
    params: HashMap<String, String>,
    balloon_class_g: Option<u32>,
) -> Response {
    predict_with_class_at(state, params, balloon_class_g, Utc::now()).await
}

async fn predict_with_class_at(
    state: AppState,
    params: HashMap<String, String>,
    balloon_class_g: Option<u32>,
    start: DateTime<Utc>,
) -> Response {
    let parsed = match parse_request_with_balloon_class(&params, start, balloon_class_g) {
        Ok(parsed) => parsed,
        Err(err) => return err.into_response(),
    };

    info!(
        "Prediction {} {}g from ({:.4}, {:.4}) at {} UTC",
        parsed.profile.name(),
        parsed.balloon_class_g,
        parsed.launch_latitude,
        parsed.launch_longitude,
        parsed.launch_datetime
    );

    let cache_dir = state.cache_dir.clone();
    let parsed_for_job = parsed.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_prediction(&parsed_for_job, cache_dir.as_path(), start)
    })
    .await;

    match result {
        Ok(Ok(response)) => format_response(&parsed, response),
        Ok(Err(err)) => err.into_response(),
        Err(err) => {
            ApiError::internal(format!("Prediction task failed: {err}"), start).into_response()
        }
    }
}

fn format_response(parsed: &ParsedRequest, response: SuccessResponse) -> Response {
    match parsed.format {
        OutputFormat::Json => (StatusCode::OK, Json(response)).into_response(),
        OutputFormat::Csv => match attachment(format_csv(&response), "text/csv") {
            Ok(resp) => resp,
            Err(err) => err.into_response(),
        },
        OutputFormat::Kml => match format_kml(&response) {
            Ok(payload) => match attachment(payload, "application/vnd.google-earth.kml+xml") {
                Ok(resp) => resp,
                Err(err) => err.into_response(),
            },
            Err(err) => err.into_response(),
        },
    }
}

fn attachment(
    (filename, body): (String, String),
    content_type: &'static str,
) -> Result<Response, ApiError> {
    let disposition = format!("attachment; filename=\"{filename}\"");
    let disposition = HeaderValue::from_str(&disposition).map_err(|err| {
        ApiError::internal(format!("Invalid Content-Disposition: {err}"), Utc::now())
    })?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (header::CONTENT_DISPOSITION, disposition),
        ],
        body,
    )
        .into_response())
}
