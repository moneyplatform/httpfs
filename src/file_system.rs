use std::ffi::OsStr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use fuser::{
    FileAttr, Filesystem, FileType, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    Request,
};
use libc::ENOENT;
use log::debug;
use users::{get_current_gid, get_current_uid};

use crate::http_reader::{DataAddr, HttpReader};

const FILE_INFO_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_READERS: usize = 5;

pub struct HttpFs {
    readers: Arc<Mutex<Vec<Arc<HttpReader>>>>,
    file_size: usize,
    file_name: String,
    resource_url: String,
    additional_headers: Vec<String>,
}

impl HttpFs {
    pub fn new(url: &str, file_size: usize, file_name: &str, additional_headers: Vec<String>) -> Self {
        HttpFs {
            readers: Arc::new(Mutex::new(vec![])),
            file_size,
            file_name: String::from(file_name),
            resource_url: String::from(url),
            additional_headers,
        }
    }

    pub fn drain_data_from_suitable_reader(&self, offset: usize, size: usize) -> Vec<u8> {
        let addr = DataAddr::new(offset, size);
        let arc = Arc::clone(&self.readers);
        let mut readers = arc.lock().unwrap();

        let mut res: Option<Vec<u8>> = None;
        for reader in &*readers {
            res = reader.try_drain_data(addr);
            if res != None {
                break;
            }
        }
        if res == None {
            debug!("!------- Suitable reader not found, creating new...");
            let reader = Arc::new(HttpReader::new(&self.resource_url, offset, self.file_size, self.additional_headers.clone()));
            let rc = Arc::clone(&reader);
            thread::spawn(move || {
                rc.fetching_loop();
            });
            debug!("HttpReader fetching loop has started");
            res = reader.try_drain_data(addr);
            readers.push(reader);

            if readers.len() > MAX_READERS {
                let stop_readers_to = readers.len() - MAX_READERS;
                debug!("Readers 0..{} will be stopped", stop_readers_to);
                for reader in &readers[0..stop_readers_to] {
                    debug!("Call stop");
                    reader.stop();
                }
                debug!("Readers {}..{} will work", stop_readers_to, readers.len());
                *readers = readers[stop_readers_to..readers.len()].to_vec();
            }
            debug!("Total readers now {}", readers.len());
        }
        let data = res.unwrap();
        data
    }

    fn get_file_attr(&self) -> FileAttr {
        FileAttr {
            ino: 2,
            size: self.file_size as u64,
            blocks: 1,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::RegularFile,
            perm: 0o644,
            nlink: 1,
            uid: get_current_uid(),
            gid: get_current_gid(),
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }

    fn get_dir_attr(&self) -> FileAttr {
        FileAttr {
            ino: 1,
            size: 0,
            blocks: 0,
            atime: SystemTime::now(),
            mtime: SystemTime::now(),
            ctime: SystemTime::now(),
            crtime: SystemTime::now(),
            kind: FileType::Directory,
            perm: 0o755,
            nlink: 2,
            uid: get_current_uid(),
            gid: get_current_gid(),
            rdev: 0,
            flags: 0,
            blksize: 512,
        }
    }
}

impl Filesystem for HttpFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent == 1 && name.to_str() == Some(&self.file_name) {
            reply.entry(&FILE_INFO_CACHE_TTL, &self.get_file_attr(), 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        match ino {
            1 => reply.attr(&FILE_INFO_CACHE_TTL, &self.get_dir_attr()),
            2 => reply.attr(&FILE_INFO_CACHE_TTL, &self.get_file_attr()),
            _ => reply.error(ENOENT),
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        debug!("-------> Requested data block: offset={} size={}", offset, _size);
        if ino == 2 {
            let data = self
                .drain_data_from_suitable_reader(offset as usize, _size as usize);
            debug!("-------> Replied data block: offset={} size={}", offset, data.len());
            reply.data(&data);
        } else {
            reply.error(ENOENT);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != 1 {
            reply.error(ENOENT);
            return;
        }

        let entries = vec![
            (1, FileType::Directory, "."),
            (1, FileType::Directory, ".."),
            (2, FileType::RegularFile, &self.file_name),
        ];

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            // i + 1 means the index of the next entry
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        reply.ok();
    }
}
