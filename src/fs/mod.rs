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

#[derive(Debug)]
struct Data {
    entry: Entry,
    // For mipmaps only
    level: usize,
    cache: Option<Vec<u8>>,
}

#[derive(Debug, Default)]
struct INode {
    /// Name of inode
    name: Cow<'static, OsStr>,
    /// Parent inode if present (root has none)
    parent: Option<u64>,
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

    fn resolve_file_attr(&mut self, ino: u64) -> FileAttr {
        let data = self.resolve_data();
        FileAttr {
            ino,
            size: data.map(|x| x.len()).unwrap_or(0) as u64,
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
pub struct WadFS {
    ttl_attr: Duration,
    inodes: Vec<INode>,

    root_ino: u64,
    pics_ino: u64,
    miptexs_ino: u64,
    fonts_ino: u64,
    other_ino: u64,
}

impl WadFS {
    pub fn new() -> Self {
        let mut this = Self {
            inodes: Vec::new(),
            ttl_attr: DEFAULT_ATTR_TTL,

            root_ino: 0,
            pics_ino: 0,
            miptexs_ino: 0,
            fonts_ino: 0,
            other_ino: 0,
        };

        this.root_ino = this.push_inode(INode {
            name: OsStr::new(".").into(),
            ..Default::default()
        });
        this.pics_ino = this.push_inode(INode {
            name: OsStr::new("pics").into(),
            parent: Some(this.root_ino),
            ..Default::default()
        });
        this.miptexs_ino = this.push_inode(INode {
            name: OsStr::new("miptexs").into(),
            parent: Some(this.root_ino),
            ..Default::default()
        });
        this.fonts_ino = this.push_inode(INode {
            name: OsStr::new("fonts").into(),
            parent: Some(this.root_ino),
            ..Default::default()
        });
        this.other_ino = this.push_inode(INode {
            name: OsStr::new("other").into(),
            parent: Some(this.root_ino),
            ..Default::default()
        });

        this
    }

    pub fn append_entries<R: Read + Seek + Send + Sync + 'static>(
        &mut self,
        reader: R,
    ) -> io::Result<()> {
        let entries = goldsrc_rs::wad_entries(reader, true)?;

        for (name, entry) in entries {
            match entry.ty {
                ContentType::Picture => {
                    self.push_inode(INode {
                        name: OsString::from(util::pic_name(name)).into(),
                        parent: Some(self.pics_ino),
                        data: Some(Data {
                            entry,
                            level: 1,
                            cache: None,
                        }),
                    });
                }
                ContentType::MipTexture => {
                    let miptex_ino = self.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(self.miptexs_ino),
                        ..Default::default()
                    });
                    for i in 0..MIP_LEVELS {
                        let entry = entry.clone();
                        self.push_inode(INode {
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
                    self.push_inode(INode {
                        name: OsString::from(util::pic_name(name)).into(),
                        parent: Some(self.fonts_ino),
                        data: Some(Data {
                            entry,
                            level: 1,
                            cache: None,
                        }),
                    });
                }
                ContentType::Other(_) => {
                    self.push_inode(INode {
                        name: OsString::from(name.as_str()).into(),
                        parent: Some(self.other_ino),
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

    fn push_inode(&mut self, inode: INode) -> u64 {
        if self.inodes.is_empty() {
            self.inodes.push(INode::default());
        }

        let idx = self.inodes.len() as u64;
        self.inodes.push(inode);
        idx
    }
}

impl Filesystem for WadFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if let Some((ino, inode)) = self
            .inodes
            .iter_mut()
            .enumerate()
            .find(|(_, inode)| inode.parent == Some(parent) && inode.name == name)
        {
            reply.entry(&self.ttl_attr, &inode.resolve_file_attr(ino as u64), 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        for (i, (ino, inode)) in self
            .inodes
            .iter()
            .enumerate()
            .filter(|(_, inode)| inode.parent == Some(ino))
            .enumerate()
            .skip(offset as usize)
            // FIXME: Error if removed
            .take(5)
        {
            if reply.add(ino as u64, (i + 1) as i64, inode.file_type(), &inode.name) {
                return;
            }
        }
        reply.ok()
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if let Some(inode) = self.inodes.get_mut(ino as usize) {
            reply.attr(&self.ttl_attr, &inode.resolve_file_attr(ino));
        } else {
            reply.error(ENOENT);
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        match self.inodes.get_mut(ino as usize) {
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
