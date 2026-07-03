use std::env;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let crun = manifest_dir.join("vendor/crun");

    if !crun.join("config.h").exists() {
        panic!(
            "vendor/crun/config.h not found. Run:\n\
             just config"
        );
    }

    println!("cargo:rustc-link-lib=crun");
    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-env-changed=LIBCRUN_LIB_DIR");

    if let Ok(dir) = env::var("LIBCRUN_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    } else {
        println!(
            "cargo:rustc-link-search=native={}",
            crun.join(".libs").display()
        );
    }

    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg(format!("-I{}", crun.display()))                       // config.h
        .clang_arg(format!("-I{}", crun.join("src/libcrun").display()))   // libcrun headers
        .clang_arg(format!("-I{}", crun.join("libocispec/src").display()))// ocispec/... headers
        .blocklist_item("FP_NAN")
        .blocklist_item("FP_INFINITE")
        .blocklist_item("FP_ZERO")
        .blocklist_item("FP_SUBNORMAL")
        .blocklist_item("FP_NORMAL")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}