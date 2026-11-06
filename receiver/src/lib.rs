//! Encrypted, framed keyboard-event protocol shared with the macOS sender.
//! Packet contents are never logged or written to disk.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use rand::RngCore;
use std::io::{self, Read, Write};
use thiserror::Error;

pub const MAX_FRAME_SIZE: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Packet {
    Key { code: u16, value: u8 },
    ReleaseAll,
    Ping,
}

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error("pairing secret must be base64-encoded 32 bytes")]
    InvalidPairingSecret,
    #[error("invalid packet")]
    InvalidPacket,
    #[error("invalid or oversized frame")]
    InvalidFrame,
    #[error("authentication failed")]
    Authentication,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        match *self {
            Self::Key { code, value } => vec![1, (code >> 8) as u8, code as u8, value],
            Self::ReleaseAll => vec![2],
            Self::Ping => vec![3],
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BridgeError> {
        match bytes {
            [1, high, low, value @ 0..=2] => Ok(Self::Key { code: u16::from_be_bytes([*high, *low]), value: *value }),
            [2] => Ok(Self::ReleaseAll),
            [3] => Ok(Self::Ping),
            _ => Err(BridgeError::InvalidPacket),
        }
    }
}

#[derive(Clone)]
pub struct PairingSecret([u8; 32]);

impl PairingSecret {
    pub fn parse(base64: &str) -> Result<Self, BridgeError> {
        let decoded = STANDARD.decode(base64).map_err(|_| BridgeError::InvalidPairingSecret)?;
        let key: [u8; 32] = decoded.try_into().map_err(|_| BridgeError::InvalidPairingSecret)?;
        Ok(Self(key))
    }

    pub fn generate() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    pub fn to_base64(&self) -> String { STANDARD.encode(self.0) }
}

pub struct SecureFramer { cipher: ChaCha20Poly1305 }

impl SecureFramer {
    pub fn new(secret: PairingSecret) -> Self { Self { cipher: ChaCha20Poly1305::new((&secret.0).into()) } }

    /// Returns a TCP frame with a 4-byte big-endian size followed by
    /// nonce (12 bytes), ciphertext, and tag (16 bytes), matching CryptoKit's combined box.
    pub fn seal(&self, packet: &Packet) -> Result<Vec<u8>, BridgeError> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut combined = nonce.to_vec();
        let ciphertext = self.cipher.encrypt(Nonce::from_slice(&nonce), packet.encode().as_ref())
            .map_err(|_| BridgeError::Authentication)?;
        combined.extend(ciphertext);
        if combined.len() > MAX_FRAME_SIZE { return Err(BridgeError::InvalidFrame); }
        let mut frame = (combined.len() as u32).to_be_bytes().to_vec();
        frame.extend(combined);
        Ok(frame)
    }

    pub fn open(&self, combined: &[u8]) -> Result<Packet, BridgeError> {
        if !(12 + 16..=MAX_FRAME_SIZE).contains(&combined.len()) { return Err(BridgeError::InvalidFrame); }
        let (nonce, ciphertext) = combined.split_at(12);
        let plaintext = self.cipher.decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| BridgeError::Authentication)?;
        Packet::decode(&plaintext)
    }

    pub fn read_packet<R: Read>(&self, reader: &mut R) -> Result<Packet, BridgeError> {
        let mut length = [0u8; 4];
        reader.read_exact(&mut length)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_FRAME_SIZE { return Err(BridgeError::InvalidFrame); }
        let mut combined = vec![0; length];
        reader.read_exact(&mut combined)?;
        self.open(&combined)
    }

    pub fn write_packet<W: Write>(&self, writer: &mut W, packet: &Packet) -> Result<(), BridgeError> {
        writer.write_all(&self.seal(packet)?)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> PairingSecret { PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap() }

    #[test]
    fn packet_round_trip() {
        let framer = SecureFramer::new(secret());
        let frame = framer.seal(&Packet::Key { code: 30, value: 1 }).unwrap();
        assert_eq!(framer.open(&frame[4..]).unwrap(), Packet::Key { code: 30, value: 1 });
    }

    #[test]
    fn length_prefix_detects_bad_frame() {
        let framer = SecureFramer::new(secret());
        let mut input = vec![0, 0, 0x20, 0x01];
        input.extend([0; 3]);
        assert!(matches!(framer.read_packet(&mut input.as_slice()), Err(BridgeError::InvalidFrame)));
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let framer = SecureFramer::new(secret());
        let mut frame = framer.seal(&Packet::Ping).unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 1;
        assert!(matches!(framer.open(&frame[4..]), Err(BridgeError::Authentication)));
    }
}
