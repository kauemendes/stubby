// crates/dummy-frontend/build.rs
fn main() {
    println!("cargo:rerun-if-changed=templates");
}
