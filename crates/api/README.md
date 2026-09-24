# PLANET-Q Space Balloon Predictor External API

## Acknowledgements

This API is built on [space-balloon-predictor](https://github.com/PLANET-Qdeveloper/space-balloon-predictor), developed by [PLANET-Q](https://github.com/PLANET-Qdeveloper) at Kyushu University. I apriciate for PLANET-Q for the predictor, the ascent physics model, and permission to use it as the engine behind this Tawhiri-compatible API.

Predict a space balloon trajectory with that engine and return it in the same response format as [Tawhiri](https://tawhiri.readthedocs.io/en/latest/api.html).

Balloon size (`coeff_k`) is not a Tawhiri query parameter, so it is selected in the path.

Published API URL:

```text
http://api-tarhiri.tba-lab.tech/pqruntime/{balloon_class}
```

`balloon_class`: `1000` / `1500` / `2000` / `3000` (g)

## Example

2000 g balloon:

```text
http://api-tarhiri.tba-lab.tech/pqruntime/2000?launch_latitude=35&launch_longitude=139&launch_datetime=2026-09-16T13:00:00Z&launch_altitude=10&ascent_rate=5&burst_altitude=30000&descent_rate=5
```

```bash
curl "http://api-tarhiri.tba-lab.tech/pqruntime/2000?launch_latitude=35&launch_longitude=139&launch_datetime=2026-09-16T13:00:00Z&launch_altitude=10&ascent_rate=5&burst_altitude=30000&descent_rate=5"
```

## Query parameters

The query string matches SondeHub / Tawhiri `standard_profile`. Longitude must be in **\[0, 360)**. For west longitudes, add 360.

| Parameter | Required | Detail |
| --- | --- | --- |
| `launch_latitude` | yes | Launch latitude (−90 to 90) |
| `launch_longitude` | yes | Launch longitude (0 to 360) |
| `launch_datetime` | yes | Launch time (RFC3339) |
| `ascent_rate` | yes | Target ascent rate at the ground (m/s) |
| `burst_altitude` | yes | Burst altitude (m) |
| `descent_rate` | yes | Sea-level equivalent descent rate (m/s) |
| `launch_altitude` | no | Launch elevation (m). Defaults to 0 |
| `dataset` | no | GFS cycle time (RFC3339). Defaults to the latest available cycle |
| `profile` | no | Defaults to `standard_profile` |
| `format` | no | `json` (default) / `csv` / `kml` |
| `gross_mass` | no | Gross mass in kg. Defaults to 6 |

`float_profile` and `reverse_profile` are not implemented and return `501 NotYetImplementedException`.

## Response

A successful body matches Tawhiri: `request`, `prediction`, and `metadata`. `prediction` has two stages, `ascent` and `descent`. The burst point is included in both.

```json
{
  "metadata": {
    "start_datetime": "2026-09-16T08:09:40Z",
    "complete_datetime": "2026-09-16T08:09:41Z"
  },
  "prediction": [
    { "stage": "ascent", "trajectory": [{ "latitude": 35.0, "longitude": 139.0, "altitude": 10.0, "datetime": "2026-09-16T13:00:00Z" }] },
    { "stage": "descent", "trajectory": [{ "latitude": 35.48, "longitude": 139.63, "altitude": 0.0, "datetime": "2026-09-16T15:00:58Z" }] }
  ],
  "request": {
    "ascent_rate": 5.0,
    "burst_altitude": 30000.0,
    "dataset": "2026-09-16T00:00:00Z",
    "descent_rate": 5.0,
    "format": "json",
    "launch_altitude": 10.0,
    "launch_datetime": "2026-09-16T13:00:00Z",
    "launch_latitude": 35.0,
    "launch_longitude": 139.0,
    "profile": "standard_profile",
    "version": 1
  },
  "warnings": {}
}
```

Errors use the same Tawhiri shape.

| type | HTTP |
| --- | --- |
| `RequestException` | 400 |
| `InvalidDatasetException` | 404 |
| `PredictionException` / `InternalException` | 500 |
| `NotYetImplementedException` | 501 |

```json
{
  "error": {
    "type": "RequestException",
    "description": "Parameter 'launch_datetime' not provided in request."
  },
  "metadata": {
    "start_datetime": "...",
    "complete_datetime": "..."
  }
}
```

## Difference from SondeHub

The query string and JSON shape match Tawhiri. The ascent model does not.

- SondeHub: uses `ascent_rate` as a constant speed until burst
- This API: picks `coeff_k` from the balloon class in the path, then solves for net lift so the ground rate equals `ascent_rate`. Ascent speeds up with altitude as the balloon expands

Descent is terminal velocity scaled by air density from the sea-level `descent_rate` in both systems. Wind comes from NOAA GFS.

## Run locally

Rust (edition 2024) is required. The first request downloads GFS from NOAA, so a network connection is needed. Files are reused under `CACHE_DIR`.

```bash
cargo run -p space-balloon-predictor-rs-server --release
```

The default listen address is `http://0.0.0.0:8000`.

```bash
curl "http://127.0.0.1:8000/pqruntime/2000?launch_latitude=35&launch_longitude=139&launch_datetime=2026-09-16T13:00:00Z&ascent_rate=5&burst_altitude=30000&descent_rate=5"
```

| Environment variable | Default | Description |
| --- | --- | --- |
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `8000` | Bind port |
| `CACHE_DIR` | `./cache/gfs` | GFS GRIB cache directory |
| `RUST_LOG` | `info` | Log level |

## Publish on a server

The process listens on port 8000. Put it behind Cloudflare Tunnel (or similar) as `api-tarhiri.tba-lab.tech`. If traffic only arrives through the tunnel, bind the API to `127.0.0.1`.

```bash
HOST=127.0.0.1 PORT=8000 CACHE_DIR=./cache/gfs ./target/release/space-balloon-predictor-rs-server
cloudflared tunnel --url http://127.0.0.1:8000
```

For a stable hostname, point a named tunnel published application at `http://127.0.0.1:8000`. The public path stays `/pqruntime/2000`.

## License

This project is licensed under the [MIT License](LICENSE).
