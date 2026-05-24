//! immich-extended binary entry point.
//!
//! Real wiring (config, tracing, axum, sqlx) lands in M0-T2 and M0-T3. The M0-T1
//! skeleton only proves the workspace builds and the binary links against every
//! library crate.

fn main() {
    println!(
        "immich-extended {} (server={}, engine={}, immich-client={}, yolo={}, common={})",
        server::version(),
        server::version(),
        engine::version(),
        immich_client::version(),
        yolo::version(),
        common::version(),
    );
}
