//! index node(inode, namely file control block) layer
//!
//! The data struct and functions for the inode layer that service file-related system calls
//!
//! NOTICE: The difference between [`Inode`] and [`DiskInode`]  can be seen from their names: DiskInode in a relatively fixed location within the disk block, while Inode Is a data structure placed in memory that records file inode information.
use super::{
    block_cache_sync_all, get_block_cache, BlockDevice, DirEntry, DiskInode, DiskInodeType,
    EasyFileSystem, DIRENT_SZ,
};
use crate::layout::NodePos;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use spin::{Mutex, MutexGuard};

/// Inode struct in memory
/// Fstat
pub struct Fstat {
    /// ref count
    pub ref_count: u32,
    /// is directory
    pub is_dir: bool,
    /// inode id
    pub inode_id: usize,
}

/// Virtual filesystem layer over easy-fs
pub struct Inode {
    /// The block id of the inode
    block_id: usize,
    /// The offset of the inode in the block
    block_offset: usize,
    /// The file system
    fs: Arc<Mutex<EasyFileSystem>>,
    /// The block device
    block_device: Arc<dyn BlockDevice>,
}

impl Inode {
    /// Create a new Disk Inode
    ///
    /// We should not acquire efs lock here.
    pub fn new(
        block_id: u32,
        block_offset: usize,
        fs: Arc<Mutex<EasyFileSystem>>,
        block_device: Arc<dyn BlockDevice>,
    ) -> Self {
        Self {
            block_id: block_id as usize,
            block_offset,
            fs,
            block_device,
        }
    }
    /// read the content of the disk inode on disk with 'f' function
    fn read_disk_inode<V>(&self, f: impl FnOnce(&DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .read(self.block_offset, f)
    }
    /// modify the content of the disk inode on disk with 'f' function
    fn modify_disk_inode<V>(&self, f: impl FnOnce(&mut DiskInode) -> V) -> V {
        get_block_cache(self.block_id, Arc::clone(&self.block_device))
            .lock()
            .modify(self.block_offset, f)
    }
    /// find the disk inode id according to the file with 'name' by search the directory entries in the disk inode with Directory type
    fn find_inode_pos(&self, name: &str, disk_inode: &DiskInode) -> Option<NodePos> {
        // assert it is a directory
        assert!(disk_inode.is_dir());
        let file_count = disk_inode.dir_count();
        let mut dirent = DirEntry::empty();
        for i in 0..file_count {
            assert_eq!(
                disk_inode.read_at(DIRENT_SZ * i, dirent.as_bytes_mut(), &self.block_device,),
                DIRENT_SZ,
            );
            if dirent.equal(name) {
                return Some(NodePos::new(dirent.inode_id(), i));
            }
        }
        None
    }
    /// Find inode under current inode by name
    pub fn find(&self, name: &str) -> Option<Arc<Inode>> {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            self.find_inode_pos(name, disk_inode)
                .map(|node_pos: NodePos| {
                    let (block_id, block_offset) = fs.get_disk_inode_pos(node_pos.file_inode_id);
                    Arc::new(Self::new(
                        block_id,
                        block_offset,
                        self.fs.clone(),
                        self.block_device.clone(),
                    ))
                })
        })
    }
    /// increase the size of file( also known as 'disk inode')
    fn increase_size(
        &self,
        new_size: u32,
        disk_inode: &mut DiskInode,
        fs: &mut MutexGuard<EasyFileSystem>,
    ) {
        if new_size < disk_inode.size {
            return;
        }
        let blocks_needed = disk_inode.blocks_num_needed(new_size);
        let mut v: Vec<u32> = Vec::new();
        for _ in 0..blocks_needed {
            v.push(fs.alloc_data());
        }
        disk_inode.increase_size(new_size, v, &self.block_device);
    }

    fn exist(&self, name: &str) -> bool {
        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_pos(name, root_inode)
        };
        self.read_disk_inode(op).is_some()
    }

    fn add_dir_entry(&self, name: &str, inode_id: u32, fs: &mut MutexGuard<EasyFileSystem>) {
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.ref_count += 1;
            });
        self.modify_disk_inode(|parent_node| {
            // append file in the dirent
            let file_count = parent_node.dir_count();
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, parent_node, fs);
            // write dirent
            let dirent = DirEntry::new(name, inode_id);
            parent_node.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });
    }

    /// create a file with 'name' in the root directory
    pub fn create(&self, name: &str) -> Option<Arc<Inode>> {
        let mut fs = self.fs.lock();
        let op = |root_inode: &mut DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_pos(name, root_inode)
        };
        if self.modify_disk_inode(op).is_some() {
            return None;
        }
        // create a new file
        // alloc an inode with an indirect block
        let new_inode_id = fs.alloc_inode();
        // initialize inode
        let (new_inode_block_id, new_inode_block_offset) = fs.get_disk_inode_pos(new_inode_id);
        get_block_cache(new_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(new_inode_block_offset, |new_inode: &mut DiskInode| {
                new_inode.initialize(DiskInodeType::File);
            });
        self.modify_disk_inode(|root_inode| {
            // append file in the dirent
            let file_count = (root_inode.size as usize) / DIRENT_SZ;
            let new_size = (file_count + 1) * DIRENT_SZ;
            // increase size
            self.increase_size(new_size as u32, root_inode, &mut fs);
            // write dirent
            let dirent = DirEntry::new(name, new_inode_id);
            root_inode.write_at(
                file_count * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });

        let (block_id, block_offset) = fs.get_disk_inode_pos(new_inode_id);
        block_cache_sync_all();
        // return inode
        Some(Arc::new(Self::new(
            block_id,
            block_offset,
            self.fs.clone(),
            self.block_device.clone(),
        )))
        // release efs lock automatically by compiler
    }

    fn drop_dir_entry(&self, name: &str, fs: &mut MutexGuard<EasyFileSystem>) -> i32 {
        let node_pos =
            self.read_disk_inode(|disk_inode| self.find_inode_pos(name, disk_inode));
        if node_pos.is_none() {
            return -1;
        }
        let file_pos = node_pos.unwrap();
        let (file_inode_block_id, file_inode_offset) =
            fs.get_disk_inode_pos(file_pos.file_inode_id);

        get_block_cache(file_inode_block_id as usize, Arc::clone(&self.block_device))
            .lock()
            .modify(file_inode_offset, |file: &mut DiskInode| {
                file.deref();
                if file.empty() {
                    file.clear_size(&self.block_device);
                }
            });

        self.modify_disk_inode(|parent_node| {
            // write dirent
            let dirent = DirEntry::new(name, 0);
            parent_node.write_at(
                file_pos.dir_entry_index * DIRENT_SZ,
                dirent.as_bytes(),
                &self.block_device,
            );
        });


        0
    }


    /// create a directory with 'name' in the root directory
    ///
    /// list the file names in the root directory

    /// Create link inode under current inode by name
    pub fn linkat(&self, new_name: &str, old_name: &str) -> i32 {
        let mut fs = self.fs.lock();
        if self.exist(new_name) {
            return -1;
        }

        let op = |root_inode: &DiskInode| {
            // assert it is a directory
            assert!(root_inode.is_dir());
            // has the file been created?
            self.find_inode_pos(old_name, root_inode)
        };
        if let Some(node_pos) = self.read_disk_inode(op) {
            self.add_dir_entry(new_name, node_pos.file_inode_id, &mut fs);
            block_cache_sync_all();
            0
        } else {
            -1
        }
    }

    /// unlink_at unlink path
    pub fn unlink_at(&self, path: &str) -> i32 {
        let mut fs = self.fs.lock();
        self.drop_dir_entry(path, &mut fs)
    }

    /// fstat
    pub fn fstat(&self) -> Fstat {
        let fs = self.fs.lock();
        self.read_disk_inode(|disk_inode: &DiskInode| {
            let inode_id = fs.get_inode_id_by_block_id(self.block_id, self.block_offset);
            Fstat{
                ref_count: disk_inode.ref_count,
                is_dir: disk_inode.is_dir(),
                inode_id,
            }
        })
    }

    /// List inodes under current inode
    pub fn ls(&self) -> Vec<String> {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| {
            let file_count = disk_inode.dir_count();
            let mut v: Vec<String> = Vec::new();
            for i in 0..file_count {
                let mut dirent = DirEntry::empty();
                assert_eq!(
                    disk_inode.read_at(i * DIRENT_SZ, dirent.as_bytes_mut(), &self.block_device,),
                    DIRENT_SZ,
                );
                v.push(String::from(dirent.name()));
            }
            v
        })
    }
    /// Read the content in offset position of the file into 'buf'
    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> usize {
        let _fs = self.fs.lock();
        self.read_disk_inode(|disk_inode| disk_inode.read_at(offset, buf, &self.block_device))
    }
    /// Write the content in 'buf' into offset position of the file
    pub fn write_at(&self, offset: usize, buf: &[u8]) -> usize {
        let mut fs = self.fs.lock();
        let size = self.modify_disk_inode(|disk_inode| {
            self.increase_size((offset + buf.len()) as u32, disk_inode, &mut fs);
            disk_inode.write_at(offset, buf, &self.block_device)
        });
        block_cache_sync_all();
        size
    }
    /// Set the file(disk inode) length to zero, delloc all data blocks of the file.
    pub fn clear(&self) {
        let mut fs = self.fs.lock();
        self.modify_disk_inode(|disk_inode| {
            let size = disk_inode.size;
            let data_blocks_dealloc = disk_inode.clear_size(&self.block_device);
            assert!(data_blocks_dealloc.len() == DiskInode::total_blocks(size) as usize);
            for data_block in data_blocks_dealloc.into_iter() {
                fs.dealloc_data(data_block);
            }
        });
        block_cache_sync_all();
    }
}
