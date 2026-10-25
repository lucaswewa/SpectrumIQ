//! With the `sdk` feature, compiles the shim against the installed eGrabber SDK (ADR-0004).

fn main() {
    println!("cargo::rerun-if-changed=shim");
    #[cfg(feature = "sdk")]
    sdk::compile();
}

#[cfg(feature = "sdk")]
mod sdk {
    use std::path::PathBuf;

    /// Where the SDK's installer puts it.
    const DEFAULT_SDK: &str = r"C:\Program Files\Euresys\eGrabber";

    pub fn compile() {
        println!("cargo::rerun-if-env-changed=EGRABBER_SDK_DIR");
        let sdk = std::env::var_os("EGRABBER_SDK_DIR")
            .map_or_else(|| PathBuf::from(DEFAULT_SDK), PathBuf::from);
        let include = sdk.join("include");
        if !include.join("EGrabber.h").is_file() {
            println!(
                "cargo::error=there is no eGrabber SDK at {}: install it, or set EGRABBER_SDK_DIR",
                sdk.display()
            );
            return;
        }
        // The header loads egrabber.dll at run time, through EURESYS_EGRABBER_LIB64: nothing to
        // link but the shim.
        cc::Build::new()
            .cpp(true)
            .file("shim/siq_egrabber.cpp")
            .include("shim")
            .include(&include)
            .flag("/std:c++17")
            .flag("/EHsc")
            // The SDK's headers are its own: warnings only for the shim.
            .flag("/external:anglebrackets")
            .flag("/external:W0")
            .compile("siq_egrabber");
    }
}
