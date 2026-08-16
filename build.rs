fn main() {
    // The catalogues are read by a macro, which cargo does not track: without
    // this an edited translation needs a touched source file to take effect.
    println!("cargo:rerun-if-changed=locales");
}
