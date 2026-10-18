//! Builds the vendored libresidfp 1.2.3 and the C shim with `cc` (ADR-011).

use std::path::Path;

fn main() {
    let vendor = Path::new("vendor/libresidfp/src");
    let mut sources: Vec<_> = std::fs::read_dir(vendor)
        .expect("vendor/libresidfp/src")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("cpp" | "cc")))
        .collect();
    sources.push(vendor.join("resample/SincResampler.cpp"));
    sources.push(vendor.join("residfp/residfp.cpp"));
    sources.push("shim/shim.cpp".into());
    sources.sort();

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .std("c++17")
        .files(&sources)
        // generated/ first: siddefs-fp.h and residfp/sidversion.h, which configure would write.
        .include("generated")
        .include(vendor)
        .warnings(false)
        .define("NDEBUG", None);
    if !build.get_compiler().is_like_msvc() {
        // Upstream's own flags (configure.ac).
        build
            .flag("-ffast-math")
            .flag("-fno-unsafe-math-optimizations");
    } else {
        build.flag("/EHsc");
    }
    build.compile("residfp");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-changed=generated");
    println!("cargo:rerun-if-changed=vendor");
}
