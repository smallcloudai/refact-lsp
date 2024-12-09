use base64::{engine::general_purpose, Engine as _};
use std::env;
use std::fs::{self, read_dir};

fn main() -> shadow_rs::SdResult<()> {
    let assets_dir = "assets/integrations";
    let out_dir = env::var("OUT_DIR").unwrap();

    for entry in read_dir(assets_dir).expect("Failed to read assets directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if path.extension().and_then(|ext| ext.to_str()) == Some("png") {
            let image_data = fs::read(&path).expect("Failed to read image file");
            let base64_image = general_purpose::STANDARD.encode(image_data);
            let file_stem = path.file_stem().and_then(|stem| stem.to_str()).expect("Failed to get file stem");

            let constant_name = format!("{}_ICON_BASE64", file_stem.to_uppercase());
            let output_file = format!("{}/{}_icon.rs", out_dir, file_stem);
            fs::write(
                output_file,
                format!(
                    "pub const {}: &str = r#\"{}\"#;",
                    constant_name, base64_image
                ),
            )
                .expect("Failed to write base64 image data");
        }
    }

    shadow_rs::new()
}
