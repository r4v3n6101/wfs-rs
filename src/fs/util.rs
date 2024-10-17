use std::io::{self, Cursor, Seek, Write};

use goldsrc_rs::{
    texture::{Font, Index, MipTexture, Picture, Rgb},
    wad::{ContentType, Entry},
};
use image::ImageFormat;

const DEFAULT_IMAGE_FMT: &str = "tga";

pub fn mip_level_name(level: u8) -> String {
    format!("mip_{}.{}", level, DEFAULT_IMAGE_FMT)
}

pub fn pic_name(name: impl AsRef<str>) -> String {
    format!("{}.{}", name.as_ref(), DEFAULT_IMAGE_FMT)
}

pub fn parse_wad_data(entry: &Entry, level: u8) -> io::Result<Vec<u8>> {
    match entry.ty {
        ContentType::MipTexture => {
            let MipTexture {
                width,
                height,
                data,
                ..
            } = goldsrc_rs::miptex(entry.reader())?;
            let mut buf = Cursor::new(vec![]);
            if let Some(data) = data {
                pic2img(
                    width >> level,
                    height >> level,
                    &data.indices[level as usize],
                    &data.palette,
                    &mut buf,
                )?;
            }

            Ok(buf.into_inner())
        }
        ContentType::Picture => {
            let Picture {
                width,
                height,
                data,
            } = goldsrc_rs::pic(entry.reader())?;
            let mut buf = Cursor::new(vec![]);
            pic2img(width, height, &data.indices[0], &data.palette, &mut buf)?;
            Ok(buf.into_inner())
        }
        ContentType::Font => {
            let Font {
                width,
                height,
                data,
                ..
            } = goldsrc_rs::font(entry.reader())?;
            let mut buf = Cursor::new(vec![]);
            pic2img(width, height, &data.indices[0], &data.palette, &mut buf)?;
            Ok(buf.into_inner())
        }
        ContentType::Other(_) => {
            // As is
            todo!()
        }
        _ => unimplemented!(),
    }
}

fn pic2img<W: Write + Seek>(
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
