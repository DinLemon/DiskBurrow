//! A metadata-only streaming protocol. Counts are limits, never allocation hints.
use crate::live_index::{entry_reserve, validate_limits, validate_name, validate_root};
use crate::{LiveEntry, LiveIndex, MAXIMUM_ENTRIES, MAXIMUM_RESIDENT_BYTES};
use anyhow::{Result, ensure};
use std::io::{Read, Write};
pub const MAXIMUM_BYTES: usize = 1024 * 1024 * 1024;
pub fn write_error(stream: &mut impl Write, message: &str) -> Result<()> {
    ensure!(
        message.len() <= 4096 && !message.contains('\0'),
        "Invalid helper error frame"
    );
    stream.write_all(&(message.len() as u32).to_le_bytes())?;
    stream.write_all(message.as_bytes())?;
    stream.flush()?;
    Ok(())
}
pub fn read_error(stream: &mut impl Read) -> Result<String> {
    let mut length = [0; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length) as usize;
    ensure!(length <= 4096, "Helper error exceeds its limit");
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes)?;
    let message = String::from_utf8(bytes)?;
    ensure!(!message.contains('\0'), "Invalid helper error frame");
    Ok(message)
}
fn nonce_valid(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn volume_root(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}
pub fn validate_peer(
    expected_user: &str,
    actual_user: &str,
    expected_nonce: &str,
    actual_nonce: &str,
) -> Result<()> {
    ensure!(
        expected_user.starts_with("S-1-")
            && expected_user == actual_user
            && nonce_valid(expected_nonce)
            && expected_nonce == actual_nonce,
        "MFT helper authentication failed"
    );
    Ok(())
}
pub fn write_hello(stream: &mut impl Write, nonce: &str, root: &str) -> Result<()> {
    ensure!(
        nonce_valid(nonce) && volume_root(root),
        "Invalid helper handshake"
    );
    stream.write_all(b"DBRUST01")?;
    stream.write_all(nonce.as_bytes())?;
    stream.write_all(root.as_bytes())?;
    stream.flush()?;
    Ok(())
}
pub fn read_hello(stream: &mut impl Read, nonce: &str, root: &str) -> Result<()> {
    ensure!(
        nonce_valid(nonce) && volume_root(root),
        "Invalid expected handshake"
    );
    let mut hello = [0u8; 75];
    stream.read_exact(&mut hello)?;
    ensure!(
        &hello[..8] == b"DBRUST01"
            && &hello[8..72] == nonce.as_bytes()
            && &hello[72..] == root.as_bytes(),
        "MFT helper authentication failed"
    );
    Ok(())
}
struct Budget<T> {
    inner: T,
    remaining: usize,
}
impl<T: Read> Read for Budget<T> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            return Err(std::io::Error::other("Helper stream budget exhausted"));
        }
        let count = buffer.len().min(self.remaining);
        let n = self.inner.read(&mut buffer[..count])?;
        self.remaining -= n;
        Ok(n)
    }
}
impl<T: Write> Write for Budget<T> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if buffer.len() > self.remaining {
            return Err(std::io::Error::other("Helper stream budget exhausted"));
        }
        let n = self.inner.write(buffer)?;
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
fn limits(bytes: usize) -> Result<()> {
    ensure!(
        bytes > 0 && bytes <= MAXIMUM_BYTES,
        "Invalid helper byte budget"
    );
    Ok(())
}
fn write_text(w: &mut impl Write, text: &str, max_units: usize) -> Result<()> {
    ensure!(
        text.encode_utf16().count() <= max_units,
        "Metadata text too long"
    );
    w.write_all(&u32::try_from(text.len())?.to_le_bytes())?;
    w.write_all(text.as_bytes())?;
    Ok(())
}
fn read_u32(r: &mut impl Read) -> Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn read_i64(r: &mut impl Read) -> Result<i64> {
    let mut b = [0; 8];
    r.read_exact(&mut b)?;
    Ok(i64::from_le_bytes(b))
}
fn read_u8(r: &mut impl Read) -> Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
fn read_bool(r: &mut impl Read) -> Result<bool> {
    let value = read_u8(r)?;
    ensure!(value <= 1, "Invalid metadata boolean");
    Ok(value == 1)
}
fn read_text(r: &mut impl Read, max_units: usize) -> Result<String> {
    let count = read_u32(r)? as usize;
    ensure!(
        count
            <= max_units
                .checked_mul(4)
                .ok_or_else(|| anyhow::anyhow!("Text budget overflow"))?,
        "Metadata text exceeds its limit"
    );
    let mut b = vec![0; count];
    r.read_exact(&mut b)?;
    let text = String::from_utf8(b)?;
    ensure!(
        text.encode_utf16().count() <= max_units,
        "Metadata text too long"
    );
    Ok(text)
}
pub fn write_index(stream: &mut impl Write, index: &LiveIndex) -> Result<()> {
    write_index_with_limits(stream, index, MAXIMUM_BYTES)
}
pub fn write_index_with_limits(
    stream: &mut impl Write,
    index: &LiveIndex,
    maximum_bytes: usize,
) -> Result<()> {
    limits(maximum_bytes)?;
    index.validate(MAXIMUM_ENTRIES, MAXIMUM_RESIDENT_BYTES)?;
    let mut w = Budget {
        inner: stream,
        remaining: maximum_bytes,
    };
    w.write_all(b"DBRUSTI2")?;
    write_text(&mut w, &index.root, 32767)?;
    w.write_all(&u32::try_from(index.entries.len())?.to_le_bytes())?;
    for entry in &index.entries {
        let parent = entry.parent.map_or(Ok(-1), i64::try_from)?;
        w.write_all(&parent.to_le_bytes())?;
        write_text(&mut w, &entry.name, 255)?;
        w.write_all(&[u8::from(entry.directory)])?;
        w.write_all(&entry.logical.to_le_bytes())?;
        w.write_all(&[u8::from(entry.allocated.is_some())])?;
        if let Some(n) = entry.allocated {
            w.write_all(&n.to_le_bytes())?;
        }
        w.write_all(&entry.modified.to_le_bytes())?;
        w.write_all(&entry.files.to_le_bytes())?;
        w.write_all(&[u8::from(entry.coverage)])?;
        w.write_all(&entry.attributes.to_le_bytes())?;
        w.write_all(&[entry.category, entry.issue])?;
    }
    w.flush()?;
    Ok(())
}
pub fn read_index(stream: &mut impl Read, expected_root: &str) -> Result<LiveIndex> {
    read_index_with_limits(
        stream,
        expected_root,
        MAXIMUM_BYTES,
        MAXIMUM_ENTRIES,
        MAXIMUM_RESIDENT_BYTES,
    )
}
pub fn read_index_with_limits(
    stream: &mut impl Read,
    expected_root: &str,
    maximum_bytes: usize,
    maximum_entries: usize,
    maximum_resident: usize,
) -> Result<LiveIndex> {
    limits(maximum_bytes)?;
    validate_limits(maximum_entries, maximum_resident)?;
    validate_root(expected_root)?;
    let mut r = Budget {
        inner: stream,
        remaining: maximum_bytes,
    };
    let mut magic = [0; 8];
    r.read_exact(&mut magic)?;
    ensure!(&magic == b"DBRUSTI2", "Unknown helper index frame");
    let root = read_text(&mut r, 32767)?;
    ensure!(root == expected_root, "Helper returned another root");
    let count = read_u32(&mut r)? as usize;
    ensure!(
        count > 0 && count <= maximum_entries,
        "Helper entry count exceeds its limit"
    );
    let mut entries: Vec<LiveEntry> = Vec::new();
    let mut resident = 0usize;
    for i in 0..count {
        let raw_parent = read_i64(&mut r)?;
        let parent = if i == 0 {
            ensure!(raw_parent == -1, "Invalid root parent");
            None
        } else {
            ensure!(
                raw_parent >= 0 && (raw_parent as u64) < i as u64,
                "Invalid metadata parent"
            );
            let parent = raw_parent as usize;
            ensure!(entries[parent].directory, "A file cannot contain children");
            Some(parent)
        };
        let name = read_text(&mut r, 255)?;
        if i == 0 {
            ensure!(name.is_empty(), "Invalid root name");
        } else {
            validate_name(&name)?;
        }
        resident = resident
            .checked_add(entry_reserve(name.len()) + 16)
            .ok_or_else(|| anyhow::anyhow!("Resident budget overflow"))?;
        ensure!(
            resident <= maximum_resident,
            "Helper resident budget exhausted"
        );
        let directory = read_bool(&mut r)?;
        let logical = read_i64(&mut r)?;
        let allocated = if read_bool(&mut r)? {
            Some(read_i64(&mut r)?)
        } else {
            None
        };
        let modified = read_i64(&mut r)?;
        let files = read_i64(&mut r)?;
        let coverage = read_bool(&mut r)?;
        let attributes = read_u32(&mut r)?;
        let category = read_u8(&mut r)?;
        let issue = read_u8(&mut r)?;
        ensure!(
            logical >= 0
                && allocated.is_none_or(|n| n >= 0)
                && files >= 0
                && category <= 8
                && issue <= 4,
            "Invalid metadata quantity"
        );
        ensure!(
            chrono::DateTime::<chrono::Utc>::from_timestamp(modified, 0).is_some(),
            "Invalid metadata time"
        );
        if let Some(parent) = parent {
            entries[parent].children.push(i);
        }
        entries.push(LiveEntry {
            parent,
            name: name.into_boxed_str(),
            directory,
            logical,
            allocated,
            modified,
            files,
            coverage,
            attributes,
            children: Vec::new(),
            category,
            issue,
        });
    }
    let index = LiveIndex { root, entries };
    index.validate(maximum_entries, maximum_resident)?;
    Ok(index)
}
