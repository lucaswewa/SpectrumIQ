# M14 — Streaming upgrades, local fast paths and advanced viewers

**Outcome:**

- 16-bit client-side rendering;
- one multiplexed binary stream with H.264;
- the staged local zero-copy paths for the desktop, each gated by a benchmark;
- the native winit+wgpu volume viewer.

Every phase here is an upgrade with a measurable gain. A phase whose benchmark gate fails is
recorded and closed **without** merging the feature.

---

### P14.1 — 16-bit tile renderer (WebGL2)

**Budget** ~950 · **Depends on** P4.8 · **Requirements** REQ-VIS-002

**Scope**

- **Endpoint:** `/api/siq/data/{id}/raw16/{level}/{col}_{row}`, little-endian u16 with
  `Content-Encoding: gzip`, so the browser inflates it natively.
- **WebGL2 renderer:**
  - `R16UI` textures;
  - a shader for window/level/gamma/colormap per channel;
  - additive multi-channel compositing;
  - instant contrast changes;
  - true values on hover, read from the loaded tiles.
- **Integration:** an OpenSeadragon custom drawer, or an overlay canvas synchronized with the
  viewport. The ADR decides.

**Tests**

- Shader maths against a JS reference.
- Endpoint tests.
- Fallback to 8-bit tiles when WebGL2 is unavailable.

**Example.** `/#/dev/viewer16`.

**ADR.** An amendment to ADR-0009.

---

### P14.2 — Multiplexed binary stream (`/api/siq/stream`)

**Budget** ~700 · **Depends on** P9.2 · **Requirements** REQ-PRV-001, REQ-NFR-004

**Scope**

- **One WebSocket per client** carrying the subscribed streams (overview, ROI, mosaic) as tagged
  binary JPEG frames.
- **Per-client flow control:** a new frame is sent only after the client acknowledges the previous
  one, so a slow client skips frames.
- **Frontend player:** `createImageBitmap` drawing to a canvas, behind a `StreamSource`
  abstraction that MJPEG also implements.
- **Frees HTTP connections** for tiles (architecture §7.3).

**Tests**

- Protocol tests.
- Flow control with a slow client.
- Reconnect.

**Example.** `/#/dev/stream`.

**ADR.** The binary streaming protocol.

---

### P14.3 — H.264 hardware encoding (`siq-mfenc`) and WebCodecs playback

**Budget** ~850 · **Depends on** P14.2 · **Requirements** REQ-PRV-005

**Scope**

- **Encoder:** Media Foundation H.264 through the `windows` crate. It picks the hardware MFT (NVENC
  on the RTX 3060) and is configured for low latency: no B-frames, CBR or VBR, keyframes on demand.
- **NV12 conversion** from the LUT-mapped preview.
- **Client:** WebCodecs `VideoDecoder`, with JPEG fallback negotiation.

**Tests**

- Encoder smoke test (skipped without a hardware MFT).
- Bitstream sanity.
- Fallback path.

**Example.** `mf_encode_bench`.

**ADR.** H.264 through Media Foundation, compared with the NVENC SDK and ffmpeg.

---

### P14.4 — Desktop local path v2: direct tile reads

**Budget** ~500 · **Depends on** P3.5, P4.5 · **Requirements** REQ-NET-005

**Scope**

- A Tauri custom protocol serves pre-rendered DZI tiles from the data folder, falling back to
  HTTP.
- The `HostAdapter` exposes a `tileUrl()` override.

**Gate.** Must cut median tile latency or CPU by ≥ 30 % compared with loopback HTTP.

**Example.** `tile_path_bench`.

**ADR.** Desktop direct tile reads, with the benchmark result.

---

### P14.5 — Desktop local path v3: shared-memory live frames (`siq-shm`)

**Budget** ~900 · **Depends on** P14.2, P3.5 · **Requirements** REQ-NET-005

**Scope**

- **Service side:** named shared memory (`CreateFileMappingW`), holding a ring of preview frames
  with a seqlock header.
- **Tauri side:** maps the ring, then hands each frame to the page through WebView2
  `CreateSharedBuffer` / `PostSharedBufferToScript` (`webview2-com`).
- **Page side:** draws from the ArrayBuffer.

**Gate.** Lower latency or CPU than P14.2's WebSocket path on the same machine.

**Example.** `shm_bench`.

**ADR.** Zero-copy local frames through WebView2 shared buffers.

---

### P14.6 — Native 3D viewer: bricked out-of-core volumes (`apps/spectrumiq-viewer3d`)

**Budget** ~800 · **Depends on** P12.5, P7.1 · **Requirements** REQ-VIS-004

**Scope**

- A winit window with wgpu.
- A brick loader from OME-Zarr levels, with an LRU brick cache in VRAM and a level-of-detail
  selection by screen-space error.
- Launched from the desktop through `HostAdapter.openNativeViewer(datasetId)`, and talking to the
  service over local HTTP.

**Tests**

- Brick addressing.
- Cache eviction.
- Level-of-detail selection.

**Example.** `viewer3d --dataset <id>`.

**ADR.** Native viewer architecture.

---

### P14.7 — Native 3D viewer: ray casting and controls

**Budget** ~900 · **Depends on** P14.6 · **Requirements** REQ-VIS-004

**Scope**

- **GPU ray casting** over the bricks: early termination, transfer function, and MIP/composite
  modes.
- **Camera controls.**
- **The transfer function,** shared with the web editor's JSON format.

**Tests**

- Ray-march against a CPU reference on a small volume.
- Transfer-function parsing.

**Example.** `viewer3d --demo`.

**ADR.** An amendment to the native viewer ADR.
