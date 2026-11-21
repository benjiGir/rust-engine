//! Les trames : `[longueur varint][drapeau][données]`.
//!
//! La longueur compte le drapeau et les données. Le drapeau dit si les données sont compressées
//! (lz4, au-delà de [`COMPRESS_OVER`] octets, et seulement si c'est plus court). Une trame est
//! encodée une fois en `Arc<[u8]>` : le même chunk part à tous les joueurs sans être réencodé.

use std::sync::Arc;

/// Au-delà, les données sont compressées.
pub const COMPRESS_OVER: usize = 256;
/// La plus grande trame acceptée (drapeau et données) : au-delà, le pair se trompe ou triche.
pub const MAX_FRAME: usize = 16 << 20;

const RAW: u8 = 0;
const LZ4: u8 = 1;

/// Une trame prête à partir, partageable entre connexions.
pub type Frame = Arc<[u8]>;

/// Encode des données en trame.
pub fn encode(payload: &[u8]) -> Frame {
    let compressed = (payload.len() > COMPRESS_OVER)
        .then(|| lz4_flex::compress_prepend_size(payload))
        .filter(|c| c.len() < payload.len());
    let (flag, body) = match &compressed {
        Some(c) => (LZ4, c.as_slice()),
        None => (RAW, payload),
    };
    let mut out = Vec::with_capacity(body.len() + 6);
    write_varint(&mut out, body.len() as u32 + 1);
    out.push(flag);
    out.extend_from_slice(body);
    out.into()
}

fn write_varint(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Lit une longueur varint au début de `bytes` : (valeur, octets lus), `None` s'il en manque.
fn read_varint(bytes: &[u8]) -> Result<Option<(usize, usize)>, FrameError> {
    let mut value = 0usize;
    for (i, &byte) in bytes.iter().enumerate().take(5) {
        value |= usize::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(Some((value, i + 1)));
        }
    }
    if bytes.len() >= 5 {
        return Err(FrameError::BadLength);
    }
    Ok(None)
}

/// Une trame illisible : la connexion est fermée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    BadLength,
    TooLarge(usize),
    BadFlag(u8),
    BadCompression,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadLength => write!(f, "longueur de trame illisible"),
            Self::TooLarge(n) => write!(f, "trame trop grande ({n} octets)"),
            Self::BadFlag(flag) => write!(f, "drapeau de trame inconnu ({flag})"),
            Self::BadCompression => write!(f, "données compressées illisibles"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Recolle les trames d'un flux d'octets arrivés par morceaux.
#[derive(Debug, Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
}

impl FrameReader {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// La prochaine trame complète, décompressée.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        let Some((len, header)) = read_varint(&self.buffer)? else {
            return Ok(None);
        };
        if len == 0 {
            return Err(FrameError::BadLength);
        }
        if len > MAX_FRAME {
            return Err(FrameError::TooLarge(len));
        }
        if self.buffer.len() < header + len {
            return Ok(None);
        }
        let flag = self.buffer[header];
        let body = &self.buffer[header + 1..header + len];
        let payload = match flag {
            RAW => body.to_vec(),
            LZ4 => {
                lz4_flex::decompress_size_prepended(body).map_err(|_| FrameError::BadCompression)?
            }
            other => return Err(FrameError::BadFlag(other)),
        };
        if payload.len() > MAX_FRAME {
            return Err(FrameError::TooLarge(payload.len()));
        }
        self.buffer.drain(..header + len);
        Ok(Some(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut reader = FrameReader::default();
        reader.push(bytes);
        let mut out = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            out.push(frame);
        }
        out
    }

    #[test]
    fn small_frames_stay_raw_and_large_ones_are_compressed() {
        let small = encode(b"bonjour");
        assert_eq!(small[1], RAW);
        let large_payload = vec![7u8; 10_000];
        let large = encode(&large_payload);
        assert_eq!(large[1], LZ4, "compressée, la longueur tient sur un octet");
        assert!(large.len() < 200);
        let mut stream = small.to_vec();
        stream.extend_from_slice(&large);
        assert_eq!(read_all(&stream), vec![b"bonjour".to_vec(), large_payload]);
    }

    #[test]
    fn frames_arriving_byte_by_byte_are_put_back_together() {
        let payload: Vec<u8> = (0..1000u32).map(|i| (i * 31 % 251) as u8).collect();
        let frame = encode(&payload);
        let mut reader = FrameReader::default();
        for (i, byte) in frame.iter().enumerate() {
            reader.push(&[*byte]);
            let got = reader.next_frame().unwrap();
            assert_eq!(got.is_some(), i == frame.len() - 1);
            if let Some(got) = got {
                assert_eq!(got, payload);
            }
        }
    }

    #[test]
    fn incompressible_data_is_sent_raw_and_bad_frames_are_refused() {
        let noise: Vec<u8> = (0..600u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        assert_eq!(read_all(&encode(&noise)), vec![noise]);
        let mut reader = FrameReader::default();
        reader.push(&[2, 9, 0]);
        assert_eq!(reader.next_frame(), Err(FrameError::BadFlag(9)));
        let mut reader = FrameReader::default();
        let mut huge = Vec::new();
        write_varint(&mut huge, (MAX_FRAME + 1) as u32);
        reader.push(&huge);
        assert_eq!(
            reader.next_frame(),
            Err(FrameError::TooLarge(MAX_FRAME + 1))
        );
        let mut reader = FrameReader::default();
        reader.push(&[0xff; 6]);
        assert_eq!(reader.next_frame(), Err(FrameError::BadLength));
    }
}
