// TODO: remove

fn main() {
    println!("cargo::rustc-check-cfg=cfg(egress_enforcement)");

    let linux = std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("linux");
    let enclave = std::env::var("CARGO_CFG_FEATURE_ENCLAVE").is_ok();
    let pebble = std::env::var("CARGO_CFG_FEATURE_PEBBLE").is_ok();

    if linux && (enclave || pebble) {
        println!("cargo:rustc-cfg=egress_enforcement");
    }
}
