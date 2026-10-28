fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = wat::parse_str(include_str!("../plugins/example-invert.wat"))?;
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "plugins/example-wasm-invert.wasm".into());
    std::fs::write(&output, &bytes)?;
    println!("Built {output}");
    let package = serde_json::json!({
        "format":"dudel-wasm-extension", "version":1, "api":1,
        "id":"dudel-wasm-invert", "name":"WASM invert", "author":"Dudel", "release":"1.0.0",
        "description":"Invert RGB using WASM while preserving alpha.", "capabilities":["pixels.filter"], "wasm":bytes
    });
    let package_path = std::path::Path::new(&output).with_extension("dudelplugin");
    std::fs::write(&package_path, serde_json::to_vec_pretty(&package)?)?;
    println!("Built {}", package_path.display());
    Ok(())
}
