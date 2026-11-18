// Gives CrewConvo.exe its icon and file properties. Does nothing on other systems.

#[cfg(windows)]
include!("src/icon.rs");

/// A Windows .ico file with one uncompressed 32-bit image per size.
#[cfg(windows)]
fn ico(sizes: &[u32]) -> Vec<u8> {
    let mut images = Vec::new();
    for &size in sizes {
        let rgba = icon_rgba(size);
        let mask_row = size.div_ceil(32) * 4;
        let mut img = Vec::new();
        img.extend_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
        img.extend_from_slice(&(size as i32).to_le_bytes());
        img.extend_from_slice(&(size as i32 * 2).to_le_bytes()); // image + mask
        img.extend_from_slice(&1u16.to_le_bytes());
        img.extend_from_slice(&32u16.to_le_bytes());
        img.extend_from_slice(&[0u8; 24]);
        for y in (0..size).rev() {
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
                img.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
            }
        }
        img.extend(std::iter::repeat_n(0u8, (mask_row * size) as usize));
        images.push((size, img));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, img) in &images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for (_, img) in images {
        out.extend(img);
    }
    out
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/icon.rs");
    println!("cargo:rerun-if-changed=../../web/dist");
    #[cfg(windows)]
    {
        let path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("crewconvo.ico");
        std::fs::write(&path, ico(&[16, 24, 32, 48, 64, 256])).expect("write icon");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(path.to_str().expect("icon path"));
        res.set("ProductName", "CrewConvo");
        res.set("FileDescription", "CrewConvo");
        if let Err(e) = res.compile() {
            // Without the Windows SDK's resource compiler the app still builds, just without an icon.
            println!("cargo:warning=could not embed the icon: {e}");
        }
    }
}
