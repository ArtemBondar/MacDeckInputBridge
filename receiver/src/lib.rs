//! Encrypted, framed keyboard-event protocol shared with the macOS sender.
//! Packet contents are never logged or written to disk.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    ChaCha20Poly1305, Nonce,
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;
use std::io::{self, Read, Write};
use thiserror::Error;

pub const MAX_FRAME_SIZE: usize = 4096;
pub const SERVER_HELLO_TYPE: u8 = 0x10;
pub const PROTOCOL_VERSION: u8 = 0x01;
pub const SESSION_INFO: &[u8] = b"MacDeckInputBridge/session/v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Packet {
    Key { code: u16, value: u8 },
    ReleaseAll,
    Ping,
    ClientReady,
    ServerReady,
    ServerReadyWithLayoutSync,
    KeyboardLayout(String),
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
    #[error("invalid handshake or sequence")]
    Handshake,
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl Packet {
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Key { code, value } => vec![1, (*code >> 8) as u8, *code as u8, *value],
            Self::ReleaseAll => vec![2],
            Self::Ping => vec![3],
            Self::ClientReady => vec![4],
            Self::ServerReady => vec![5],
            Self::ServerReadyWithLayoutSync => vec![6],
            Self::KeyboardLayout(identifier) => {
                let bytes = identifier.as_bytes();
                debug_assert!((1..=128).contains(&bytes.len()));
                let mut encoded = Vec::with_capacity(bytes.len() + 2);
                encoded.extend([7, bytes.len() as u8]);
                encoded.extend(bytes);
                encoded
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, BridgeError> {
        match bytes {
            [1, high, low, value @ 0..=2] => Ok(Self::Key {
                code: u16::from_be_bytes([*high, *low]),
                value: *value,
            }),
            [2] => Ok(Self::ReleaseAll),
            [3] => Ok(Self::Ping),
            [4] => Ok(Self::ClientReady),
            [5] => Ok(Self::ServerReady),
            [6] => Ok(Self::ServerReadyWithLayoutSync),
            [7, length, identifier @ ..]
                if (1..=128).contains(length)
                    && identifier.len() == usize::from(*length)
                    && identifier.iter().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                    }) =>
            {
                Ok(Self::KeyboardLayout(
                    String::from_utf8(identifier.to_vec())
                        .map_err(|_| BridgeError::InvalidPacket)?,
                ))
            }
            _ => Err(BridgeError::InvalidPacket),
        }
    }
}

#[derive(Clone)]
pub struct PairingSecret([u8; 32]);

impl PairingSecret {
    pub fn parse(base64: &str) -> Result<Self, BridgeError> {
        let decoded = STANDARD
            .decode(base64)
            .map_err(|_| BridgeError::InvalidPairingSecret)?;
        let key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| BridgeError::InvalidPairingSecret)?;
        Ok(Self(key))
    }

    pub fn generate() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self(key)
    }

    pub fn to_base64(&self) -> String {
        STANDARD.encode(self.0)
    }
}

/// Derives a one-connection key from the long-lived pairing key and a fresh
/// receiver challenge. Reusing an encrypted frame on another connection then
/// fails authentication because its session key is different.
pub fn derive_session_secret(pairing: &PairingSecret, challenge: &[u8; 32]) -> PairingSecret {
    let hkdf = Hkdf::<Sha256>::new(Some(challenge), &pairing.0);
    let mut key = [0u8; 32];
    hkdf.expand(SESSION_INFO, &mut key)
        .expect("fixed-length HKDF output is valid");
    PairingSecret(key)
}

pub fn generate_challenge() -> [u8; 32] {
    let mut challenge = [0u8; 32];
    OsRng.fill_bytes(&mut challenge);
    challenge
}

pub fn write_server_hello<W: Write>(
    writer: &mut W,
    challenge: &[u8; 32],
) -> Result<(), BridgeError> {
    let mut hello = Vec::with_capacity(34);
    hello.push(SERVER_HELLO_TYPE);
    hello.push(PROTOCOL_VERSION);
    hello.extend(challenge);
    write_plain_frame(writer, &hello)
}

pub fn read_server_hello<R: Read>(reader: &mut R) -> Result<[u8; 32], BridgeError> {
    let hello = read_plain_frame(reader)?;
    let [kind, version, challenge @ ..] = hello.as_slice() else {
        return Err(BridgeError::Handshake);
    };
    if *kind != SERVER_HELLO_TYPE || *version != PROTOCOL_VERSION || challenge.len() != 32 {
        return Err(BridgeError::Handshake);
    }
    challenge.try_into().map_err(|_| BridgeError::Handshake)
}

pub fn write_plain_frame<W: Write>(writer: &mut W, bytes: &[u8]) -> Result<(), BridgeError> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME_SIZE {
        return Err(BridgeError::InvalidFrame);
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

pub fn read_plain_frame<R: Read>(reader: &mut R) -> Result<Vec<u8>, BridgeError> {
    let mut length = [0u8; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME_SIZE {
        return Err(BridgeError::InvalidFrame);
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub struct SecureFramer {
    cipher: ChaCha20Poly1305,
}

impl SecureFramer {
    pub fn new(secret: PairingSecret) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new((&secret.0).into()),
        }
    }

    /// Returns a TCP frame with a 4-byte big-endian size followed by
    /// nonce (12 bytes), ciphertext, and tag (16 bytes), matching CryptoKit's combined box.
    pub fn seal(&self, packet: &Packet) -> Result<Vec<u8>, BridgeError> {
        self.seal_plaintext(&packet.encode())
    }

    fn seal_plaintext(&self, plaintext: &[u8]) -> Result<Vec<u8>, BridgeError> {
        let mut nonce = [0u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let mut combined = nonce.to_vec();
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .map_err(|_| BridgeError::Authentication)?;
        combined.extend(ciphertext);
        if combined.len() > MAX_FRAME_SIZE {
            return Err(BridgeError::InvalidFrame);
        }
        let mut frame = (combined.len() as u32).to_be_bytes().to_vec();
        frame.extend(combined);
        Ok(frame)
    }

    pub fn open(&self, combined: &[u8]) -> Result<Packet, BridgeError> {
        Packet::decode(&self.open_plaintext(combined)?)
    }

    fn open_plaintext(&self, combined: &[u8]) -> Result<Vec<u8>, BridgeError> {
        if !(12 + 16..=MAX_FRAME_SIZE).contains(&combined.len()) {
            return Err(BridgeError::InvalidFrame);
        }
        let (nonce, ciphertext) = combined.split_at(12);
        let plaintext = self
            .cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| BridgeError::Authentication)?;
        Ok(plaintext)
    }

    pub fn read_packet<R: Read>(&self, reader: &mut R) -> Result<Packet, BridgeError> {
        self.open(&read_plain_frame(reader)?)
    }

    pub fn write_packet<W: Write>(
        &self,
        writer: &mut W,
        packet: &Packet,
    ) -> Result<(), BridgeError> {
        writer.write_all(&self.seal(packet)?)?;
        writer.flush()?;
        Ok(())
    }
}

/// Session records bind every packet to a freshly-derived per-connection key
/// and include an authenticated, strictly increasing sequence number.
pub struct SessionFramer {
    framer: SecureFramer,
}

impl SessionFramer {
    pub fn new(session_secret: PairingSecret) -> Self {
        Self {
            framer: SecureFramer::new(session_secret),
        }
    }

    pub fn seal(&self, sequence: u64, packet: &Packet) -> Result<Vec<u8>, BridgeError> {
        let mut plaintext = sequence.to_be_bytes().to_vec();
        plaintext.extend(packet.encode());
        self.framer.seal_plaintext(&plaintext)
    }

    pub fn open(&self, combined: &[u8]) -> Result<(u64, Packet), BridgeError> {
        let plaintext = self.framer.open_plaintext(combined)?;
        if plaintext.len() < 9 {
            return Err(BridgeError::InvalidPacket);
        }
        let sequence = u64::from_be_bytes(
            plaintext[..8]
                .try_into()
                .map_err(|_| BridgeError::InvalidPacket)?,
        );
        Ok((sequence, Packet::decode(&plaintext[8..])?))
    }

    pub fn read_packet<R: Read>(&self, reader: &mut R) -> Result<(u64, Packet), BridgeError> {
        self.open(&read_plain_frame(reader)?)
    }

    pub fn write_packet<W: Write>(
        &self,
        writer: &mut W,
        sequence: u64,
        packet: &Packet,
    ) -> Result<(), BridgeError> {
        writer.write_all(&self.seal(sequence, packet)?)?;
        writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> PairingSecret {
        PairingSecret::parse("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap()
    }

    #[test]
    fn packet_round_trip() {
        let framer = SecureFramer::new(secret());
        let frame = framer.seal(&Packet::Key { code: 30, value: 1 }).unwrap();
        assert_eq!(
            framer.open(&frame[4..]).unwrap(),
            Packet::Key { code: 30, value: 1 }
        );
    }

    #[test]
    fn layout_identifier_packet_accepts_only_bounded_safe_identifiers() {
        let packet = Packet::KeyboardLayout("com.apple.keylayout.Ukrainian".to_owned());
        assert_eq!(Packet::decode(&packet.encode()).unwrap(), packet);
        assert!(matches!(
            Packet::decode(&[7, 2, b'u', b' ']),
            Err(BridgeError::InvalidPacket)
        ));
    }

    #[test]
    fn length_prefix_detects_bad_frame() {
        let framer = SecureFramer::new(secret());
        let mut input = vec![0, 0, 0x20, 0x01];
        input.extend([0; 3]);
        assert!(matches!(
            framer.read_packet(&mut input.as_slice()),
            Err(BridgeError::InvalidFrame)
        ));
    }

    #[test]
    fn rejects_tampered_ciphertext() {
        let framer = SecureFramer::new(secret());
        let mut frame = framer.seal(&Packet::Ping).unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 1;
        assert!(matches!(
            framer.open(&frame[4..]),
            Err(BridgeError::Authentication)
        ));
    }

    #[test]
    fn decrypts_deterministic_swift_interop_fixture() {
        let key = [0u8; 32];
        let nonce = [0u8; 12];
        let cipher = ChaCha20Poly1305::new((&key).into());
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Packet::Key { code: 30, value: 1 }.encode().as_ref(),
            )
            .unwrap();
        assert_eq!(
            hex::encode(&ciphertext),
            "9e07f9bf24604abcc6cbb670b4ca01b0ca60a51d"
        );
        let mut combined = nonce.to_vec();
        combined.extend(ciphertext);
        assert_eq!(
            SecureFramer::new(secret()).open(&combined).unwrap(),
            Packet::Key { code: 30, value: 1 }
        );
    }

    #[test]
    fn server_hello_is_plain_length_framed_and_strict() {
        let challenge = [7u8; 32];
        let mut bytes = Vec::new();
        write_server_hello(&mut bytes, &challenge).unwrap();
        assert_eq!(&bytes[..4], &(34u32).to_be_bytes());
        assert_eq!(&bytes[4..6], &[SERVER_HELLO_TYPE, PROTOCOL_VERSION]);
        assert_eq!(read_server_hello(&mut bytes.as_slice()).unwrap(), challenge);

        let invalid = vec![0, 0, 0, 2, SERVER_HELLO_TYPE, PROTOCOL_VERSION];
        assert!(matches!(
            read_server_hello(&mut invalid.as_slice()),
            Err(BridgeError::Handshake)
        ));
    }

    #[test]
    fn distinct_challenges_produce_distinct_session_keys() {
        let pairing = secret();
        let first = derive_session_secret(&pairing, &[1; 32]);
        let second = derive_session_secret(&pairing, &[2; 32]);
        let first_session = SessionFramer::new(first);
        let second_session = SessionFramer::new(second);
        let frame = first_session.seal(0, &Packet::ClientReady).unwrap();
        assert!(matches!(
            second_session.open(&frame[4..]),
            Err(BridgeError::Authentication)
        ));
    }

    #[test]
    fn session_record_round_trip_includes_sequence() {
        let session = SessionFramer::new(derive_session_secret(&secret(), &[9; 32]));
        let frame = session
            .seal(41, &Packet::Key { code: 30, value: 1 })
            .unwrap();
        assert_eq!(
            session.open(&frame[4..]).unwrap(),
            (41, Packet::Key { code: 30, value: 1 })
        );
    }
}
