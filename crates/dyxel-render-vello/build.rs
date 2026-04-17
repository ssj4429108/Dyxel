// Copyright 2024 Dyxel Contributors
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=src/blit.wgsl");

    let shader_source = fs::read_to_string("src/blit.wgsl").expect("Failed to read blit.wgsl");

    let mut frontend = naga::front::wgsl::Frontend::new();
    let module = frontend
        .parse(&shader_source)
        .expect("Failed to parse WGSL");

    // Use Android-compatible capabilities (limited to basic features supported by Vulkan 1.0 core)
    let capabilities = naga::valid::Capabilities::empty();

    let mut validator =
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), capabilities);
    let info = validator
        .validate(&module)
        .expect("Failed to validate shader");

    // Configure SPIR-V generation options, optimized for Android Vulkan
    let mut options = naga::back::spv::Options::default();
    // Set target version to SPIR-V 1.0 (Vulkan 1.0 compatible)
    options.lang_version = (1, 0);

    let mut writer = naga::back::spv::Writer::new(&options).expect("Failed to create SPV writer");

    let mut words = Vec::new();
    writer
        .write(&module, &info, None, &None, &mut words)
        .expect("Failed to write SPV");

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("blit.spv");
    fs::write(dest_path, bytemuck::cast_slice(&words)).expect("Failed to write blit.spv");
}
