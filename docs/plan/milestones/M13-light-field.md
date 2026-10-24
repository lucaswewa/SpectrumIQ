# M13 — 3D: light field

**Outcome:** a light-field data model and MLA calibration, with sub-aperture views, synthetic
refocusing, depth maps and point clouds. Conventional LFM comes first, with Fourier LFM behind the
same trait (ADR-0014: the geometry is not yet decided).

**Exit criteria:**

- on synthetic light fields from `siq-datagen`, MLA calibration recovers the pitch within 0.1 px
  and the rotation within 0.05°;
- refocused stacks give depth RMSE within the target set in P13.2;
- the UI scrubs the views and the refocus interactively.

---

### P13.1 — Light-field model, MLA calibration and synthetic light fields (`siq-lightfield`)

**Budget** ~900 · **Depends on** P1.4, P4.9, P5.1 · **Requirements** REQ-3D-004, REQ-SIM-004

**Scope**

- **`LfGeometry` trait,** with `Conventional` and `Fourier` implementations.
- **MLA calibration from a white image:**
  - FFT peaks give the pitch and rotation;
  - a centroid per lenslet refines the centre grid;
  - vignetting is modelled per lenslet;
  - the result is a calibration record in the registry.
- **`siq-datagen lightfield`:** ray-optics rendering of 3D bead or height worlds through an ideal
  MLA, for both geometries.

**Tests**

- Calibration recovery.
- Rotation and offset robustness.
- Determinism of the synthetic renders.

**Example.** `lf_calibrate`.

**ADR.** The light-field geometry abstraction and its calibration.

---

### P13.2 — Sub-aperture views, refocusing and depth

**Budget** ~900 · **Depends on** P13.1, P11.1 · **Requirements** REQ-3D-004

**Scope**

- **Conventional LFM:**
  - rectify the lenslet grid;
  - extract the (u, v) sub-aperture views by resampling;
  - shift-and-sum refocus over α, giving a focal stack.
- **Fourier LFM:** extract the elemental images and refocus by disparity shifts.
- **Depth:** DFF (`siq-dff`) on the refocused stack, then a point cloud `(x, y, z, intensity)`.

**Tests**

- Ground-truth depths.
- View geometry.
- Refocus at known planes.

**Example.** `lf_refocus`.

**ADR.** An amendment to the light-field ADR covering the refocus and depth methods.

---

### P13.3 — Light-field workflow, outputs and UI

**Budget** ~900 · **Depends on** P13.2, P11.4 · **Requirements** REQ-3D-004, REQ-VIS-003

**Scope**

- **Acquisition mode:** `ModalityUnit` with the MLA in, and the right exposure presets.
- **Job outputs:** views as a Zarr array `(u, v, y, x)`, the focal stack, depth, and points.
- **Endpoints:**
  - the `view(u, v)` image;
  - `refocus(α)`, rendered on the server as JPEG;
  - the binary point cloud.
- **UI:**
  - a (u, v) scrubber pad;
  - a refocus slider;
  - the depth map;
  - the point cloud in VTK.js.

**Tests**

- Endpoint HTTP tests.
- UI stores.
- An integration run on simulated light-field data.

**Example.** `/#/dev/lightfield`.

**ADR.** Light-field UI and data transport.

---

### P13.4 — (Stretch) 3D light-field deconvolution

**Budget** ~950 · **Depends on** P13.3, P12.3 · **Requirements** REQ-3D-004

**Scope**

- A forward model, ray-optics first and wave-optics as an option.
- Richardson–Lucy on the GPU.
- Validation against ground truth on synthetic volumes.

**Tests.** Ground-truth volume recovery metrics.

**Example.** `lf_deconv`.

**ADR.** The light-field 3D reconstruction method.
