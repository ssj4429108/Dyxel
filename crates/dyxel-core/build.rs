// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    uniffi::generate_scaffolding("src/dyxel_core.udl").expect("UniFFI scaffolding generation failed");

    // AOT Compile blit.wgsl to SPIR-V
    let wgsl_source = fs::read_to_string("src/blit.wgsl").expect("Failed to read blit.wgsl");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let spv_path = out_dir.join("blit.spv");

    // Naga 27.x API
    let mut frontend = naga::front::wgsl::Frontend::new();
    let module = frontend.parse(&wgsl_source).expect("Failed to parse WGSL");

    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    let info = validator.validate(&module).expect("Failed to validate shader");

    let mut writer = naga::back::spv::Writer::new(&naga::back::spv::Options::default()).expect("Failed to create SPV writer");
    let mut words = Vec::new();
    // naga 27.x takes 5 arguments: module, info, options, debug_info, output
    writer.write(&module, &info, None, &None, &mut words).expect("Failed to write SPV");

    let spv_bytes = bytemuck::cast_slice(&words);
    fs::write(&spv_path, spv_bytes).expect("Failed to write blit.spv");

    println!("cargo:rerun-if-changed=src/dyxel_core.udl");
    println!("cargo:rerun-if-changed=src/blit.wgsl");
}
