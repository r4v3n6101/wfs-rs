use std::{
    borrow::Cow,
    ffi::{OsStr, OsString},
    io::{self, Read, Seek},
    time::{Duration, SystemTime},
};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use goldsrc_rs::{
    texture::MIP_LEVELS,
    wad::{ContentType, Entry},
};
use libc::{EIO, ENOENT};

mod util;

const DEFAULT_ATTR_TTL: Duration = Duration::from_secs(60);

type Ino = u64;

#[derive(Debug)]
struct Data {
    entry: Entry,
    // For mipmaps only
    level: u8,
    cache: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct INode {
    /// Name of inode
    name: Cow<'static, OsStr>,
    /// Parent inode if present (root has none)
    parent: Option<Ino>,
    /// For mipmaps
    data: Option<Data>,
}

impl INode {
    fn file_type(&self) -> FileType {
        match self.data {
            Some(_) => FileType::RegularFile,
            None => FileType::Directory,
        }
    }

    fn resolve_data(&mut self) -> Option<&mut [u8]> {
        self.data.as_mut().map(
            |Data {
                 entry,
                 level,
                 cache,
             }| {
                cache
                    .get_or_insert_with(|| match util::parse_wad_data(entry, *level) {
                        Ok(buf) => buf,
                        Err(err) => {
                            tracing::warn!(%err, "invalid entry content");
                            vec![]
                        }
                    })
                    .as_mut()
            },
        )
    }

    fn resolve_file_attr(&mut self, ino: Ino) -> FileAttr {
        let data = self.resolve_data();
        FileAttr {
            ino,
            size: data.map(|x| x.len()).unwrap_or(0) as Ino,
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
    inner: Vec<INode>,
}

impl INodes {
    fn empty() -> Self {
        Self {
            inner: vec![INode::default()],
        }
    }

    fn push_inode(&mut self, inode: INode) -> Ino {
        let idx = self.inner.len() as Ino;
        self.inner.push(inode);
        idx
    }
}

#[derive(Debug)]
pub struct WadFS {
    ttl_attr: Duration,
    inodes: INodes,

    root_ino: Ino,
    pics_ino: Option<Ino>,
    miptexs_ino: Option<Ino>,
    fonts_ino: Option<Ino>,
    other_ino: Option<Ino>,
}

impl WadFS {
    pub fn new() -> Self {
        let mut inodes = INodes::empty();
        let root_ino = inodes.push_inode(INode {
            name: OsStr::new(".").into(),
            ..Default::default()
        });

        Self {
            inodes,
            root_ino,

            ttl_attr: DEFAULT_ATTR_TTL,
            pics_ino: None,
            miptexs_ino: None,
            fonts_ino: None,
            other_ino: None,
        }
    }

    pub fn append_entries<R: Read + Seek + Send + Sync + 'static>(
        &mut self,
        reader: R,
    ) -> io::Result<()> {
        let Self {
            inodes,
            root_ino,
            pics_ino,
            miptexs_ino,
            fonts_ino,
            other_ino,
            ..
        } = self;

        for (name, entry) in goldsrc_rs::wad_entries(reader, true)? {
            match entry.ty {
                ContentType::Picture => {
                    let pics_ino = pics_ino.get_or_insert_with(|| {
                        inodes.push_inode(INode {
                            name: OsStr::new("pics").into(),
                            parent: Some(*root_ino),
                            ..Default::default()
                        })
                    });
                    inodes.push_inode(INode {
                        name: OsString::from(util::pic_name(name)).into(),
                        parent: Some(*pics_ino),
                        data: Some(Data {
                            entry,
                            level: 1,
                            cache: None,
                        }),
                    });
                }
                ContentType::MipTexture => {
                    let miptexs_ino = miptexs_ino.get_or_insert_with(|| {
                        inodes.push_inode(INode {
                            name: OsStr::new("miptexs").into(),
                            parent: Some(*root_ino),
                            ..Default::default()
                        })
                    });
                    let miptex_ino = inodes.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(*miptexs_ino),
                        ..Default::default()
                    });
                    for i in 0..MIP_LEVELS {
                        let i = i as u8;

                        let entry = entry.clone();
                        inodes.push_inode(INode {
                            name: OsString::from(util::mip_level_name(i)).into(),
                            parent: Some(miptex_ino),
                            data: Some(Data {
                                entry,
                                level: i,
                                cache: None,
                            }),
                        });
                    }
                }
                ContentType::Font => {
                    let fonts_ino = fonts_ino.get_or_insert_with(|| {
                        inodes.push_inode(INode {
                            name: OsStr::new("fonts").into(),
                            parent: Some(*root_ino),
                            ..Default::default()
                        })
                    });
                    inodes.push_inode(INode {
                        name: OsString::from(util::pic_name(name)).into(),
                        parent: Some(*fonts_ino),
                        data: Some(Data {
                            entry,
                            level: 0,
                            cache: None,
                        }),
                    });
                }
                ContentType::Other(_) => {
                    let other_ino = other_ino.get_or_insert_with(|| {
                        inodes.push_inode(INode {
                            name: OsStr::new("other").into(),
                            parent: Some(*root_ino),
                            ..Default::default()
                        })
                    });
                    inodes.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(*other_ino),
                        data: Some(Data {
                            entry,
                            level: 0,
                            cache: None,
                        }),
                    });
                }
                _ => unimplemented!(),
            }
        }

        Ok(())
    }
}

impl Filesystem for WadFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: Ino, name: &OsStr, reply: ReplyEntry) {
        if let Some((ino, inode)) = self
            .inodes
            .inner
            .iter_mut()
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
        if let Some(inode) = self.inodes.inner.get_mut(ino as usize) {
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
        match self.inodes.inner.get_mut(ino as usize) {
            Some(inode) => match inode.resolve_data() {
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
