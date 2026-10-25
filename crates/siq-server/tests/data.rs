//! The data routes (ADR-0045): a dataset's OME-Zarr image as deep-zoom tiles over HTTP, the
//! descriptor, ETags, a tile's pixels through a LUT, the value probe, and the requests refused.
//! REQ-DATA-005, REQ-NET-003, REQ-VIS-001.

mod common;

use std::path::Path;

use common::config;
use http_body_util::BodyExt;
use siq_pyramid::build;
use siq_server::Service;
use siq_storage::zarr::{ChannelInfo, ImageInfo, SampleType, Size5, ZarrImage};
use siq_storage::{Dataset, DatasetKind, NewDataset};
use siq_types::{DatasetId, OwnedImage};
use teta_wot::http::axum::Router;
use teta_wot::http::axum::body::{Body, Bytes};
use teta_wot::http::axum::http::{HeaderMap, Request, StatusCode};
use tower::ServiceExt;

/// A dataset under `root` with a 1000 × 600, two-channel image, `snap`, its pyramid built.
/// Channel 1's pixel is `x + y`.
fn dataset(root: &Path) -> DatasetId {
    let dataset = Dataset::create(root, NewDataset::new(DatasetKind::Snap, "snap")).unwrap();
    let channel = |label: &str, colour| ChannelInfo {
        label: label.to_owned(),
        colour,
        window: (0.0, 2000.0),
    };
    let info = ImageInfo {
        name: "snap".to_owned(),
        sample: SampleType::U16,
        size: Size5 {
            t: 1,
            c: 2,
            z: 1,
            y: 600,
            x: 1000,
        },
        levels: 1,
        chunk: 256,
        pixel_size: [1.0, 0.376, 0.376],
        origin: [0.0; 3],
        channels: vec![channel("white", [255; 3]), channel("green", [0, 255, 0])],
    };
    let mut image = ZarrImage::create(&dataset.zarr_dir().join("snap.ome.zarr"), info).unwrap();
    for c in 0..2u64 {
        for (row, col) in (0..3).flat_map(|row| (0..4).map(move |col| (row, col))) {
            let (x0, y0) = (col * 256, row * 256);
            let (w, h) = (256.min(1000 - x0), 256.min(600 - y0));
            let chunk = OwnedImage::from_fn(w as usize, h as usize, |x, y| {
                ((x0 + x as u64 + y0 + y as u64) * c) as u16
            });
            image
                .write_chunk(0, (0, c, 0), (row, col), chunk.view())
                .unwrap();
        }
    }
    build(&mut image, 256, &()).unwrap();
    dataset.manifest().id
}

async fn get(router: &Router, path: &str, etag: Option<&str>) -> (StatusCode, HeaderMap, Bytes) {
    let mut request = Request::builder().uri(path);
    if let Some(etag) = etag {
        request = request.header("If-None-Match", etag);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    (
        parts.status,
        parts.headers,
        body.collect().await.unwrap().to_bytes(),
    )
}

#[tokio::test]
async fn a_dataset_s_image_is_served_as_deep_zoom_tiles() {
    let root = tempfile::tempdir().unwrap();
    let id = dataset(root.path());
    let mut config = config();
    config.server.data_root = root.path().to_owned();
    let service = Service::build(config).await.unwrap();
    let router = service.router();
    let base = format!("/api/siq/data/{id}/snap");

    let (status, headers, xml) = get(&router, &format!("{base}/1/0/0.dzi"), None).await;
    assert_eq!(
        (status, &headers["content-type"]),
        (StatusCode::OK, &"application/xml".parse().unwrap())
    );
    let xml = String::from_utf8(xml.to_vec()).unwrap();
    assert!(
        xml.contains(
            r#"TileSize="254" Overlap="1" Format="jpeg"><Size Width="1000" Height="600"/>"#
        ),
        "{xml}"
    );

    // A tile, then the same again: not modified.
    let tile = format!("{base}/1/0/0_files/10/1_0.jpeg");
    let (status, headers, jpeg) = get(&router, &tile, None).await;
    assert_eq!(
        (status, &headers["content-type"]),
        (StatusCode::OK, &"image/jpeg".parse().unwrap())
    );
    assert_eq!(&jpeg[..2], [0xFF, 0xD8]);
    let etag = headers["etag"].to_str().unwrap().to_owned();
    assert_eq!(headers["cache-control"], "private, no-cache");
    let (status, _, body) = get(&router, &tile, Some(&etag)).await;
    assert_eq!((status, body.len()), (StatusCode::NOT_MODIFIED, 0));

    // A PNG of level 9 (Zarr level 1) through a grey window of 0 to 1000: its first pixel is
    // level 1's (0, 0), the mean of x + y over 2 × 2, 1.
    let (status, _, png_bytes) = get(
        &router,
        &format!("{base}/1/0/0_files/9/0_0.png?min=0&max=1000&cmap=grey"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut reader = png::Decoder::new(std::io::Cursor::new(png_bytes.to_vec()))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(
        (frame.width, frame.height, frame.color_type),
        (255, 255, png::ColorType::Grayscale)
    );
    assert_eq!(pixels[0], (1.0f64 / 1000.0 * 255.0).round() as u8);
    assert_eq!(
        pixels[254],
        (509.0f64 / 1000.0 * 255.0).round() as u8,
        "level 1's (254, 0): (508 + 509 + 509 + 510) / 4"
    );

    // The true value of a pixel.
    let (status, _, body) = get(
        &router,
        &format!("{base}/value?c=1&level=0&x=999&y=599"),
        None,
    )
    .await;
    let probe: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (status, probe["value"].as_f64()),
        (StatusCode::OK, Some(1598.0))
    );

    for missing in [
        format!("/api/siq/data/{}/snap/0/0/0.dzi", DatasetId::generate()),
        "/api/siq/data/not-an-id/snap/0/0/0.dzi".to_owned(),
        format!("/api/siq/data/{id}/other/0/0/0.dzi"),
        format!("/api/siq/data/{id}/..%5C..%5Cmanifest/0/0/0.dzi"),
        format!("{base}/2/0/0.dzi"),
        format!("{base}/0/0/0_files/11/0_0.jpeg"),
        format!("{base}/0/0/0_files/10/9_9.jpeg"),
        format!("{base}/0/0/0_files/10/0_0.gif"),
        format!("{base}/value?x=1000"),
    ] {
        let (status, _, body) = get(&router, &missing, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}: {body:?}");
    }
    for bad in ["min=5&max=5", "gamma=0", "cmap=purple", "min=dark"] {
        let (status, _, body) = get(
            &router,
            &format!("{base}/0/0/0_files/10/0_0.jpeg?{bad}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body:?}");
    }

    // A dataset made since the service started is found too.
    let later = dataset(root.path());
    let (status, ..) = get(
        &router,
        &format!("/api/siq/data/{later}/snap/0/0/0.dzi"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
