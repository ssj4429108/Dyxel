use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    uniffi::generate_scaffolding("src/host_core.udl").expect("UniFFI scaffolding generation failed");

    // AOT Compile blit.wgsl to SPIR-V
    let wgsl_source = fs::read_to_string("src/blit.wgsl").expect("Failed to read blit.wgsl");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let spv_path = out_dir.join("blit.spv");

    // Use naga v0.20 API
    let module = naga::front::wgsl::parse_str(&wgsl_source).expect("Failed to parse WGSL");
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("Failed to validate WGSL module");

    let mut spv_out = Vec::new();
    let mut writer = naga::back::spv::Writer::new(&naga::back::spv::Options {
        lang_version: (1, 3),
        flags: naga::back::spv::WriterFlags::empty(),
        ..Default::default()
    }).expect("Failed to create SPV writer");

    // writer.write in naga v0.20 takes 5 arguments, 4th is &Option<DebugInfo>
    writer.write(&module, &info, None, &None, &mut spv_out).expect("Failed to write SPIR-V");
    
    // SPIR-V is a Vec<u32>, we need to convert it to bytes for include_bytes!
    let spv_bytes: Vec<u8> = spv_out.iter().flat_map(|&u| u.to_le_bytes().to_vec()).collect();
    fs::write(&spv_path, spv_bytes).expect("Failed to write blit.spv");

    println!("cargo:rerun-if-changed=src/blit.wgsl");
}
