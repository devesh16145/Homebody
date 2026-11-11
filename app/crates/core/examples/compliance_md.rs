//! Prints the compliance tables for docs/compliance.md:  cargo run -p homebody-core --example compliance_md
fn main() {
    print!("{}", homebody_core::compliance::markdown());
}
