use num_bigint::BigUint;
use num_traits::Zero;

use crate::error::{AppError, Result};

/// Reproduce the portal's RSAUtils.encryptedString for UTF-16 code units.
///
/// The portal concatenates `password>mac`, reverses UTF-16 units, packs them
/// little-endian in chunks of `2 * (modulus_digits - 1)` units, then applies
/// raw RSA. The resulting hex blocks are space-separated.
pub fn rsa_encrypt(
    password: &str,
    mac: &str,
    exponent_hex: &str,
    modulus_hex: &str,
) -> Result<String> {
    let invalid_key = || AppError::msg("认证页返回了无效的 RSA 公钥");

    let exponent = BigUint::parse_bytes(exponent_hex.as_bytes(), 16).ok_or_else(invalid_key)?;
    let modulus = BigUint::parse_bytes(modulus_hex.as_bytes(), 16).ok_or_else(invalid_key)?;

    let digits = modulus.bits().div_ceil(16);
    let chunk_size = digits.saturating_sub(1) * 2;
    if exponent.is_zero() || modulus.is_zero() || chunk_size == 0 {
        return Err(invalid_key());
    }
    let chunk_size = usize::try_from(chunk_size).map_err(|_| invalid_key())?;

    let combined = format!("{password}>{mac}");
    let mut units: Vec<u16> = combined.encode_utf16().collect();
    units.reverse();

    let mut blocks = Vec::new();
    for block in units.chunks(chunk_size) {
        let mut padded = block.to_vec();
        padded.resize(chunk_size, 0);

        // This mirrors the portal's JavaScript arithmetic exactly. Each pair
        // of UTF-16 units forms a 32-bit digit, shifted by 16 bits per pair.
        let mut value = BigUint::from(0u8);
        for index in (0..chunk_size).step_by(2) {
            let pair = u32::from(padded[index]) + (u32::from(padded[index + 1]) << 8);
            value += BigUint::from(pair) << (8 * index);
        }
        let encrypted = value.modpow(&exponent, &modulus);
        let raw_hex = encrypted.to_str_radix(16);
        let width = std::cmp::max(4, raw_hex.len().div_ceil(4) * 4);
        blocks.push(format!("{raw_hex:0>width$}"));
    }

    Ok(blocks.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_portal_little_endian_blocks() {
        assert_eq!(
            rsa_encrypt("abc", "12", "01", "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF").unwrap(),
            "6162633e3132"
        );
    }

    #[test]
    fn rejects_malformed_public_key() {
        assert!(rsa_encrypt("secret", "mac", "", "bad-key").is_err());
        assert!(rsa_encrypt("secret", "mac", "01", "00").is_err());
    }

    #[test]
    fn pads_each_block_to_a_hex_boundary() {
        let value = rsa_encrypt(
            "abc",
            "12",
            "01",
            "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        )
        .unwrap();
        assert!(value.starts_with("6162633e3132"));
        assert_eq!(value.len() % 4, 0);
    }
}
