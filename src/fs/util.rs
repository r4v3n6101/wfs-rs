use std::io::{self, Seek, Write};

use goldsrc_rs::texture::{Index, Rgb};
use image::ImageFormat;

const DEFAULT_IMAGE_FMT: &str = "tga";

pub fn mip_level_name(level: usize) -> String {
    format!("mip_{}.{}", level, DEFAULT_IMAGE_FMT)
}

pub fn pic_name(name: impl AsRef<str>) -> String {
    format!("{}.{}", name.as_ref(), DEFAULT_IMAGE_FMT)
}

pub fn pic2img<W: Write + Seek>(
    width: u32,
    height: u32,
    indices: &[Index],
    palette: &[Rgb],
    mut output: W,
) -> io::Result<()> {
    let data: Vec<_> = indices
        .iter()
        .flat_map(|&i| {
            let rgb_i = i as usize;
            let [r, g, b] = palette[rgb_i];
            if r == 255 || g == 255 || b == 255 {
                [0; 4]
            } else {
                [r, g, b, 255]
            }
        })
        .collect();

    image::RgbaImage::from_vec(width, height, data)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "buffer is not big enough to fulfill image",
            )
        })
        .and_then(|img| {
            img.write_to(&mut output, ImageFormat::Tga)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        })
}
