//! The peripherals' Things, `imaging`, and the system routes, with the simulated instrument of
//! `config/sim.toml`: each peripheral over HTTP, a channel switch sampled in the true state, an
//! instrument with no peripherals, and the metrics and memory routes.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::{config, finished, invoke, serve};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use siq_config::model::PeripheralsConfig;
use siq_imgproc::Colormap;
use siq_server::Service;
use siq_sim::state::TrueState;
use siq_types::ChannelId;
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::Request;
use tower::ServiceExt;

#[tokio::test]
async fn illumination_sets_a_level_by_source_name_and_announces_it() {
    let (client, _) = serve(config()).await;
    let sources = client.get("/api/v1/illumination/sources").await.json();
    assert_eq!(sources, json!(["white", "365nm", "470nm", "565nm"]));
    let mut changes = client.events("/api/v1/illumination/changed").await;
    let input = json!({"source": "470nm", "level": 0.5});
    let levels = invoke(&client, "/api/v1/illumination/set_level", input).await;
    // The simulator starts with every light at full.
    assert_eq!(levels, json!([1.0, 1.0, 0.5, 1.0]));
    assert_eq!(changes.next().await, Some(levels.clone()));
    let property = client.get("/api/v1/illumination/levels").await.json();
    assert_eq!(property, levels);

    let unknown = json!({"source": "UV", "level": 1.0});
    let refused = client
        .post_json("/api/v1/illumination/set_level", Some(&unknown))
        .await;
    let refused = finished(&client, refused).await;
    assert_eq!(refused["status"], "error", "{refused}");
    assert!(refused.to_string().contains("no source `UV`"), "{refused}");
    client.stop().await;
}

#[tokio::test]
async fn the_filter_wheel_turns_then_announces_the_filter() {
    let (client, _) = serve(config()).await;
    let filters = client.get("/api/v1/filter_wheel/filters").await.json();
    assert_eq!(filters, json!(["empty", "DAPI", "GFP", "TRITC"]));
    let mut moving = client.events("/api/v1/filter_wheel/moving").await;
    let mut changes = client.events("/api/v1/filter_wheel/changed").await;
    let at = invoke(
        &client,
        "/api/v1/filter_wheel/select",
        json!({"position": 3}),
    )
    .await;
    assert_eq!(at, 3);
    assert_eq!(changes.next().await, Some(json!(3)));
    // It turned, and stopped.
    let mut seen = Vec::new();
    while let Some(value) = moving.next_within(Duration::from_millis(300)).await {
        seen.push(value);
    }
    assert!(seen.contains(&json!(true)), "{seen:?}");
    assert_eq!(
        client.get("/api/v1/filter_wheel/moving").await.json(),
        false
    );
    assert_eq!(client.get("/api/v1/filter_wheel/position").await.json(), 3);
    client.stop().await;
}

#[tokio::test]
async fn selecting_an_objective_labels_the_frames_with_it() {
    let (client, instrument) = serve(config()).await;
    let objectives = client.get("/api/v1/objective/objectives").await.json();
    assert_eq!(objectives.as_array().map(Vec::len), Some(3), "{objectives}");
    let objective = invoke(&client, "/api/v1/objective/select", json!({"position": 2})).await;
    assert_eq!(objective["name"], "60x/1.40 oil", "{objective}");
    let current = client.get("/api/v1/objective/current").await.json();
    assert_eq!(current, objective);
    let context = instrument.acquisition.context();
    assert_eq!(context.objective.map(|id| id.0), Some(3));

    // Position 3 has no configured objective, so the frames have none.
    let none = invoke(&client, "/api/v1/objective/select", json!({"position": 3})).await;
    assert_eq!(none, Value::Null);
    assert_eq!(instrument.acquisition.context().objective, None);
    client.stop().await;
}

#[tokio::test]
async fn the_modality_unit_takes_a_mode_and_its_pattern() {
    let (client, _) = serve(config()).await;
    let patterns = client.get("/api/v1/modality/patterns").await.json();
    assert_eq!(patterns, json!({"orientations": 3, "phases": 3}));
    let modes = client.get("/api/v1/modality/modes").await.json();
    assert_eq!(modes.as_array().map(Vec::len), Some(4), "{modes}");
    let mut changes = client.events("/api/v1/modality/changed").await;
    let input = json!({
        "mode": "structured_illumination",
        "pattern": {"orientation": 1, "phase": 2},
    });
    let modality = invoke(&client, "/api/v1/modality/set_modality", input.clone()).await;
    assert_eq!(modality, input);
    assert_eq!(changes.next().await, Some(input.clone()));
    assert_eq!(client.get("/api/v1/modality/modality").await.json(), input);
    // The pattern defaults to none.
    let confocal = json!({"mode": "confocal"});
    let modality = invoke(&client, "/api/v1/modality/set_modality", confocal).await;
    assert_eq!(modality, json!({"mode": "confocal", "pattern": null}));

    let outside = json!({
        "mode": "structured_illumination",
        "pattern": {"orientation": 3, "phase": 0},
    });
    let refused = client
        .post_json("/api/v1/modality/set_modality", Some(&outside))
        .await;
    let refused = finished(&client, refused).await;
    assert_eq!(refused["status"], "error", "{refused}");
    client.stop().await;
}

/// One look at the light path: the filter, and each source's level.
#[derive(Debug, Clone, PartialEq)]
struct Sample {
    filter: Option<u8>,
    levels: [f64; 4],
}

/// Samples the light path as fast as it can until `done`. A sample is kept only if the filter was
/// the same before and after the levels were read.
fn sample(state: &TrueState, done: &AtomicBool) -> Vec<Sample> {
    let mut samples = Vec::new();
    while !done.load(Ordering::Relaxed) {
        let filter = state.filter();
        let levels = std::array::from_fn(|source| state.level(source));
        if state.filter() == filter {
            samples.push(Sample { filter, levels });
        }
        std::thread::yield_now();
    }
    samples
}

#[tokio::test]
async fn a_channel_switch_darkens_the_old_light_before_the_filter_moves_and_lights_the_new_one_after()
 {
    let (client, instrument) = serve(config()).await;
    let channels = client.get("/api/v1/imaging/channels").await.json();
    assert_eq!(channels.as_array().map(Vec::len), Some(3), "{channels}");
    let dapi = invoke(&client, "/api/v1/imaging/select_channel", json!({"id": 2})).await;
    assert_eq!(dapi["name"], "DAPI");
    assert_eq!(instrument.state.level(1), 0.8);
    assert_eq!(instrument.state.filter(), Some(1));
    let mut changes = client.events("/api/v1/imaging/channel_changed").await;

    // From DAPI (365 nm, the filter at 1) to GFP (470 nm, the filter at 2), sampled meanwhile.
    let (state, done) = (instrument.state.clone(), Arc::new(AtomicBool::new(false)));
    let sampler = std::thread::spawn({
        let done = done.clone();
        move || sample(&state, &done)
    });
    let began = Instant::now();
    let gfp = invoke(&client, "/api/v1/imaging/select_channel", json!({"id": 3})).await;
    let took = began.elapsed();
    done.store(true, Ordering::Relaxed);
    let samples = sampler.join().unwrap();

    assert_eq!(gfp["name"], "GFP");
    assert!(samples.len() > 100, "{} samples", samples.len());
    for sample in &samples {
        let lit = sample.levels.iter().filter(|level| **level > 0.0).count();
        assert!(lit <= 1, "two lights at once: {sample:?}");
        if sample.filter.is_none() {
            assert_eq!(lit, 0, "light while the wheel turns: {sample:?}");
        }
        if sample.levels[2] > 0.0 {
            assert_eq!(
                sample.filter,
                Some(2),
                "470 nm before its filter: {sample:?}"
            );
        }
        if sample.levels[1] > 0.0 {
            assert_eq!(
                sample.filter,
                Some(1),
                "365 nm after its filter: {sample:?}"
            );
        }
    }
    assert!(samples.iter().any(|sample| sample.filter.is_none()));
    let last = Sample {
        filter: Some(2),
        levels: [0.0, 0.0, 0.6, 0.0],
    };
    assert_eq!(samples.last(), Some(&last));
    // The light off (5 ms), a position of the wheel (60 ms), the light on (5 ms), and half the
    // 80 ms exposure before the frames are labelled.
    assert!(took >= Duration::from_millis(110), "{took:?}");

    assert_eq!(changes.next().await, Some(json!(3)));
    assert_eq!(client.get("/api/v1/imaging/channel").await.json(), 3);
    assert_eq!(instrument.acquisition.context().channel, Some(ChannelId(3)));
    let exposure = instrument.config.channels[2].exposure;
    assert_eq!(instrument.camera.settings().exposure, exposure);
    let colormap = instrument.previews.display().colormap;
    assert_eq!(colormap, Colormap::Colour([0x3f, 0xff, 0x6f]));
    client.stop().await;
}

#[tokio::test]
async fn without_peripherals_only_imaging_is_added_and_a_channel_sets_the_camera() {
    let mut bare = config();
    bare.peripherals = PeripheralsConfig::default();
    let (client, instrument) = serve(bare).await;
    let things = client.get("/api/v1/things/").await.json();
    let mut names: Vec<_> = things.as_object().unwrap().keys().cloned().collect();
    names.sort();
    // `calibration`, `jobs` and `datasets` are served whatever is fitted.
    assert_eq!(
        names,
        [
            "calibration",
            "camera",
            "datasets",
            "display",
            "focus",
            "imaging",
            "jobs",
            "stage"
        ]
    );
    let brightfield = invoke(&client, "/api/v1/imaging/select_channel", json!({"id": 1})).await;
    assert_eq!(brightfield["name"], "Brightfield");
    let exposure = instrument.config.channels[0].exposure;
    assert_eq!(instrument.camera.settings().exposure, exposure);
    assert_eq!(instrument.previews.display().colormap, Colormap::Grey);

    let refused = client
        .post_json("/api/v1/imaging/select_channel", Some(&json!({"id": 9})))
        .await;
    let refused = finished(&client, refused).await;
    assert!(refused.to_string().contains("no channel 9"), "{refused}");
    client.stop().await;
}

async fn get(service: &Service, path: &str) -> Value {
    let request = Request::get(path).body(Body::empty()).unwrap();
    let response = service.router().oneshot(request).await.unwrap();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn the_metrics_route_has_the_pool_previews_and_acquisition() {
    let service = Service::build(config()).await.unwrap();
    let metrics = get(&service, "/api/siq/system/metrics").await;
    let kinds: serde_json::Map<_, _> = metrics
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, metric)| (name.clone(), metric["kind"].clone()))
        .collect();
    insta::assert_json_snapshot!("metrics", kinds);
    assert_eq!(metrics["camera.pool.free_slots"]["value"], 6.0);
    assert_eq!(metrics["acquisition.published"]["value"], 0.0);
}

#[tokio::test]
async fn the_memory_route_has_the_pool_the_process_and_the_budgets() {
    let service = Service::build(config()).await.unwrap();
    let memory = get(&service, "/api/siq/system/memory").await;
    let pool = &memory["pool"];
    assert_eq!(pool["slots"], 6, "{memory}");
    assert_eq!(pool["slot_bytes"], 256 * 192 * 2);
    assert_eq!(pool["free"], 6);
    let resident = memory["process"]["working_set"].as_u64().unwrap();
    assert!(resident > 1 << 20, "{memory}");
    let peak = memory["process"]["peak_working_set"].as_u64().unwrap();
    assert!(peak >= resident, "{memory}");
    assert_eq!(memory["budgets"]["job_ram"], 14e9);
    assert_eq!(memory["budgets"]["tile_cache"], 1024e6);
}
