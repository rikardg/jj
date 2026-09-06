// Copyright 2025 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Git LFS pointer parsing and local object cache access.

use std::fs;
use std::fs::File;
use std::io;
use std::io::Read;
use std::io::Seek as _;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use sha2::Digest as _;
use sha2::Sha256;

const LFS_VERSION_LINE: &str = "version https://git-lfs.github.com/spec/v1";
const LFS_OID_PREFIX: &str = "oid sha256:";

/// Largest file that is worth inspecting as an LFS pointer.
///
/// The spec caps pointers at 1024 bytes; the ones Git LFS writes are around
/// 130. Anything bigger is content, so it isn't read into memory.
const MAX_LFS_POINTER_SIZE: u64 = 1024;

/// A parsed Git LFS pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsPointer {
    /// SHA-256 hex digest of the object content.
    pub oid: String,
    /// Size in bytes of the object content.
    pub size: u64,
}

/// Returns `true` if `bytes` looks like an LFS pointer (quick check).
pub fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(LFS_VERSION_LINE.as_bytes())
}

/// Parses an LFS pointer from its text representation.
///
/// Returns `None` if the bytes are not a valid LFS pointer.
pub fn parse_lfs_pointer(bytes: &[u8]) -> Option<LfsPointer> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines();

    if lines.next()? != LFS_VERSION_LINE {
        return None;
    }

    let oid_line = lines.next()?;
    let oid = oid_line.strip_prefix(LFS_OID_PREFIX)?;
    if oid.len() != 64 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    let size_line = lines.next()?;
    let size_str = size_line.strip_prefix("size ")?;
    let size: u64 = size_str.parse().ok()?;

    Some(LfsPointer {
        oid: oid.to_string(),
        size,
    })
}

/// Generates the text representation of an LFS pointer.
pub fn generate_lfs_pointer(pointer: &LfsPointer) -> Vec<u8> {
    format!(
        "{LFS_VERSION_LINE}\n{LFS_OID_PREFIX}{}\nsize {}\n",
        pointer.oid, pointer.size
    )
    .into_bytes()
}

/// Computes the path to an LFS object in the local cache.
pub fn lfs_cache_path(git_dir: &Path, oid: &str) -> PathBuf {
    git_dir
        .join("lfs")
        .join("objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid)
}

/// Opens an LFS object from the local cache for reading.
pub fn read_lfs_object(git_dir: &Path, pointer: &LfsPointer) -> io::Result<File> {
    let path = lfs_cache_path(git_dir, &pointer.oid);
    File::open(path)
}

/// Returns the contents of `file` if they are already an LFS pointer.
///
/// `file` is rewound before returning, so the caller can read it from the
/// start whichever way this goes.
///
/// A working-copy file can already hold a pointer: the checkout writes one as
/// a placeholder when the object is missing from the local cache, and Git
/// writes one when it checks the file out without the LFS smudge filter
/// configured. Such a file must be stored verbatim rather than cleaned again,
/// which would produce a pointer to the pointer text and detach the commit
/// from the real content.
pub fn read_lfs_pointer_file(file: &mut File) -> io::Result<Option<Vec<u8>>> {
    let size = file.metadata()?.len();
    if size > MAX_LFS_POINTER_SIZE {
        return Ok(None);
    }
    let mut bytes = Vec::with_capacity(size as usize);
    let read = file.read_to_end(&mut bytes);
    file.rewind()?;
    read?;
    Ok(parse_lfs_pointer(&bytes).is_some().then_some(bytes))
}

/// Writes content to the LFS object cache using streaming SHA-256 hashing.
///
/// Returns the LFS pointer for the written content. Hashes and writes to
/// a temp file in a single pass, then either persists or discards the temp
/// file depending on whether the object was already cached.
pub fn write_lfs_object(git_dir: &Path, mut content: impl Read) -> io::Result<LfsPointer> {
    let tmp_dir = git_dir.join("lfs").join("tmp");
    fs::create_dir_all(&tmp_dir)?;

    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut buf = [0u8; 65536];
    let mut tmp_file = io::BufWriter::new(tempfile::NamedTempFile::new_in(&tmp_dir)?);

    loop {
        let n = content.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        tmp_file.write_all(&buf[..n])?;
        size += n as u64;
    }
    tmp_file.flush()?;
    let tmp_file = tmp_file.into_inner()?;

    let oid = hex::encode(hasher.finalize());
    let dest = lfs_cache_path(git_dir, &oid);

    if dest.exists() {
        // Already cached — discard the temp file.
        return Ok(LfsPointer { oid, size });
    }

    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }

    match tmp_file.persist(&dest) {
        Ok(_) => {}
        Err(e) if dest.exists() => {
            drop(e);
        }
        Err(e) => return Err(e.error),
    }

    Ok(LfsPointer { oid, size })
}

/// Hex-encode helper (avoids pulling in the `hex` crate).
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            write!(s, "{b:02x}").unwrap();
            s
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_POINTER: &str = "\
version https://git-lfs.github.com/spec/v1
oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393
size 12345
";

    #[test]
    fn test_parse_valid_pointer() {
        let pointer = parse_lfs_pointer(SAMPLE_POINTER.as_bytes()).unwrap();
        assert_eq!(
            pointer.oid,
            "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393"
        );
        assert_eq!(pointer.size, 12345);
    }

    #[test]
    fn test_is_lfs_pointer() {
        assert!(is_lfs_pointer(SAMPLE_POINTER.as_bytes()));
        assert!(!is_lfs_pointer(b"not a pointer"));
        assert!(!is_lfs_pointer(b""));
    }

    #[test]
    fn test_parse_invalid_content() {
        assert!(parse_lfs_pointer(b"").is_none());
        assert!(parse_lfs_pointer(b"not a pointer").is_none());
        assert!(parse_lfs_pointer(b"version https://git-lfs.github.com/spec/v1\n").is_none());
        // Wrong hash length
        assert!(
            parse_lfs_pointer(
                b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 10\n"
            )
            .is_none()
        );
        // Non-hex characters
        assert!(parse_lfs_pointer(
            b"version https://git-lfs.github.com/spec/v1\noid sha256:zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\nsize 10\n"
        )
        .is_none());
        // Missing size
        assert!(parse_lfs_pointer(
            b"version https://git-lfs.github.com/spec/v1\noid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n"
        )
        .is_none());
    }

    #[test]
    fn test_roundtrip_pointer() {
        let pointer = LfsPointer {
            oid: "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393".to_string(),
            size: 12345,
        };
        let bytes = generate_lfs_pointer(&pointer);
        let parsed = parse_lfs_pointer(&bytes).unwrap();
        assert_eq!(parsed, pointer);
    }

    #[test]
    fn test_cache_path() {
        let path = lfs_cache_path(
            Path::new("/repo/.git"),
            "4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393",
        );
        assert_eq!(
            path,
            PathBuf::from(
                "/repo/.git/lfs/objects/4d/7a/\
                 4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393"
            )
        );
    }

    #[test]
    fn test_read_lfs_pointer_file() {
        let temp_dir = tempfile::tempdir().unwrap();

        let write = |name: &str, content: &[u8]| {
            let path = temp_dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            File::open(&path).unwrap()
        };

        // A pointer file is returned verbatim, and left readable from the start.
        let mut file = write("pointer", SAMPLE_POINTER.as_bytes());
        assert_eq!(
            read_lfs_pointer_file(&mut file).unwrap().as_deref(),
            Some(SAMPLE_POINTER.as_bytes())
        );
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, SAMPLE_POINTER.as_bytes());

        // Ordinary content is not, however small.
        let mut file = write("content", b"stored content\n");
        assert!(read_lfs_pointer_file(&mut file).unwrap().is_none());
        let mut rest = Vec::new();
        file.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"stored content\n");

        // Neither is content that merely starts like a pointer.
        let truncated = SAMPLE_POINTER.strip_suffix("size 12345\n").unwrap();
        let mut file = write("truncated", truncated.as_bytes());
        assert!(read_lfs_pointer_file(&mut file).unwrap().is_none());

        // A file too big to be a pointer is not read at all.
        let mut oversized = SAMPLE_POINTER.as_bytes().to_vec();
        oversized.resize(MAX_LFS_POINTER_SIZE as usize + 1, b'\n');
        let mut file = write("oversized", &oversized);
        assert!(read_lfs_pointer_file(&mut file).unwrap().is_none());

        let mut file = write("empty", b"");
        assert!(read_lfs_pointer_file(&mut file).unwrap().is_none());
    }

    #[test]
    fn test_write_and_read_lfs_object() {
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path();

        let content = b"hello, this is LFS content";
        let pointer = write_lfs_object(git_dir, io::Cursor::new(content)).unwrap();
        assert_eq!(pointer.size, content.len() as u64);

        // Verify the pointer oid is a valid sha256 of the content.
        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected_oid = hex::encode(hasher.finalize());
        assert_eq!(pointer.oid, expected_oid);

        // Read back and verify content.
        let mut file = read_lfs_object(git_dir, &pointer).unwrap();
        let mut read_back = Vec::new();
        file.read_to_end(&mut read_back).unwrap();
        assert_eq!(read_back, content);
    }

    #[test]
    fn test_write_lfs_object_idempotent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let git_dir = temp_dir.path();

        let content = b"same content twice";
        let p1 = write_lfs_object(git_dir, io::Cursor::new(content)).unwrap();
        let p2 = write_lfs_object(git_dir, io::Cursor::new(content)).unwrap();
        assert_eq!(p1, p2);
    }

    #[test]
    fn test_generate_pointer_format() {
        let pointer = LfsPointer {
            oid: "abcd".repeat(16),
            size: 42,
        };
        let bytes = generate_lfs_pointer(&pointer);
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.starts_with("version https://git-lfs.github.com/spec/v1\n"));
        assert!(text.contains(&format!("oid sha256:{}", pointer.oid)));
        assert!(text.contains("size 42\n"));
        assert!(text.ends_with('\n'));
    }
}
