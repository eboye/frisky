use std::path::PathBuf;
use std::process::Command;

fn main() {
    glib_build_tools::compile_resources(
        &["data/resources"],
        "data/resources/frisky.gresource.xml",
        "frisky.gresource",
    );

    compile_schema_for_dev_runs();
}

/// Compiles the GSettings schema into `OUT_DIR` so `cargo run` works without a
/// system-wide install.
///
/// Release builds get the schema from the Flatpak/`make install` prefix
/// instead; this only exists so the dev loop does not abort inside gio on a
/// missing schema.
fn compile_schema_for_dev_runs() {
    let schema = "data/io.github.eboye.Frisky.gschema.xml";
    println!("cargo:rerun-if-changed={schema}");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR is always set"));
    let schema_dir = out_dir.join("schemas");

    if let Err(error) = std::fs::create_dir_all(&schema_dir) {
        println!("cargo:warning=could not create schema dir: {error}");
        return;
    }
    if let Err(error) = std::fs::copy(
        schema,
        schema_dir.join("io.github.eboye.Frisky.gschema.xml"),
    ) {
        println!("cargo:warning=could not stage schema: {error}");
        return;
    }

    match Command::new("glib-compile-schemas")
        .arg(&schema_dir)
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => println!("cargo:warning=glib-compile-schemas failed: {status}"),
        Err(error) => println!("cargo:warning=glib-compile-schemas not found: {error}"),
    }

    println!("cargo:rustc-env=FRISKY_SCHEMA_DIR={}", schema_dir.display());
}
