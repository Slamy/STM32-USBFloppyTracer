extern crate bindgen;

use std::env;
use std::path::PathBuf;

fn main() {
    // build libcapsimage via cmake

    let windows_target = env::var("CARGO_CFG_TARGET_OS").unwrap().contains("windows");

    let mut capsimage_cmake = cmake::Config::new("../extern/capsimage/");

    if windows_target {
        // capsimage can't be build with msvc.
        // Either GNU or clang will do fine though
        capsimage_cmake
            .target("windows-gnu")
            .define("CMAKE_BUILD_TYPE", "Release");
    }

    let capsimage_build_path = capsimage_cmake.define("BUILD_SHARED_LIBS", "OFF").build();

    let capsimage_include_path = capsimage_build_path.join("include");
    let capsimage_lib_path = capsimage_build_path.join("lib");

    println!(
        "cargo:rustc-link-search=native={}",
        capsimage_lib_path.display()
    );

    let libname = if windows_target {
        "CAPSImg"
    } else {
        "capsimage"
    };
    // link capsimage to this project
    println!("cargo:rustc-link-lib=static={libname}");

    // Tell cargo to invalidate the built crate whenever the wrapper changes
    println!("cargo:rerun-if-changed=wrapper.h");

    if !windows_target {
        // libstdc++ must be linked to this project to make the static library of capsimage work
        println!("cargo:rustc-link-lib=stdc++");
    }
    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate
        // bindings for.
        .header("wrapper.h")
        .clang_arg(format!("-I{}", capsimage_include_path.display()))
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
