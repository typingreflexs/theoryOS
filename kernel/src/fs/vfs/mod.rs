//! Virtual filesystem core — inodes, dentries, mounts, path walk, file ops.

pub mod dentry;
pub mod file;
pub mod inode;
pub mod mount;
pub mod ops;
pub mod path;
pub mod superblock;

pub use dentry::{Dentry, DentryCache};
pub use file::{OpenFile, FileMode};
pub use inode::{FileType, InodeAttr, InodeId, InodeMode};
pub use mount::{MountPoint, MountTable};
pub use ops::{fs_err_to_errno, lseek_file, mkdir_path, open_path, read_file, read_path, stat_path, symlink_path, write_file, write_path};
pub use path::{PathResolution, resolve_path, resolve_path_at, ResolvedInode};
pub use superblock::{FileSystem, FsError, DirEntry};
