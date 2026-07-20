//! Guest image metadata and boot contract for Linux/ARM64.

pub const IMAGE_IPA_BASE: u64 = 0x4020_0000;
pub const IMAGE_SIZE: u64 = 0x0400_0000;
pub const DTB_IPA: u64 = 0x4900_0000;

pub fn validate_boot_contract(image: &[u8]) -> bool {
    image.len() >= 0x40 && image[0x38..0x3c] == *b"ARMd"
}
