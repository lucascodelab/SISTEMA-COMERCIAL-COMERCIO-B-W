use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
use image::imageops::FilterType;
use std::{fs::File, path::Path};

fn main() {
  let _ = std::fs::create_dir_all("icons");
  let source = image::open("../src/assets/logo-oficial.png").expect("logo oficial ausente");
  let mut directory = IconDir::new(ResourceType::Icon);
  for size in [16, 32, 48, 64, 128, 256] {
    let square = source.resize_exact(size, size, FilterType::Lanczos3).to_rgba8();
    let image = IconImage::from_rgba_data(size, size, square.into_raw());
    directory.add_entry(IconDirEntry::encode(&image).expect("falha ao criar ícone"));
  }
  directory.write(File::create(Path::new("icons/icon.ico")).expect("falha ao gravar ícone")).expect("falha ao finalizar ícone");
  tauri_build::build()
}
