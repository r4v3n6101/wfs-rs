use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    io::{self, Cursor, Read, Seek},
    sync::RwLock,
    time::{Duration, SystemTime},
};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use goldsrc_rs::{
    texture::{Font, MipTexture, Picture, MIP_LEVELS},
    wad::ContentType,
};
use libc::{EIO, ENOENT};
use rayon::iter::{IntoParallelIterator, ParallelIterator};

mod util;

const DEFAULT_ATTR_TTL: Duration = Duration::from_secs(60);

type Ino = u64;

#[derive(Debug, Default)]
struct INode {
    /// Name of inode
    name: Cow<'static, OsStr>,
    /// Parent inode if present (root has none)
    parent: Option<Ino>,
    data: Option<Vec<u8>>,
}

impl INode {
    fn file_type(&self) -> FileType {
        match self.data {
            Some(_) => FileType::RegularFile,
            None => FileType::Directory,
        }
    }

    fn size(&self) -> u64 {
        self.data.as_ref().map(|x| x.len()).unwrap_or(0) as u64
    }

    fn resolve_file_attr(&self, ino: Ino) -> FileAttr {
        FileAttr {
            ino,
            size: self.size(),
            blocks: 0,
            atime: SystemTime::UNIX_EPOCH,
            mtime: SystemTime::UNIX_EPOCH,
            ctime: SystemTime::UNIX_EPOCH,
            crtime: SystemTime::UNIX_EPOCH,
            kind: self.file_type(),
            perm: 0o755,
            nlink: 1,
            uid: 1000,
            gid: 1000,
            rdev: 0,
            blksize: 0,
            flags: 0,
        }
    }
}

#[derive(Debug)]
struct INodes {
    inner: RwLock<Vec<INode>>,
}

impl INodes {
    fn empty() -> Self {
        Self {
            inner: RwLock::new(vec![INode::default()]),
        }
    }

    fn push_inode(&self, inode: INode) -> Ino {
        let idx = self.inner.read().unwrap().len() as Ino;
        self.inner.write().unwrap().push(inode);
        idx
    }
}

#[derive(Debug)]
pub struct WadFS {
    ttl_attr: Duration,
    inodes: INodes,

    pics_ino: Ino,
    miptexs_ino: Ino,
    fonts_ino: Ino,
    other_ino: Ino,
}

impl WadFS {
    pub fn new() -> Self {
        let inodes = INodes::empty();
        let root_ino = inodes.push_inode(INode {
            name: OsStr::new(".").into(),
            ..Default::default()
        });

        Self {
            pics_ino: inodes.push_inode(INode {
                name: OsStr::new("pics").into(),
                parent: Some(root_ino),
                ..Default::default()
            }),
            miptexs_ino: inodes.push_inode(INode {
                name: OsStr::new("miptexs").into(),
                parent: Some(root_ino),
                ..Default::default()
            }),
            fonts_ino: inodes.push_inode(INode {
                name: OsStr::new("fonts").into(),
                parent: Some(root_ino),
                ..Default::default()
            }),
            other_ino: inodes.push_inode(INode {
                name: OsStr::new("other").into(),
                parent: Some(root_ino),
                ..Default::default()
            }),

            ttl_attr: DEFAULT_ATTR_TTL,
            inodes,
        }
    }

    pub fn append_entries<R: Read + Seek + Send + Sync + 'static>(
        &mut self,
        reader: R,
    ) -> io::Result<()> {
        let Self {
            inodes,
            pics_ino,
            miptexs_ino,
            fonts_ino,
            other_ino,
            ..
        } = self;

        goldsrc_rs::wad_entries(reader, true)?
            .into_par_iter()
            .for_each(|(name, entry)| match entry.ty {
                ContentType::Picture => match goldsrc_rs::pic(entry.reader()) {
                    Ok(Picture {
                        width,
                        height,
                        data,
                    }) => {
                        let mut buf = Cursor::new(vec![]);
                        if let Err(err) = util::pic2img(
                            width,
                            height,
                            &data.indices[0],
                            &data.palette,
                            &mut buf,
                        ) {
                            tracing::warn!(%err, %name, ?entry, "couldn't convert wad entry to image");
                        }

                        inodes.push_inode(INode {
                            name: OsString::from(util::pic_name(name)).into(),
                            parent: Some(*pics_ino),
                            data: Some(buf.into_inner()),
                        });
                    }
                    Err(err) => {
                        tracing::warn!(%err, %name, ?entry, "couldn't read wad picture entry");
                    }
                }
                ContentType::MipTexture => {
                    let miptex_ino = inodes.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(*miptexs_ino),
                        ..Default::default()
                    });

                    match goldsrc_rs::miptex(entry.reader()) {
                        Ok(MipTexture {
                            width,
                            height,
                            data,
                            ..
                        }) => {
                            if let Some(data) = &data {
                                for i in 0..MIP_LEVELS {
                                    let mut buf = Cursor::new(vec![]);
                                    if let Err(err) = util::pic2img(
                                        width >> i,
                                        height >> i,
                                        &data.indices[i],
                                        &data.palette,
                                        &mut buf,
                                    ) {
                                        tracing::warn!(%err, %name, ?entry, "couldn't convert wad entry to image");
                                    }

                                    inodes.push_inode(INode {
                                        name: OsString::from(util::mip_level_name(i)).into(),
                                        parent: Some(miptex_ino),
                                        data: Some(buf.into_inner()),
                                    });
                                }
                            }
                        }
                        Err(err) =>  {
                            tracing::warn!(%err, %name, ?entry, "couldn't read wad miptex entry");
                        }
                    }
                }
                ContentType::Font => match goldsrc_rs::font(entry.reader()) {
                    Ok(Font {
                        width,
                        height,
                        data,
                        ..
                    }) => {
                        let mut buf = Cursor::new(vec![]);
                        if let Err(err) = util::pic2img(width, height, &data.indices[0], &data.palette, &mut buf) {
                            tracing::warn!(%err, %name, ?entry, "couldn't convert wad entry to image");
                        }

                        inodes.push_inode(INode {
                            name: OsString::from(util::pic_name(name)).into(),
                            parent: Some(*fonts_ino),
                            data: Some(buf.into_inner()),
                        });
                    }
                    Err(err) => {
                        tracing::warn!(%err, %name, ?entry, "couldn't read wad font entry");
                    }
                }
                ContentType::Other(_) => {
                    let mut buf = vec![];
                    if let Err(err) = entry.reader().read_to_end(&mut buf) {
                        tracing::warn!(%err, %name, ?entry, "couldn't read wad entry");
                    }

                    inodes.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(*other_ino),
                        data: Some(buf),
                    });
                }
                _ => unimplemented!(),
            });

        Ok(())
    }
}

impl Filesystem for WadFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: Ino, name: &OsStr, reply: ReplyEntry) {
        if let Some((ino, inode)) = self
            .inodes
            .inner
            .read()
            .unwrap()
            .iter()
            .enumerate()
            .find(|(_, inode)| inode.parent == Some(parent) && inode.name == name)
        {
            reply.entry(&self.ttl_attr, &inode.resolve_file_attr(ino as Ino), 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: Ino,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        for (i, (ino, inode)) in self
            .inodes
            .inner
            .read()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, inode)| inode.parent == Some(ino))
            .enumerate()
            .skip(offset as usize)
            // FIXME: Error if removed
            .take(5)
        {
            if reply.add(ino as Ino, (i + 1) as i64, inode.file_type(), &inode.name) {
                return;
            }
        }
        reply.ok()
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: Ino, reply: ReplyAttr) {
        if let Some(inode) = self.inodes.inner.read().unwrap().get(ino as usize) {
            reply.attr(&self.ttl_attr, &inode.resolve_file_attr(ino));
        } else {
            reply.error(ENOENT);
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: Ino,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        match self.inodes.inner.read().unwrap().get(ino as usize) {
            Some(inode) => match &inode.data {
                Some(data) => {
                    let start = offset as usize;
                    let end = start + size as usize;
                    match data.get(start..end) {
                        Some(buf) => reply.data(buf),
                        None => reply.error(EIO),
                    }
                }
                None => reply.error(EIO),
            },
            None => reply.error(ENOENT),
        }
    }
}
