//! Rebuilds the server when the web app's build changes, so that a release build embeds the
//! current one (`src/webapp.rs`). While `web/dist` doesn't exist, this runs on every build.

fn main() {
    println!("cargo::rerun-if-changed=../../web/dist");
}
