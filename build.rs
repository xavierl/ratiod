// Generates the Windows executable icon from the code-drawn ratiod icon (no binary file in the repo).

include!("src/icon.rs");

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    let size = 256;
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ratiod.ico");
    let image = ico::IconImage::from_rgba_data(size, size, rgba(size));
    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    dir.add_entry(ico::IconDirEntry::encode(&image).unwrap());
    dir.write(std::fs::File::create(&out).unwrap()).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon(out.to_str().unwrap());
        res.compile().unwrap();
    }
}
