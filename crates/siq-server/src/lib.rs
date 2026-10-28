//! SpectrumIQ's service core (REQ-NET-001, REQ-NET-002, ADR-0003, ADR-0033, ADR-0034): the devices
//! chosen from the configuration, the engines, and the instrument's Things on teta-wot, with
//! SpectrumIQ's own routes in front.
//!
//! - **Things,** under `/api/v1/`: `camera`, `display`, `stage`, `focus`, the fitted peripherals
//!   (`illumination`, `filter_wheel`, `objective`, `modality`), `imaging`, `jobs`, `datasets`,
//!   `calibration`, `workflow` and `mosaic_live`.
//!   The global lock is on: hardware actions and writes take it, and `stage.halt`, `focus.halt`,
//!   `jobs.cancel`, the `datasets` actions other than `snap_to_dataset`, the `workflow`
//!   actions other than its runs, and `mosaic_live`, don't. A run holds it throughout.
//! - **Routes,** under `/api/siq/system/`: `health`, `metrics` and `memory`; and under
//!   `/api/siq/data/`, a dataset's manifest and thumbnail, and its images' deep-zoom tiles and
//!   pixel values (ADR-0045).
//! - **The web app** at `/` and `/assets/…`, embedded in release builds (ADR-0036).
//!
//! teta-wot's router serves every other request.
//! - **The lifecycle,** as teta-wot's `serve_with`, but with the routes merged, and live view and
//!   motion stopped before the Things are.
//!
//! This is the only crate that depends on teta-wot.

use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock, PoisonError, RwLock};
use std::time::Instant;

use anyhow::Context;
use serde_json::{Value, json};
use siq_acquire::Acquisition;
use siq_buffer::{FramePool, PoolConfig};
use siq_config::model::InstrumentConfig;
use siq_devices::FocusDrive;
use siq_gpu::{Compute, GpuOptions};
use siq_preview::{PreviewConfig, Previews};
use siq_sim::state::TrueState;
use siq_telemetry::Registry;
use siq_types::{Clock, SystemClock};
use teta_wot::http::axum::body::Body;
use teta_wot::http::axum::http::Request;
use teta_wot::http::axum::middleware;
use teta_wot::http::axum::routing::get;
use teta_wot::http::axum::{self, Json, Router};
use teta_wot::server::{ThingServer, ThingServerBuilder};
use tokio::net::TcpListener;
use tower::ServiceExt;
use tracing::{info, warn};

mod audit;
pub mod auth;
pub mod authz;
pub mod calibration;
pub mod control;
mod data;
pub mod datasets;
pub mod devices;
pub mod imaging;
pub mod jobs;
pub mod mosaic;
pub mod peripherals;
mod stitch;
pub mod things;
pub mod webapp;
pub mod workflow;

pub use teta_wot::server::shutdown_signal;

use auth::Auth;
use calibration::Calibration;
use control::Control;
use datasets::Datasets;
use devices::Peripherals;
use imaging::Imaging;
use jobs::Jobs;
use mosaic::MosaicLive;
use peripherals::{FilterWheel, Illumination, ModalityUnit, Objective};
use siq_calib::CalibrationRegistry;
use siq_calib::shading::ShadingRegistry;
use siq_jobs::{Scheduler, SchedulerConfig};
use siq_pyramid::tiles::TileService;
use siq_storage::Catalog;
use things::{Camera, Display, Focus, LatestStats, Stage};
use workflow::Workflow;

/// Told when a workflow's dataset is complete: M8 queues its stitching so.
pub trait DatasetHook: Send + Sync {
    /// The run that made a dataset has ended, however it ended: `report` says how. It should
    /// queue its work, not do it: the run's invocation waits for it.
    fn on_dataset_complete(
        &self,
        instrument: &Instrument,
        report: &siq_workflow::engine::RunReport,
    );
}

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
    /// The fitted peripherals.
    pub peripherals: Peripherals,
    /// The simulated instrument's true state.
    pub state: Arc<TrueState>,
    /// The metrics.
    pub registry: Registry,
    /// The background jobs (ADR-0043).
    pub jobs: Arc<Scheduler>,
    /// The datasets of `[server] data_root` (ADR-0041).
    pub catalog: Arc<Catalog>,
    /// The deep-zoom tiles, within `[memory] tile_cache_mb` (ADR-0045).
    pub tiles: Arc<TileService>,
    /// The calibrations of `[server] settings`, versioned (ADR-0049).
    pub calibrations: Arc<CalibrationRegistry>,
    /// The darks and flats of `[server] settings`, versioned (ADR-0050).
    pub shading: Arc<ShadingRegistry>,
    /// The clock the drivers, the acquisition and the workflows keep time on.
    pub clock: Arc<dyn Clock>,
    /// The accounts and sessions (ADR-0065).
    pub auth: Arc<Auth>,
    pub(crate) gpu: OnceLock<Compute>,
    /// Whether a finished tile scan is stitched by itself: `jobs.auto_stitch`.
    pub(crate) auto_stitch: AtomicBool,
    hooks: RwLock<Vec<Arc<dyn DatasetHook>>>,
    slot_bytes: usize,
    started: Instant,
}

impl Instrument {
    /// The GPU of `[gpu]`, opened the first time it is asked for, or why there is none.
    pub fn gpu(&self) -> &Compute {
        self.gpu.get_or_init(|| {
            let gpu = &self.config.gpu;
            Compute::open(&GpuOptions {
                cpu: !gpu.enabled,
                software: gpu.software,
            })
        })
    }

    /// The instrument as the workflows see it.
    pub fn devices(&self) -> siq_workflow::DeviceSet {
        let peripherals = &self.peripherals;
        siq_workflow::DeviceSet {
            acquisition: self.acquisition.clone(),
            camera: Arc::clone(&self.camera),
            stage: Arc::clone(&self.stage),
            focus: Arc::clone(&self.focus),
            illumination: peripherals.illumination.clone(),
            filter_wheel: peripherals.filter_wheel.clone(),
            turret: peripherals.turret.clone(),
            modality: peripherals.modality.clone(),
            channels: self.config.channels.clone(),
            calibrations: Arc::clone(&self.calibrations),
            clock: Arc::clone(&self.clock),
        }
    }

    /// Tells `hook` of every workflow's dataset completed from now on.
    pub fn on_dataset_complete(&self, hook: Arc<dyn DatasetHook>) {
        self.hooks
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(hook);
    }

    /// The hooks told of completed datasets.
    pub(crate) fn hooks(&self) -> Vec<Arc<dyn DatasetHook>> {
        self.hooks
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Stops live view, then any motion: the hardware is safe before the Things stop. The jobs
    /// are cancelled, and waited for.
    pub async fn shutdown(&self) {
        let jobs = Arc::clone(&self.jobs);
        let stopping = tokio::task::spawn_blocking(move || jobs.shutdown());
        if let Err(error) = self.acquisition.stop().await {
            warn!(%error, "live view didn't stop");
        }
        let (stage, focus) = tokio::join!(self.stage.stop(), self.focus.stop());
        for (device, stopped) in [("stage", stage), ("focus drive", focus)] {
            if let Err(error) = stopped {
                warn!(%error, "the {device} didn't stop");
            }
        }
        let _ = stopping.await;
    }

    fn health(&self) -> Value {
        json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "uptime_s": self.started.elapsed().as_secs_f64(),
            "camera": self.camera.info().model,
        })
    }

    /// The registry's metrics, with the acquisition's statistics as gauges.
    fn metrics(&self) -> Value {
        let stats = self.acquisition.stats();
        let gauges = [
            ("acquisition.fps", stats.fps),
            ("acquisition.published", stats.published as f64),
            ("acquisition.lost_no_slot", stats.lost_no_slot as f64),
            ("acquisition.lost_device", stats.lost_device as f64),
            ("acquisition.waiting", stats.waiting as f64),
        ];
        for (name, value) in gauges {
            if let Ok(gauge) = self.registry.gauge(name) {
                gauge.set(value);
            }
        }
        serde_json::to_value(self.registry.snapshot()).unwrap_or_default()
    }

    /// The frame pool, the process's memory, and the configured budgets, in bytes.
    fn memory(&self) -> Value {
        let pool = self.acquisition.pool_stats();
        let process = siq_buffer::process_memory().ok().map(|memory| {
            json!({
                "working_set": memory.working_set,
                "peak_working_set": memory.peak_working_set,
                "private": memory.private,
            })
        });
        let budgets = &self.config.memory;
        json!({
            "pool": {
                "slots": pool.slots,
                "slot_bytes": self.slot_bytes,
                "free": pool.free,
                "filling": pool.filling,
                "leased": pool.leased,
                "high_water": pool.high_water,
            },
            "process": process,
            "budgets": {
                "job_ram": budgets.job_ram_gb * 1e9,
                "job_vram": budgets.job_vram_gb * 1e9,
                "tile_cache": f64::from(budgets.tile_cache_mb) * 1e6,
            },
        })
    }
}

/// The Things, with their names and the server's options: the peripherals' Things only for those
/// `fitted` says.
fn things(camera: Camera, fitted: impl Fn(&str) -> bool) -> ThingServerBuilder {
    let mut builder = ThingServer::builder()
        .api_prefix(API_PREFIX)
        .api_info("SpectrumIQ", env!("CARGO_PKG_VERSION"))
        .global_lock(true)
        .thing("camera", camera)
        .thing("display", Display::default())
        .thing("stage", Stage::default())
        .thing("focus", Focus::default());
    if fitted("illumination") {
        builder = builder.thing("illumination", Illumination::default());
    }
    if fitted("filter_wheel") {
        builder = builder.thing("filter_wheel", FilterWheel::default());
    }
    if fitted("objective") {
        builder = builder.thing("objective", Objective::default());
    }
    if fitted("modality") {
        builder = builder.thing("modality", ModalityUnit::default());
    }
    builder
        .thing("imaging", Imaging::default())
        .thing("jobs", Jobs::default())
        .thing("datasets", Datasets::default())
        .thing("calibration", Calibration::default())
        .thing("workflow", Workflow::default())
        .thing("mosaic_live", MosaicLive::default())
        .thing("control", Control::default())
}

/// The service: the Things' server, and the instrument behind it.
pub struct Service {
    server: ThingServer,
    instrument: Arc<Instrument>,
    webapp: webapp::WebApp,
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
        let (camera, stats, registry) = (
            Camera::default(),
            Arc::new(LatestStats::default()),
            Registry::new(),
        );
        let pool = FramePool::new(&pool, clock.clone(), Some(&registry))?;
        let acquisition = Acquisition::with_pool(devices.camera.clone(), pool, clock.clone());
        acquisition.follow_stage(devices.stage.status());
        acquisition.follow_focus(devices.focus.status());
        let sink = Arc::new(camera.streams());
        let config_previews = PreviewConfig::default();
        let previews = Previews::start(
            &acquisition,
            &config_previews,
            sink,
            stats.clone(),
            clock.clone(),
            Some(&registry),
        )?;
        let budgets = &config.memory;
        let schedule = SchedulerConfig::new(
            (budgets.job_ram_gb * 1e9) as u64,
            (budgets.job_vram_gb * 1e9) as u64,
        );
        let jobs = Arc::new(Scheduler::new(schedule, Some(&registry)).map_err(anyhow::Error::msg)?);
        let catalog = Arc::new(Catalog::scan(&config.server.data_root)?);
        let tiles = Arc::new(TileService::new(budgets.tile_cache_mb as usize * 1_000_000));
        let calibrations = Arc::new(CalibrationRegistry::open(&config.server.settings)?);
        let shading = Arc::new(ShadingRegistry::open(&config.server.settings)?);
        let auth = Arc::new(Auth::open(&config, clock.clone())?);
        let instrument = Arc::new(Instrument {
            config,
            camera: devices.camera,
            acquisition,
            previews,
            stats,
            stage: devices.stage,
            focus: devices.focus,
            peripherals: devices.peripherals.clone(),
            state: devices.state,
            registry,
            jobs,
            catalog,
            tiles,
            calibrations,
            shading,
            clock,
            auth,
            gpu: OnceLock::new(),
            auto_stitch: AtomicBool::new(true),
            hooks: RwLock::default(),
            slot_bytes: full,
            started: Instant::now(),
        });
        let fitted = |name: &str| {
            let fitted = &devices.peripherals;
            match name {
                "illumination" => fitted.illumination.is_some(),
                "filter_wheel" => fitted.filter_wheel.is_some(),
                "objective" => fitted.turret.is_some(),
                _ => fitted.modality.is_some(),
            }
        };
        calibration::stamp(&instrument);
        instrument.on_dataset_complete(Arc::new(stitch::AutoStitch));
        // The jobs' lifecycles go into the audit trail (ADR-0067); the token records its own.
        audit::record_jobs(&instrument.jobs, Arc::clone(&instrument.auth.audit));
        let server = things(camera, fitted).service(instrument.clone()).build()?;
        let webapp = webapp::WebApp::Embedded;
        Ok(Self {
            server,
            instrument,
            webapp,
        })
    }

    /// The instrument.
    pub fn instrument(&self) -> &Arc<Instrument> {
        &self.instrument
    }

    /// Serves the web app from `source` instead of the build compiled in.
    pub fn with_webapp(mut self, source: webapp::WebApp) -> Self {
        self.webapp = source;
        self
    }

    /// The Things' server and the instrument, apart: for an in-process test client.
    pub fn into_parts(self) -> (ThingServer, Arc<Instrument>) {
        (self.server, self.instrument)
    }

    /// Every route: SpectrumIQ's and the web app's first, and teta-wot's for any request they
    /// don't match; all behind the session layer (ADR-0065).
    pub fn router(&self) -> Router {
        let auth = Arc::clone(&self.instrument.auth);
        routes(&self.instrument)
            .merge(auth::routes(&auth))
            .merge(audit::routes(&auth.audit))
            .merge(data::routes(&self.instrument))
            .merge(webapp::routes(self.webapp.clone()))
            .fallback_service(self.server.router())
            .layer(middleware::from_fn_with_state(auth, auth::layer))
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
        let Self {
            server, instrument, ..
        } = self;
        let (runtime, grace) = (Arc::clone(server.runtime()), server.shutdown_grace());
        drop(server);
        runtime
            .start()
            .await
            .map_err(|failure| anyhow::anyhow!("{failure}"))?;
        info!("serving on http://{}", listener.local_addr()?);

        let (stop, mut stopped) = tokio::sync::watch::channel(false);
        // Each request knows its client's address: the login's limit counts by it.
        let router = router.into_make_service_with_connect_info::<SocketAddr>();
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
    let route = |answer: fn(&Instrument) -> Value| {
        let instrument = Arc::clone(instrument);
        get(move || {
            let instrument = Arc::clone(&instrument);
            async move { Json(answer(&instrument)) }
        })
    };
    Router::new()
        .route("/api/siq/system/health", route(Instrument::health))
        .route("/api/siq/system/metrics", route(Instrument::metrics))
        .route("/api/siq/system/memory", route(Instrument::memory))
}

/// The Things' OpenAPI document, with every peripheral. It needs no devices.
pub async fn openapi() -> anyhow::Result<Value> {
    let server = things(Camera::default(), |_| true).build()?;
    let request = Request::get("/openapi.json").body(Body::empty())?;
    let Ok(response) = server.router().oneshot(request).await;
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await?;
    serde_json::from_slice(&body).context("the OpenAPI document")
}
