fn main() {
    uniffi::generate_scaffolding("src/host_core.udl").unwrap();
}
