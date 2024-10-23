use std::{
    ffi::OsString,
    io::{self, Cursor, Read, Seek, Write},
};

use goldsrc_rs::{
    texture::{Font, Index, MipTexture, Picture, Rgb, MIP_LEVELS},
    wad::{ContentType, Entry},
    CStr16,
};
use image::ImageFormat;

use super::{INode, Ino, WadFS, FONTS_DIR_INO, MIPTEXS_DIR_INO, OTHER_DIR_INO, PICS_DIR_INO};

const DEFAULT_IMAGE_FMT: &str = "tga";

#[inline]
fn mip_level_name(level: usize) -> String {
    format!("mip_{}.{}", level, DEFAULT_IMAGE_FMT)
}

#[inline]
fn pic_name(name: impl AsRef<str>) -> String {
    format!("{}.{}", name.as_ref(), DEFAULT_IMAGE_FMT)
}

#[tracing::instrument(err, skip_all)]
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
        .inspect(|img| {
            tracing::debug!(
                width = img.width(),
                height = img.height(),
                "allocated image"
            )
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::OutOfMemory,
                "buffer is not big enough to fulfill image",
            )
        })
        .and_then(|img| {
            img.write_to(&mut output, ImageFormat::Tga)
                .inspect(|_| tracing::debug!("written to tga"))
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
        })
}

#[tracing::instrument(skip(fs))]
pub fn create_inode(fs: &WadFS, name: CStr16, entry: Entry) {
    match entry.ty {
        ContentType::Picture => match goldsrc_rs::pic(entry.reader()) {
            Ok(Picture {
                width,
                height,
                data,
            }) => {
                let mut buf = Cursor::new(vec![]);
                if pic2img(width, height, &data.indices[0], &data.palette, &mut buf).is_ok() {
                    let buf = buf.into_inner();
                    let mut inodes = fs.inodes.write().unwrap();

                    tracing::debug!(buflen = buf.len(), ino = inodes.len(), "new inode for pic");
                    inodes.push(INode {
                        name: OsString::from(pic_name(name)).into(),
                        parent: Some(PICS_DIR_INO),
                        data: Some(buf),
                    });
                }
            }
            Err(err) => {
                tracing::warn!(%err, "couldn't read wad picture entry");
            }
        },
        ContentType::MipTexture => match goldsrc_rs::miptex(entry.reader()) {
            Ok(MipTexture {
                width,
                height,
                data,
                ..
            }) => {
                if let Some(data) = &data {
                    let miptex_ino = {
                        let mut inodes = fs.inodes.write().unwrap();
                        let ino = inodes.len() as Ino;
                        inodes.push(INode {
                            name: OsString::from(name.as_str()).into(),
                            parent: Some(MIPTEXS_DIR_INO),
                            ..Default::default()
                        });

                        ino
                    };

                    for i in 0..MIP_LEVELS {
                        let mut buf = Cursor::new(vec![]);
                        if pic2img(
                            width >> i,
                            height >> i,
                            &data.indices[i],
                            &data.palette,
                            &mut buf,
                        )
                        .is_ok()
                        {
                            let buf = buf.into_inner();
                            let mut inodes = fs.inodes.write().unwrap();

                            tracing::debug!(
                                buflen = buf.len(),
                                ino = inodes.len(),
                                miplevel = i,
                                "new inode for miptex level"
                            );
                            inodes.push(INode {
                                name: OsString::from(mip_level_name(i)).into(),
                                parent: Some(miptex_ino),
                                data: Some(buf),
                            });
                        }
                    }
                } else {
                    tracing::info!("empty miptex detected");
                }
            }
            Err(err) => {
                tracing::warn!(%err, "couldn't read wad miptex entry");
            }
        },
        ContentType::Font => match goldsrc_rs::font(entry.reader()) {
            Ok(Font {
                width,
                height,
                data,
                ..
            }) => {
                let mut buf = Cursor::new(vec![]);
                if pic2img(width, height, &data.indices[0], &data.palette, &mut buf).is_ok() {
                    let buf = buf.into_inner();
                    let mut inodes = fs.inodes.write().unwrap();

                    tracing::debug!(buflen = buf.len(), ino = inodes.len(), "new inode for font");
                    inodes.push(INode {
                        name: OsString::from(pic_name(name)).into(),
                        parent: Some(FONTS_DIR_INO),
                        data: Some(buf),
                    });
                }
            }
            Err(err) => {
                tracing::warn!(%err, "couldn't read wad font entry");
            }
        },
        ContentType::Other(_) => {
            let mut buf = vec![];
            match entry.reader().read_to_end(&mut buf) {
                Ok(_) => {
                    let mut inodes = fs.inodes.write().unwrap();

                    tracing::debug!(
                        buflen = buf.len(),
                        ino = inodes.len(),
                        "new inode for other"
                    );
                    inodes.push(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(OTHER_DIR_INO),
                        data: Some(buf),
                    });
                }
                Err(err) => {
                    tracing::warn!(%err, "couldn't read wad entry");
                }
            }
        }
        _ => unimplemented!(),
    }
}
