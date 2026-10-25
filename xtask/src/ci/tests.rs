use super::sdk_files;

#[test]
fn the_sdks_files_are_found_and_the_shims_are_not() {
    let paths = [
        "crates/siq-euresys-sys/shim/siq_egrabber.h",
        "crates/siq-euresys-sys/shim/siq_egrabber.cpp",
        "crates/siq-euresys-sys/src/bindings.rs",
        "vendor/include/EGrabber.h",
        r"vendor\include\internal\generated\cEGrabber.generated.h",
        "vendor/include/GenTL_v1_6.h",
        "vendor/include/internal/class.EGrabber.h",
        "tools/playlink.cti",
        "bin/egrabber.dll",
        "docs/adr/0004-euresys-via-cpp-shim-and-bindgen.md",
    ];
    assert_eq!(
        sdk_files(paths),
        [
            "vendor/include/EGrabber.h",
            r"vendor\include\internal\generated\cEGrabber.generated.h",
            "vendor/include/GenTL_v1_6.h",
            "vendor/include/internal/class.EGrabber.h",
            "tools/playlink.cti",
            "bin/egrabber.dll",
        ]
    );
}
