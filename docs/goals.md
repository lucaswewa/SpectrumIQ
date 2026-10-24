I want to build a professional scientific microscope instrument software for lab applications.

The following are the requirements:

* The microscope has a large sensor camera, about 100MP 14-bit monochrome. It uses a Euresys CoaXPress framegrabber. It's native FPS is about 9 fps.
* the micscope has a motorized stage XYZ.
* the microscope should be capable of stitching images so that the resulting image can be as large as 500MP.

* primary goal: developing a scientific microscope application with high-resolution sensors (100MP, 14-bit monochrome) and real-time processing.
* Use Rust and Tauri for the desktop application; Also, a web based application is required.
* The application should be split into two distinct domains:
  - The Rust Core (The Engine): handles hardware SDK communication, framegrabber buffer management, raw image processing, stitching, and 3D reconstruction.
  - The Tauri Frontend (The View): handles user controls, parameter tuning, workflow orchestration, UI rendering, and deep-zoom visualization.
* Key Tech and Architectural decisions
  - Framegrabber SDK: Euresys CoaXPress SDK
  - Memory Management & Ring Buffers: implement a pre-allocated ring buffer (circular buffer) in Rust. The framegrabber writes directly into pinned/pre-allocated memory blocks, eliminating runtime allocations.
  - Motorized stage control: Use standard serial communication crates (tokio-serial or serialport-rs) in Rust to send G-code or proprietary ASCII commands asynchronously over tokio channels.
* Solve the Tauri IPC bottle neck (Crucial)
  - The Problem: Tauri's standard event system serialize data to JSON or string payloads. Pushing a 200MB raw frame over Tauri IPC every frame will bottleneck the application instantly.
  - The solution: Dual-Stream Strategy:
    1. Live View (Preview): In Rust, downsample and compress the 14-bit raw frame into a lightweight WebP/JPEG or H.264 (or render via a shared WebGPU context) and send lightweight compressed preview frames to the UI at 30-60 FPS.
    2. Raw Data (analysis/Stitching/Saving): Keep the pristine 14-bit raw data strictly in Rust memory or flush it directly to a high-speed NVMe scratch disk (using memory-mapped files or async file I/O via tokio).
* Image Processing, Live Stitching, & Deep Zoom
  - Processing pipeline (Rust): implement or wrap 3D reconstruction algorithms (e.g., Depth-from-focus, Optical Sectioning/Deconvolution) in Rust.
  - Rendering: For visualizing 3D volumes or meshes in the UI. Render inside a native window overlay or WebGPU canvas in the Tauri frontend using WebGPU/Three.js/Babylon.js.
* The frame rate of the camera is 9 fps for the 100MP image
  - Euresys CoaXPress SDK C/C++ API
  - a thin safe-Rust wrapper around the Euresys C API using `bindgen`
  - Configure the framegrabber to deliver buffers via callback functions or a locked multi-buffer queue.
  - Allocate a pre-sized Ring Buffer in Rust to accept incoming frames without triggering heap memory allocations, preventing latency spikes.
* Dual-Mode stitching architecture
  - Phase 1: real-time live stitching (stage-calibration based). As the stage moves, match each incoming frame's timestamp/index with the real-time XYZ coordinates reported by the motorized stage controller. Use a fast affine transformation pipeline backed by `wgpu` (CPU compute shaders). Because we rely on calibration metrics (pixel-to-micron scale, rotation, shear), the transformation matrix is deterministric. The GPU projects and blends the incoming frame onto a massive global canvas texture in real time, feeding a downscaled version back to the Tauri frontend via a low-bandwidth stream.
  - Phase 2: Post-Acquisition stitching (Feature/Correlation based + Heuristics). Use `Rayon` for multi-threaded CPU parallel processing combined with GPU acceleration. Use the stage coordinates as initial heuristics (rough positions) to narrow down search windows, then apply Phase Correlation or SIFT/SURF feature matching in overlapping regions to compute sub-pixel fine alignment. Global optimization (Bundle Adjustment / Pose Graph Optimization) is run to eliminate accumulated drift.
* Advanced 3D Processing Pipeline (Depth-from-Focus, Light Field, Confocal)
  - Since I want to executing all three intensive 3D methodologies, segregate these into a dedicated background worker pool in Rust using tokio and rayon
  - Depth-from-Focus (DFF): process focal stacks by applying Laplacian of Gaussian (LoG) filters across the image sequence to locate maximum sharpness per pixel. Implement this via parallelized matrix kernels in Rust or custom compute shaders.
  - Light Field & Confocal/Volume Rendering: Compute sub-aperture views and volumetric point clouds. Generate multi-resolution isosurfaces or ray-casting volume textures.
* Handling the 500MP + Deep Zoom & 3D Visualization in Tauri
  - 2D Deep Zoom Visualization: the Rust backend stitches the final image, slices the output directly into a Pyramidal Tile Structure (DZI format) on disk. Use OpenSeadragon or a custom WebGPU-accelerated tile renderer in the frontend to fetch only the requested tiles at the current zoom factor.
  - 3D Volume/Mesh Rendering: render the 3D reconstructed output using a WebGPU viewport inside the frontend framework (integrated via Three.js/Babylon.js or native WebGPU pipelines). For extremely heavy point clouds or ray-cast volume rendering where webviews might hit performance ceilings, spawn a native secondary window directly from Rust using winit + wgpu.
* A Dual-frontend architecture: the application runs natively via Tauri on the local desktop, and opens in a web browser locally or remotely. I want to ahve a decoupled client-server core architecture.
* Unified frontend codebase
  - frontend UI uses Vue bundled with Vite.
* Embed a high-performance asynchronous web framework inside the Rust backend, using the `teta-wot`(c:\repos\teta-wot) framework.
* Use `microscope`(c:\repos\microscope) UI and theme (look and feel) as reference frontend design.
* Solving the 1.8GB/s Data Stream Remotely: streaming raw 100MP 14-bit (1.8GB/s) to a remote browser over a standard network is impossible without massive bandwidth. I want to implement a Multi-Tier Streaming Strategy:
  - For the Local desktop (Tauri): utlize local zero-copy file sharing or high-speed local memory references to load deep-zoom tiles or live views.
  - For remote web browsers:
    + Live Preview: Rust must dynamically downscale the 100MP frames to 1080p, encode them into a compressed stream (e.g., WebRTC, MJPEG over HTTP, or H.264 via hardware-accelerated encoding using NVENC/QuickSync), and push them over WebSockets.
    + Deep Zoom / Stiching Inspection: the browser requests specific pyramidal tiles (DZI format) via HTTP from the Rust server on demand as the user pans and zooms.
* Handling state and multi-client concurrency: I want to have a concurrency model in Rust.
  - State Manager
  - Exclusive Control Lock: implement an "active operator" lock mechanism. The remote browser might have View-Only permissions by default, while the local Tauri desktop (or an authenticated remote user) holds the Hardware Control Token to prevent conflicting stage movements.
* Use VTK in the Tauri desktop architecture: use VTK.js inside the Vue UI to render the deep-zoom tiles and 3D volumes locally with zero overhead
* Use VTK in the remote web frontend: use VTK.js to render 3D volumes/surfaces dynamically on the client side, while letting the Rust backend handle tile-serving and data slicing over HTTP/WebSockets.
* Windows is the only support platform for now.
* I want to make sure the camera and stage should have simulation respectively to support development without connection to the hardware.
* I want to have simulated sample images and raw data to support algorithm develop for all the features and workflow.
* I want to make an implementation plan, which:
  - split the progress into milestones, which contains phases. Each phases should contain <1000 lines code, excluding tests and documents.
  - a professional software architecture
  - all phases should have implementation notes, ADRs, readme.md, tests, examples