fn main() {
    let manifest =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest"));
    let icon_dir = manifest.join("icons");
    std::fs::create_dir_all(&icon_dir).expect("create icon directory");
    let pixels: Vec<u8> = (0..32 * 32)
        .flat_map(|index| {
            let x = index % 32;
            let y = index / 32;
            if (x > 7 && x < 24) || (y > 7 && y < 24) {
                [54, 95, 75, 255]
            } else {
                [231, 217, 189, 255]
            }
        })
        .collect();
    let png_path = icon_dir.join("icon.png");
    let mut png_data = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png_data, 32, 32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(&pixels).expect("png pixels");
    }
    std::fs::write(&png_path, &png_data).expect("write generated PNG icon");
    let ico_path = icon_dir.join("icon.ico");
    let image = ico::IconImage::from_rgba_data(32, 32, pixels);
    let mut directory = ico::IconDir::new(ico::ResourceType::Icon);
    directory.add_entry(ico::IconDirEntry::encode(&image).expect("encode icon"));
    let mut file = std::fs::File::create(&ico_path).expect("create icon");
    directory.write(&mut file).expect("write icon");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new().window_icon_path(ico_path)),
    )
    .expect("tauri build script failed");
}
