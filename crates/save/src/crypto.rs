//! Echoes of Aincrad save encryption: AES-256-ECB with the same key that
//! encrypts the game's paks. Decrypting the `.sav` yields a standard UE5 GVAS
//! SaveGame; re-encrypting the (re-serialized) plaintext is byte-exact for any
//! block we didn't touch, so edits are surgical.
//!
//! The save length is always a multiple of 16 (no separate padding scheme).

use crate::SaveError;
use aes::cipher::{generic_array::GenericArray, BlockDecryptMut, BlockEncryptMut, KeyInit};

/// Parse a `0x…`/bare hex AES-256 key into 32 bytes.
pub fn parse_key(hex_key: &str) -> Result<[u8; 32], SaveError> {
    let h = hex_key.trim().trim_start_matches("0x");
    let bytes = hex::decode(h).map_err(|_| SaveError::BadKey)?;
    bytes.try_into().map_err(|_| SaveError::BadKey)
}

/// Decrypt an encrypted save blob in place-free fashion (returns a new Vec).
pub fn decrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, SaveError> {
    if data.is_empty() || !data.len().is_multiple_of(16) {
        return Err(SaveError::BadLength(data.len()));
    }
    let mut cipher = ecb::Decryptor::<aes::Aes256>::new(key.into());
    let mut out = data.to_vec();
    for chunk in out.chunks_mut(16) {
        cipher.decrypt_block_mut(GenericArray::from_mut_slice(chunk));
    }
    Ok(out)
}

/// Encrypt a plaintext GVAS blob. Length must already be a multiple of 16
/// (uesave re-serialization preserves the original 16-aligned length).
pub fn encrypt(key: &[u8; 32], data: &[u8]) -> Result<Vec<u8>, SaveError> {
    if !data.len().is_multiple_of(16) {
        return Err(SaveError::BadLength(data.len()));
    }
    let mut cipher = ecb::Encryptor::<aes::Aes256>::new(key.into());
    let mut out = data.to_vec();
    for chunk in out.chunks_mut(16) {
        cipher.encrypt_block_mut(GenericArray::from_mut_slice(chunk));
    }
    Ok(out)
}
