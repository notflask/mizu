//! Just enough of the zip format to read entries of an EPUB: the central
//! directory, stored and deflated entries. Large books (hundreds of MB of
//! images) are never read as a whole; entries are read on demand.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
struct Entry {
    method: u16,
    comp_size: u64,
    size: u64,
    /// Offset of the local file header.
    header: u64,
}

#[derive(Debug)]
pub struct Zip {
    path: PathBuf,
    entries: HashMap<String, Entry>,
}

fn u16le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn bad(what: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("zip: {what}"))
}

impl Zip {
    pub fn open(path: &Path) -> std::io::Result<Zip> {
        let mut f = File::open(path)?;
        let len = f.seek(SeekFrom::End(0))?;
        // End of central directory: 22 bytes plus a comment of up to 64 KiB.
        let tail = len.min(22 + 65535);
        f.seek(SeekFrom::Start(len - tail))?;
        let mut buf = vec![0u8; tail as usize];
        f.read_exact(&mut buf)?;
        let eocd = (0..buf.len().saturating_sub(21))
            .rev()
            .find(|&i| buf[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
            .ok_or_else(|| bad("no end of central directory"))?;
        let count = u16le(&buf, eocd + 10).ok_or_else(|| bad("short eocd"))? as usize;
        let cd_size = u32le(&buf, eocd + 12).ok_or_else(|| bad("short eocd"))? as u64;
        let cd_offset = u32le(&buf, eocd + 16).ok_or_else(|| bad("short eocd"))? as u64;
        if cd_offset == 0xffff_ffff || cd_offset + cd_size > len {
            return Err(bad("zip64 or damaged archive"));
        }
        f.seek(SeekFrom::Start(cd_offset))?;
        let mut cd = vec![0u8; cd_size as usize];
        f.read_exact(&mut cd)?;
        let mut entries = HashMap::with_capacity(count);
        let mut at = 0;
        while at + 46 <= cd.len() && cd[at..at + 4] == [0x50, 0x4b, 0x01, 0x02] {
            let method = u16le(&cd, at + 10).unwrap_or(0);
            let comp_size = u32le(&cd, at + 20).unwrap_or(0) as u64;
            let size = u32le(&cd, at + 24).unwrap_or(0) as u64;
            let name_len = u16le(&cd, at + 28).unwrap_or(0) as usize;
            let extra_len = u16le(&cd, at + 30).unwrap_or(0) as usize;
            let comment_len = u16le(&cd, at + 32).unwrap_or(0) as usize;
            let header = u32le(&cd, at + 42).unwrap_or(0) as u64;
            let name = cd
                .get(at + 46..at + 46 + name_len)
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .ok_or_else(|| bad("short entry name"))?;
            entries.insert(
                name,
                Entry {
                    method,
                    comp_size,
                    size,
                    header,
                },
            );
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip {
            path: path.to_path_buf(),
            entries,
        })
    }

    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// Read and decompress one entry.
    pub fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        let e = *self
            .entries
            .get(name)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string()))?;
        let mut f = File::open(&self.path)?;
        f.seek(SeekFrom::Start(e.header))?;
        let mut local = [0u8; 30];
        f.read_exact(&mut local)?;
        if local[0..4] != [0x50, 0x4b, 0x03, 0x04] {
            return Err(bad("bad local header"));
        }
        let skip = u16le(&local, 26).unwrap_or(0) as i64 + u16le(&local, 28).unwrap_or(0) as i64;
        f.seek(SeekFrom::Current(skip))?;
        let mut data = vec![0u8; e.comp_size as usize];
        f.read_exact(&mut data)?;
        match e.method {
            0 => Ok(data),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(&data, e.size as usize + 1)
                .map_err(|_| bad("cannot inflate")),
            m => Err(bad(&format!("compression method {m} not supported"))),
        }
    }

    pub fn read_string(&self, name: &str) -> std::io::Result<String> {
        Ok(String::from_utf8_lossy(&self.read(name)?).into_owned())
    }
}

/// Resolve `href` (relative to the entry `base`) to an entry name.
pub fn resolve(base: &str, href: &str) -> String {
    let href = href.split(['#', '?']).next().unwrap_or("");
    let href = percent_decode(href);
    let mut parts: Vec<&str> = match base.rfind('/') {
        Some(i) => base[..i].split('/').collect(),
        None => Vec::new(),
    };
    if href.starts_with('/') {
        parts.clear();
    }
    for p in href.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_paths() {
        assert_eq!(
            resolve("OEBPS/text/p1.xhtml", "../images/a.png"),
            "OEBPS/images/a.png"
        );
        assert_eq!(
            resolve("OEBPS/content.opf", "text/p1.xhtml#x"),
            "OEBPS/text/p1.xhtml"
        );
        assert_eq!(resolve("content.opf", "a%20b.xhtml"), "a b.xhtml");
        assert_eq!(resolve("a/b.opf", "/c.xhtml"), "c.xhtml");
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.epub");
        std::fs::write(&path, crate::testutil::make_epub(&[("One", "Hello zip")])).unwrap();
        let z = Zip::open(&path).unwrap();
        assert_eq!(z.read_string("mimetype").unwrap(), "application/epub+zip");
        assert!(z
            .read_string("OEBPS/c1.xhtml")
            .unwrap()
            .contains("Hello zip"));
        assert!(z.read("nope").is_err());

        // A deflated entry.
        let data = b"deflated text deflated text deflated text".repeat(20);
        let comp = miniz_oxide::deflate::compress_to_vec(&data, 6);
        let zipped = crate::testutil::zip_entries(&[("x.txt", &comp, 8, data.len())]);
        std::fs::write(&path, zipped).unwrap();
        let z = Zip::open(&path).unwrap();
        assert_eq!(z.read("x.txt").unwrap(), data);
    }
}
