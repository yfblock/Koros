//! ext2 extended-attribute (xattr) block encoding.
//!
//! Handles the on-disk format of the *external* attribute block referenced by
//! an inode's `i_file_acl` field:
//!
//! ```text
//! [ ext2_xattr_header (32 bytes) ]
//! [ entry, entry, … , 0-terminator ]   (grows up from offset 32)
//!            … free space …
//! [ … values … ]                       (packed down from the block end)
//! ```
//!
//! Each entry stores a `(name_index, name)` key and the value is kept inline
//! in the same block (`e_value_block == 0`).  Per-entry and whole-block hashes
//! are computed exactly as Linux does so `e2fsck` accepts the block.
//!
//! Attributes stored *inline* inside a large inode's extra space are a
//! separate mechanism and are not handled here.

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use kor::FsError;

/// Magic number at the start of an xattr block header.
pub const XATTR_MAGIC: u32 = 0xEA02_0000;
/// Size of the `ext2_xattr_header`.
const HEADER_SIZE: usize = 32;
/// Fixed size of an `ext2_xattr_entry` (before the trailing name).
const ENTRY_HEADER_SIZE: usize = 16;
/// On-disk alignment for entries and values.
const XATTR_PAD: usize = 4;

const NAME_HASH_SHIFT: u32 = 5;
const VALUE_HASH_SHIFT: u32 = 16;
const BLOCK_HASH_SHIFT: u32 = 16;

// Attribute name-index prefixes.
const IDX_USER: u8 = 1;
const IDX_ACL_ACCESS: u8 = 2;
const IDX_ACL_DEFAULT: u8 = 3;
const IDX_TRUSTED: u8 = 4;
const IDX_SECURITY: u8 = 6;
const IDX_SYSTEM: u8 = 7;

/// A decoded extended attribute: a `(name_index, name)` key and its value.
pub struct Attr {
    pub index: u8,
    pub name: Vec<u8>,
    pub value: Vec<u8>,
}

/// Padded on-disk length of an entry whose name is `name_len` bytes.
fn entry_len(name_len: usize) -> usize {
    (ENTRY_HEADER_SIZE + name_len + XATTR_PAD - 1) & !(XATTR_PAD - 1)
}

/// Split a full attribute name (e.g. `"user.foo"`) into its `(name_index,
/// suffix)` on-disk form.  Returns `None` for an unknown or empty namespace.
pub fn split_name(full: &str) -> Option<(u8, &str)> {
    if let Some(s) = full.strip_prefix("user.") {
        return (!s.is_empty()).then_some((IDX_USER, s));
    }
    if full == "system.posix_acl_access" {
        return Some((IDX_ACL_ACCESS, ""));
    }
    if full == "system.posix_acl_default" {
        return Some((IDX_ACL_DEFAULT, ""));
    }
    if let Some(s) = full.strip_prefix("trusted.") {
        return (!s.is_empty()).then_some((IDX_TRUSTED, s));
    }
    if let Some(s) = full.strip_prefix("security.") {
        return (!s.is_empty()).then_some((IDX_SECURITY, s));
    }
    if let Some(s) = full.strip_prefix("system.") {
        return (!s.is_empty()).then_some((IDX_SYSTEM, s));
    }
    None
}

/// Reconstruct a full attribute name from its `(name_index, suffix)` form.
pub fn full_name(index: u8, suffix: &[u8]) -> Option<String> {
    let prefix = match index {
        IDX_USER => "user.",
        IDX_ACL_ACCESS => return Some(String::from("system.posix_acl_access")),
        IDX_ACL_DEFAULT => return Some(String::from("system.posix_acl_default")),
        IDX_TRUSTED => "trusted.",
        IDX_SECURITY => "security.",
        IDX_SYSTEM => "system.",
        _ => return None,
    };
    let mut s = String::from(prefix);
    s.push_str(core::str::from_utf8(suffix).ok()?);
    Some(s)
}

/// Read the reference count from an xattr block header.
pub fn refcount(buf: &[u8]) -> u32 {
    u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]])
}

/// Overwrite the reference count in an xattr block header.
pub fn set_refcount(buf: &mut [u8], value: u32) {
    buf[4..8].copy_from_slice(&value.to_le_bytes());
}

/// Parse every attribute in an external xattr block.
pub fn parse_block(buf: &[u8]) -> Result<Vec<Attr>, FsError> {
    if buf.len() < HEADER_SIZE {
        return Err(FsError::InvalidInput);
    }
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != XATTR_MAGIC {
        return Err(FsError::InvalidInput);
    }

    let mut out = Vec::new();
    let mut off = HEADER_SIZE;
    loop {
        if off + ENTRY_HEADER_SIZE > buf.len() {
            break;
        }
        let name_len = buf[off] as usize;
        if name_len == 0 {
            break; // zero entry terminates the list
        }
        let index = buf[off + 1];
        let value_offs = u16::from_le_bytes([buf[off + 2], buf[off + 3]]) as usize;
        let value_block = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let value_size =
            u32::from_le_bytes([buf[off + 8], buf[off + 9], buf[off + 10], buf[off + 11]]) as usize;

        // Values stored in a separate block are not supported.
        if value_block != 0 {
            return Err(FsError::Unsupported);
        }
        let name_start = off + ENTRY_HEADER_SIZE;
        if name_start + name_len > buf.len() || value_offs + value_size > buf.len() {
            return Err(FsError::InvalidInput);
        }

        let name = buf[name_start..name_start + name_len].to_vec();
        let value = buf[value_offs..value_offs + value_size].to_vec();
        out.push(Attr { index, name, value });
        off += entry_len(name_len);
    }
    Ok(out)
}

/// Serialize `attrs` into a fresh block image of `block_size` bytes (refcount
/// 1), matching Linux's layout and hashes.  Returns [`FsError::NoSpace`] if
/// the attributes do not fit.
pub fn serialize_block(attrs: &mut [Attr], block_size: usize) -> Result<Vec<u8>, FsError> {
    // Linux keeps entries sorted by (name_index, name); mirror that so the
    // block hash we compute matches the stored order.
    attrs.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.name.cmp(&b.name)));

    let mut buf = vec![0u8; block_size];
    buf[0..4].copy_from_slice(&XATTR_MAGIC.to_le_bytes());
    buf[4..8].copy_from_slice(&1u32.to_le_bytes()); // h_refcount
    buf[8..12].copy_from_slice(&1u32.to_le_bytes()); // h_blocks

    let mut entry_off = HEADER_SIZE;
    let mut value_end = block_size;
    let mut entry_hashes = Vec::with_capacity(attrs.len());

    for a in attrs.iter() {
        let name_len = a.name.len();
        let elen = entry_len(name_len);
        let vlen = a.value.len();
        let vpad = (vlen + XATTR_PAD - 1) & !(XATTR_PAD - 1);

        let value_offs = if vlen == 0 {
            0
        } else {
            if value_end < HEADER_SIZE + vpad {
                return Err(FsError::NoSpace);
            }
            value_end -= vpad;
            value_end
        };

        // Entries grow up, values grow down; keep room for the 4-byte
        // terminator too.
        if entry_off + elen + XATTR_PAD > value_end {
            return Err(FsError::NoSpace);
        }

        if vlen != 0 {
            buf[value_offs..value_offs + vlen].copy_from_slice(&a.value);
        }

        let e_hash = hash_entry(&a.name, &a.value);
        entry_hashes.push(e_hash);

        buf[entry_off] = name_len as u8;
        buf[entry_off + 1] = a.index;
        buf[entry_off + 2..entry_off + 4].copy_from_slice(&(value_offs as u16).to_le_bytes());
        // e_value_block stays 0.
        buf[entry_off + 8..entry_off + 12].copy_from_slice(&(vlen as u32).to_le_bytes());
        buf[entry_off + 12..entry_off + 16].copy_from_slice(&e_hash.to_le_bytes());
        buf[entry_off + ENTRY_HEADER_SIZE..entry_off + ENTRY_HEADER_SIZE + name_len]
            .copy_from_slice(&a.name);
        entry_off += elen;
    }

    // Whole-block hash (matches ext2_xattr_rehash).
    let mut h = 0u32;
    let mut any_zero = false;
    for &eh in &entry_hashes {
        if eh == 0 {
            any_zero = true;
            break;
        }
        h = (h << BLOCK_HASH_SHIFT) ^ (h >> (32 - BLOCK_HASH_SHIFT)) ^ eh;
    }
    let block_hash = if any_zero { 0 } else { h };
    buf[12..16].copy_from_slice(&block_hash.to_le_bytes());

    Ok(buf)
}

/// Per-entry hash of name and (inline) value — mirrors `ext2_xattr_hash_entry`.
fn hash_entry(name: &[u8], value: &[u8]) -> u32 {
    let mut hash = 0u32;
    for &c in name {
        hash = (hash << NAME_HASH_SHIFT) ^ (hash >> (32 - NAME_HASH_SHIFT)) ^ (c as u32);
    }
    if !value.is_empty() {
        let words = value.len().div_ceil(4);
        for i in 0..words {
            let start = i * 4;
            let end = core::cmp::min(start + 4, value.len());
            let mut word = [0u8; 4];
            word[..end - start].copy_from_slice(&value[start..end]);
            let v = u32::from_le_bytes(word);
            hash = (hash << VALUE_HASH_SHIFT) ^ (hash >> (32 - VALUE_HASH_SHIFT)) ^ v;
        }
    }
    hash
}
