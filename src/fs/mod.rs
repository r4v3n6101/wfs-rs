use std::{
    borrow::Cow,
    ffi::OsStr,
    io::{self, Read, Seek},
    sync::RwLock,
    time::{Duration, SystemTime},
};

use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, Request,
};
use libc::{EIO, ENOENT};
use rayon::iter::{IntoParallelIterator, ParallelIterator};

mod util;

const DEFAULT_ATTR_TTL: Duration = Duration::from_secs(60);
const ROOT_INO: Ino = 1;
const PICS_DIR_INO: Ino = 2;
const MIPTEXS_DIR_INO: Ino = 3;
const FONTS_DIR_INO: Ino = 4;
const OTHER_DIR_INO: Ino = 5;

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

    fn file_attr(&self, ino: Ino) -> FileAttr {
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
pub struct WadFS {
    ttl_attr: Duration,
    inodes: RwLock<Vec<INode>>,
}

impl WadFS {
    pub fn new() -> Self {
        let inodes = vec![
            INode::default(),
            INode {
                name: OsStr::new(".").into(),
                ..Default::default()
            },
            INode {
                name: OsStr::new("pics").into(),
                parent: Some(ROOT_INO),
                ..Default::default()
            },
            INode {
                name: OsStr::new("miptexs").into(),
                parent: Some(ROOT_INO),
                ..Default::default()
            },
            INode {
                name: OsStr::new("fonts").into(),
                parent: Some(ROOT_INO),
                ..Default::default()
            },
            INode {
                name: OsStr::new("other").into(),
                parent: Some(ROOT_INO),
                ..Default::default()
            },
        ];

        Self {
            inodes: RwLock::new(inodes),
            ttl_attr: DEFAULT_ATTR_TTL,
        }
    }

    pub fn append_entries<R: Read + Seek + Send + Sync + 'static>(
        &mut self,
        reader: R,
    ) -> io::Result<()> {
        goldsrc_rs::wad_entries(reader, true)?
            .into_par_iter()
            .for_each(|(name, entry)| util::create_inode(self, name, entry));

        Ok(())
    }
}

impl Filesystem for WadFS {
    fn lookup(&mut self, _req: &Request<'_>, parent: Ino, name: &OsStr, reply: ReplyEntry) {
        if let Some((ino, inode)) = self
            .inodes
            .read()
            .unwrap()
            .iter()
            .enumerate()
            .find(|(_, inode)| inode.parent == Some(parent) && inode.name == name)
        {
            reply.entry(&self.ttl_attr, &inode.file_attr(ino as Ino), 0);
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
            .read()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, inode)| inode.parent == Some(ino))
            .enumerate()
            .skip(offset as usize)
        {
            if reply.add(ino as Ino, (i + 1) as i64, inode.file_type(), &inode.name) {
                break;
            }
        }
        reply.ok()
    }

    fn getattr(&mut self, _req: &Request<'_>, ino: Ino, reply: ReplyAttr) {
        if let Some(inode) = self.inodes.read().unwrap().get(ino as usize) {
            reply.attr(&self.ttl_attr, &inode.file_attr(ino));
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
        match self.inodes.read().unwrap().get(ino as usize) {
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
