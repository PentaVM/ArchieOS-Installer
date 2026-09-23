//! Finds NeMo-Speech.cpp's libraries when the `nemo` feature is on.
//!
//! The SDK is not vendored: it is built and installed separately (see the crate docs), and
//! `NEMO_SPEECH_PREFIX` names where. The libraries are found at run time through the same
//! path, so a binary built here runs without setting a library path.

fn main() {
    println!("cargo:rerun-if-env-changed=NEMO_SPEECH_PREFIX");
    if std::env::var_os("CARGO_FEATURE_NEMO").is_none() {
        return;
    }
    let Some(prefix) = std::env::var_os("NEMO_SPEECH_PREFIX") else {
        panic!(
            "the nemo feature links NeMo-Speech.cpp: set NEMO_SPEECH_PREFIX to where its SDK \
             is installed (the directory holding lib/ and include/)"
        );
    };
    let lib = std::path::Path::new(&prefix).join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
}
