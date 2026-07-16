use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=web/dist/assets");
    println!("cargo:rerun-if-changed=web/dist/index.html");

    let assets_dir = Path::new("web/dist/assets");
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let mut js_file = String::new();
    let mut css_file = String::new();

    if let Ok(entries) = fs::read_dir(assets_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("index-") && name.ends_with(".js") {
                js_file = name;
            } else if name.starts_with("index-") && name.ends_with(".css") {
                css_file = name;
            }
        }
    }

    let generated = format!(
        "#[allow(dead_code)]\npub const JS_FILENAME: &str = {:?};\n#[allow(dead_code)]\npub const CSS_FILENAME: &str = {:?};\npub const JS_CONTENT: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/web/dist/assets/{}\"));\npub const CSS_CONTENT: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/web/dist/assets/{}\"));\n",
        js_file, css_file, js_file, css_file
    );

    fs::write(Path::new(&out_dir).join("web_assets.rs"), generated).unwrap();
}
