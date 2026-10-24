//! SpectrumIQ's service core (REQ-NET-001, REQ-NET-002, ADR-0003, ADR-0033, ADR-0034): the devices
//! chosen from the configuration, the engines, and the instrument's Things on teta-wot, with
//! SpectrumIQ's own routes in front.
//!
//! - **Things,** under `/api/v1/`: `camera`, `display`, `stage` and `focus`. The global lock is on:
//!   hardware actions and writes take it, and `stage.stop` and `focus.stop` don't.
//! - **Routes,** under `/api/siq/`: `system/health`. teta-wot's router serves every other request.
//! - **The lifecycle,** as teta-wot's `serve_with`, but with the routes merged, and live view and
//!   motion stopped before the Things are.
//!
//! This is the only crate that depends on teta-wot.

use std::future::{Future, IntoFuture};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use serde_json::{Value, json};
use siq_acquire::Acquisition;
use siq_buffer::PoolConfig;
use siq_config::model::InstrumentConfig;
use siq_devices::FocusDrive;
use siq_preview::{PreviewConfig, Previews};
use siq_sim::state::TrueState;
use siq_telemetry::Registry;
use siq_types::{Clock, SystemClock};
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::Request;
use teta_wot::http::axum::routing::get;
use teta_wot::http::axum::{self, Json, Router};
use teta_wot::server::{ThingServer, ThingServerBuilder};
use tokio::net::TcpListener;
use tower::ServiceExt;
use tracing::{info, warn};

pub mod devices;
pub mod things;

pub use teta_wot::server::shutdown_signal;

use things::{Camera, Display, Focus, LatestStats, Stage};

/// The prefix of the Things' routes.
pub const API_PREFIX: &str = "/api/v1";

/// The instrument's devices and engines: the service each Thing uses.
pub struct Instrument {
    /// The configuration it was made from.
    pub config: InstrumentConfig,
    /// The camera, which the acquisition engine drives.
    pub camera: Arc<dyn siq_devices::Camera>,
    /// The acquisition engine.
    pub acquisition: Acquisition,
    /// The live previews.
    pub previews: Previews,
    pub(crate) stats: Arc<LatestStats>,
    /// The XYZ stage.
    pub stage: Arc<dyn siq_devices::Stage>,
    /// The focus drive.
    pub focus: Arc<dyn FocusDrive>,
    /// The simulated instrument's true state.
    pub state: Arc<TrueState>,
    /// The metrics.
    pub registry: Registry,
    started: Instant,
}

impl Instrument {
    /// Stops live view, then any motion: the hardware is safe before the Things stop.
    pub async fn shutdown(&self) {
        if let Err(error) = self.acquisition.stop().await {
            warn!(%error, "live view didn't stop");
        }
        let (stage, focus) = tokio::join!(self.stage.stop(), self.focus.stop());
        for (device, stopped) in [("stage", stage), ("focus drive", focus)] {
            if let Err(error) = stopped {
                warn!(%error, "the {device} didn't stop");
            }
        }
    }

    fn health(&self) -> Value {
        json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": self.started.elapsed().as_secs_f64(),
            "camera": self.camera.info().model,
        })
    }
}

/// The Things, with their names and the server's options.
fn things(camera: Camera) -> ThingServerBuilder {
    ThingServer::builder()
        .api_prefix(API_PREFIX)
        .api_info("SpectrumIQ", env!("CARGO_PKG_VERSION"))
        .global_lock(true)
        .thing("camera", camera)
        .thing("display", Display::default())
        .thing("stage", Stage::default())
        .thing("focus", Focus::default())
}

/// The service: the Things' server, and the instrument behind it.
pub struct Service {
    server: ThingServer,
    instrument: Arc<Instrument>,
}

impl Service {
    /// Opens the devices of `config`, starts the engines, and makes the Things.
    pub async fn build(config: InstrumentConfig) -> anyhow::Result<Self> {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let devices = devices::open(&config, clock.clone()).await?;
        let capabilities = devices.camera.capabilities();
        // Each slot holds a full frame at 16 bits.
        let full = capabilities.sensor_width as usize * capabilities.sensor_height as usize * 2;
        let pool = PoolConfig {
            name: "camera.pool".into(),
            slots: config.memory.pool_slots,
            slot_bytes: full,
            backend: devices.backend,
        };
        let acquisition = Acquisition::new(devices.camera.clone(), &pool, clock.clone())?;
        acquisition.follow_stage(devices.stage.status());
        acquisition.follow_focus(devices.focus.status());
        let (camera, stats, registry) = (
            Camera::default(),
            Arc::new(LatestStats::default()),
            Registry::new(),
        );
        let sink = Arc::new(camera.streams());
        let config_previews = PreviewConfig::default();
        let previews = Previews::start(
            &acquisition,
            &config_previews,
            sink,
            stats.clone(),
            clock,
            Some(&registry),
        )?;
        let instrument = Arc::new(Instrument {
            config,
            camera: devices.camera,
            acquisition,
            previews,
            stats,
            stage: devices.stage,
            focus: devices.focus,
            state: devices.state,
            registry,
            started: Instant::now(),
        });
        let server = things(camera).service(instrument.clone()).build()?;
        Ok(Self { server, instrument })
    }

    /// The instrument.
    pub fn instrument(&self) -> &Arc<Instrument> {
        &self.instrument
    }

    /// The Things' server and the instrument, apart: for an in-process test client.
    pub fn into_parts(self) -> (ThingServer, Arc<Instrument>) {
        (self.server, self.instrument)
    }

    /// Every route: SpectrumIQ's first, and teta-wot's for any request they don't match.
    pub fn router(&self) -> Router {
        routes(&self.instrument).fallback_service(self.server.router())
    }

    /// Starts the Things, serves on `listener` until `shutdown` completes, then shuts down: live
    /// view and motion stop, connections close, invocations are cancelled and given the grace
    /// period, and the Things stop in reverse order.
    pub async fn serve(
        self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let router = self.router();
        let Self { server, instrument } = self;
        let (runtime, grace) = (Arc::clone(server.runtime()), server.shutdown_grace());
        drop(server);
        runtime
            .start()
            .await
            .map_err(|failure| anyhow::anyhow!("{failure}"))?;
        info!("serving on http://{}", listener.local_addr()?);

        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        let serving = axum::serve(listener, router).with_graceful_shutdown(async move {
            let _ = stopped.wait_for(|stop| *stop).await;
        });
        let mut serving = tokio::spawn(serving.into_future());
        tokio::select! {
            result = &mut serving => {
                // The server stopped by itself: an I/O error.
                instrument.shutdown().await;
                runtime.shutdown(grace).await;
                return Ok(result??);
            }
            () = shutdown => {}
        }
        info!("shutting down");
        let _ = stop.send(true);
        instrument.shutdown().await;
        // Ends observation streams, which would otherwise hold their connections open.
        runtime.broker().close();
        runtime.invocations().cancel_all();
        let drained = tokio::time::timeout(grace, async {
            let _ = (&mut serving).await;
            runtime.invocations().wait_idle().await;
        });
        if drained.await.is_err() {
            warn!("requests or invocations were still running after {grace:?}");
            serving.abort();
        }
        runtime.stop().await;
        info!("stopped");
        Ok(())
    }
}

/// SpectrumIQ's own routes.
fn routes(instrument: &Arc<Instrument>) -> Router {
    let instrument = Arc::clone(instrument);
    let health = move || {
        let instrument = Arc::clone(&instrument);
        async move { Json(instrument.health()) }
    };
    Router::new().route("/api/siq/system/health", get(health))
}

/// The Things' OpenAPI document. It needs no devices.
pub async fn openapi() -> anyhow::Result<Value> {
    let server = things(Camera::default()).build()?;
    let request = Request::get("/openapi.json").body(Body::empty())?;
    let Ok(response) = server.router().oneshot(request).await;
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    serde_json::from_slice(&body).context("the OpenAPI document")
}
