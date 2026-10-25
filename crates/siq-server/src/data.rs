//! SpectrumIQ's data routes (ADR-0045): deep-zoom tiles of a dataset's OME-Zarr images, rendered
//! on demand, and the true value of a pixel. `{image}` names an image of the dataset's `zarr/`
//! folder, without `.ome.zarr`; `{dataset}` is the dataset's id.
//!
//! - `GET /api/siq/data/{dataset}/{image}/{c}/{z}/{t}.dzi`: the plane's DZI descriptor;
//! - `GET /api/siq/data/{dataset}/{image}/{c}/{z}/{t}_files/{level}/{col}_{row}.{jpeg|png}`: a
//!   tile, with `min`, `max`, `gamma` and `cmap` (`grey`, `inverted` or `rrggbb`) in the query,
//!   each the channel's own if not given. OpenSeadragon repeats the descriptor's query on its
//!   tiles;
//! - `GET /api/siq/data/{dataset}/{image}/value?c=&z=&t=&level=&x=&y=`: `{"value": …}`, at a Zarr
//!   level.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use siq_imgproc::Colormap;
use siq_pyramid::tiles::{TileAt, TileError, TileFormat, TileLut, channel_lut, value};
use siq_types::DatasetId;
use teta_wot::http::axum::extract::{Path as UrlPath, Query, State};
use teta_wot::http::axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use teta_wot::http::axum::response::{IntoResponse, Response};
use teta_wot::http::axum::routing::get;
use teta_wot::http::axum::{Json, Router};

use crate::Instrument;

type Params = HashMap<String, String>;

/// The data routes.
pub(crate) fn routes(instrument: &Arc<Instrument>) -> Router {
    Router::new()
        .route("/api/siq/data/{dataset}/{image}/value", get(probe))
        .route(
            "/api/siq/data/{dataset}/{image}/{c}/{z}/{t}",
            get(descriptor),
        )
        .route(
            "/api/siq/data/{dataset}/{image}/{c}/{z}/{t}/{level}/{tile}",
            get(tile),
        )
        .with_state(Arc::clone(instrument))
}

/// A failed request's answer: 404, 400 or 500, with its reason.
struct Refusal(StatusCode, String);

impl From<TileError> for Refusal {
    fn from(error: TileError) -> Self {
        let status = match error {
            TileError::NotFound(_) => StatusCode::NOT_FOUND,
            TileError::BadRequest(_) => StatusCode::BAD_REQUEST,
            TileError::Storage(_) | TileError::Encode(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self(status, error.to_string())
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "detail": self.1 }))).into_response()
    }
}

fn not_found(why: impl Into<String>) -> Refusal {
    Refusal(StatusCode::NOT_FOUND, why.into())
}

fn bad(why: impl Into<String>) -> Refusal {
    Refusal(StatusCode::BAD_REQUEST, why.into())
}

/// The folder of image `image` of dataset `dataset`, looking the dataset up again if it isn't
/// known yet.
fn image_path(instrument: &Instrument, dataset: &str, image: &str) -> Result<PathBuf, Refusal> {
    let id: DatasetId = dataset
        .parse()
        .map_err(|_| not_found(format!("{dataset} isn't a dataset's id")))?;
    let catalog = &instrument.catalog;
    let summary = catalog.get(id).or_else(|| {
        let _ = catalog.refresh();
        catalog.get(id)
    });
    let summary = summary.ok_or_else(|| not_found(format!("there is no dataset {id}")))?;
    let plain = image
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    let path = summary.path.join("zarr").join(format!("{image}.ome.zarr"));
    if !plain || image.starts_with('.') || !path.is_dir() {
        return Err(not_found(format!("dataset {id} has no image {image:?}")));
    }
    Ok(path)
}

fn number<T: std::str::FromStr>(text: &str, what: &str) -> Result<T, Refusal> {
    text.parse()
        .map_err(|_| not_found(format!("{text:?} isn't a {what}")))
}

/// The LUT a query asks for: the channel's own, with what it gives instead.
fn lut(query: &Params, own: TileLut) -> Result<Option<TileLut>, Refusal> {
    if !["min", "max", "gamma", "cmap"]
        .iter()
        .any(|name| query.contains_key(*name))
    {
        return Ok(None);
    }
    let value = |name: &str, default: f64| -> Result<f64, Refusal> {
        query.get(name).map_or(Ok(default), |text| {
            text.parse()
                .map_err(|_| bad(format!("{name}={text:?} isn't a number")))
        })
    };
    let colormap = match query.get("cmap").map(|c| c.trim_start_matches('#')) {
        None => own.colormap,
        Some("grey" | "gray") => Colormap::Grey,
        Some("inverted") => Colormap::Inverted,
        Some(hex) if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) => {
            let byte = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
            Colormap::Colour([byte(0), byte(2), byte(4)])
        }
        Some(other) => {
            return Err(bad(format!(
                "cmap={other:?} isn't grey, inverted or rrggbb"
            )));
        }
    };
    let lut = TileLut {
        min: value("min", own.min)?,
        max: value("max", own.max)?,
        gamma: value("gamma", own.gamma)?,
        colormap,
    };
    lut.check()?;
    Ok(Some(lut))
}

async fn descriptor(
    State(instrument): State<Arc<Instrument>>,
    UrlPath((dataset, image, c, z, t)): UrlPath<(String, String, String, String, String)>,
) -> Result<Response, Refusal> {
    let t = t
        .strip_suffix(".dzi")
        .ok_or_else(|| not_found("a descriptor ends in .dzi"))?;
    let path = image_path(&instrument, &dataset, &image)?;
    let (c, z, t): (u64, u64, u64) = (
        number(&c, "channel")?,
        number(&z, "plane")?,
        number(t, "time point")?,
    );
    let (opened, _) = instrument.tiles.image(&path)?;
    let size = opened.info().size;
    if c >= size.c || z >= size.z || t >= size.t {
        return Err(not_found(format!("the image has no plane ({t}, {c}, {z})")));
    }
    let xml = instrument.tiles.descriptor(&path, TileFormat::Jpeg)?;
    Ok(([(header::CONTENT_TYPE, "application/xml")], xml).into_response())
}

async fn tile(
    State(instrument): State<Arc<Instrument>>,
    UrlPath((dataset, image, c, z, t, level, tile)): UrlPath<(
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    )>,
    Query(query): Query<Params>,
    headers: HeaderMap,
) -> Result<Response, Refusal> {
    let t = t
        .strip_suffix("_files")
        .ok_or_else(|| not_found("tiles are in a _files folder"))?;
    let path = image_path(&instrument, &dataset, &image)?;
    let (name, extension) = tile
        .split_once('.')
        .ok_or_else(|| not_found("a tile has an extension"))?;
    let format = TileFormat::from_extension(extension)
        .ok_or_else(|| not_found(format!("tiles aren't .{extension}")))?;
    let (col, row) = name
        .split_once('_')
        .ok_or_else(|| not_found("a tile is <col>_<row>"))?;
    let at = TileAt {
        plane: (
            number(t, "time point")?,
            number(&c, "channel")?,
            number(&z, "plane")?,
        ),
        level: number(&level, "level")?,
        col: number(col, "column")?,
        row: number(row, "row")?,
    };
    let (opened, _) = instrument.tiles.image(&path)?;
    if at.plane.1 >= opened.info().size.c {
        return Err(not_found(format!(
            "the image has no channel {}",
            at.plane.1
        )));
    }
    let lut = lut(&query, channel_lut(&opened, at.plane.1))?;
    let tiles = Arc::clone(&instrument.tiles);
    let rendered = tokio::task::spawn_blocking(move || tiles.tile(&path, at, lut, format))
        .await
        .map_err(|error| Refusal(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))??;
    let etag = HeaderValue::from_str(&rendered.etag).expect("an ETag is ASCII");
    let cache = (
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-cache"),
    );
    if headers.get(header::IF_NONE_MATCH) == Some(&etag) {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag), cache]).into_response());
    }
    let kind = HeaderValue::from_static(format.content_type());
    Ok((
        [(header::CONTENT_TYPE, kind), (header::ETAG, etag), cache],
        rendered.bytes.clone(),
    )
        .into_response())
}

async fn probe(
    State(instrument): State<Arc<Instrument>>,
    UrlPath((dataset, image)): UrlPath<(String, String)>,
    Query(query): Query<Params>,
) -> Result<Response, Refusal> {
    let path = image_path(&instrument, &dataset, &image)?;
    let get = |name: &str| -> Result<u64, Refusal> {
        let text = query.get(name).map_or("0", String::as_str);
        text.parse()
            .map_err(|_| bad(format!("{name}={text:?} isn't a whole number")))
    };
    let (c, z, t, level) = (get("c")?, get("z")?, get("t")?, get("level")?);
    let (x, y) = (get("x")?, get("y")?);
    let (opened, _) = instrument.tiles.image(&path)?;
    let (x, y) = (
        u32::try_from(x).unwrap_or(u32::MAX),
        u32::try_from(y).unwrap_or(u32::MAX),
    );
    let found = value(&opened, (t, c, z), level as usize, x, y)?;
    Ok(Json(json!({ "value": found, "level": level, "x": x, "y": y })).into_response())
}
