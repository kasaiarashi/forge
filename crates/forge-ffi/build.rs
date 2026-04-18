// Copyright (c) 2026 Krishna Teja. All rights reserved.
// Licensed under the MIT License.

//! Generate `include/forge_ffi.h` from the Rust public signatures so
//! the UE plugin can `#include <forge_ffi.h>` without us hand-writing
//! (and drifting) a parallel header.
//!
//! Deliberately best-effort: on a machine without cbindgen's
//! prerequisites the header simply isn't regenerated — the last
//! committed version still works, and a follow-up PR will notice when
//! the Rust signatures changed but the header didn't.

fn main() {
    // Only regenerate when the Rust source actually changes.
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");

    let crate_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR unset");
    let header = std::path::PathBuf::from(&crate_dir)
        .join("include")
        .join("forge_ffi.h");
    std::fs::create_dir_all(header.parent().unwrap()).ok();

    let Ok(builder) = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(cbindgen::Config::from_file(
            std::path::PathBuf::from(&crate_dir).join("cbindgen.toml"),
        ).unwrap_or_default())
        .generate()
    else {
        // Don't fail the build if cbindgen can't expand syn — the
        // header update is non-blocking. Print a notice so CI can
        // flag a regeneration request.
        println!("cargo:warning=cbindgen failed; header not regenerated");
        return;
    };
    builder.write_to_file(&header);
}
