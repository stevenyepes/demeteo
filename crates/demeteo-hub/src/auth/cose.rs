//! The one byte form of a passkey's public key: an ES256 `COSE_Key` in CTAP2
//! canonical encoding. Its base64url text ([`super::challenge`]) is the
//! `new_public_key` an endorsement challenge is computed over, so two
//! encodings of one key would be two challenges and a record nothing can
//! recompute. [`parse`] therefore accepts a single byte string per key, and no
//! CBOR decoder is involved: a decoder would also accept reordered labels,
//! non-minimal lengths and extra entries.
//!
//! P-256 only. A key of any other algorithm has no form here and is refused.

/// `a5 01 02 03 26 20 01 21 58 20 <x:32> 22 58 20 <y:32>`.
pub const COSE_KEY_LEN: usize = 77;

/// A five-entry map, `kty` (1) = EC2 (2), `alg` (3) = ES256 (-7), `crv` (-1) =
/// P-256 (1), then label `x` (-2) and the header of a 32-byte string.
const HEAD: [u8; 10] = [0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20];
/// Label `y` (-3) and the header of a 32-byte string.
const Y_HEAD: [u8; 3] = [0x22, 0x58, 0x20];

const SEC1_UNCOMPRESSED: u8 = 0x04;

/// The two affine coordinates, big-endian. Nothing here checks that they lie
/// on the curve: that is the signature verifier's refusal to make, when it is
/// handed [`P256Point::to_sec1_uncompressed`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P256Point {
    pub x: [u8; 32],
    pub y: [u8; 32],
}

impl P256Point {
    /// `04 ‖ x ‖ y`, the form `aws_lc_rs::signature` reads and writes.
    pub fn to_sec1_uncompressed(&self) -> [u8; 65] {
        let mut out = [0u8; 65];
        out[0] = SEC1_UNCOMPRESSED;
        out[1..33].copy_from_slice(&self.x);
        out[33..].copy_from_slice(&self.y);
        out
    }

    /// `None` for anything but the 65-byte uncompressed form; a compressed or
    /// hybrid point is not expanded.
    pub fn from_sec1_uncompressed(bytes: &[u8]) -> Option<Self> {
        let (tag, rest) = bytes.split_first()?;
        if *tag != SEC1_UNCOMPRESSED {
            return None;
        }
        let (x, y) = rest.split_first_chunk::<32>()?;
        Some(Self {
            x: *x,
            y: <[u8; 32]>::try_from(y).ok()?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CoseKeyError {
    #[error("a COSE_Key is {COSE_KEY_LEN} bytes, not {0}")]
    WrongLength(usize),
    #[error("not a canonical ES256 COSE_Key")]
    NotCanonicalEs256,
}

pub fn encode(point: &P256Point) -> [u8; COSE_KEY_LEN] {
    let mut out = [0u8; COSE_KEY_LEN];
    out[..10].copy_from_slice(&HEAD);
    out[10..42].copy_from_slice(&point.x);
    out[42..45].copy_from_slice(&Y_HEAD);
    out[45..].copy_from_slice(&point.y);
    out
}

/// Accepts exactly the bytes [`encode`] produces.
pub fn parse(bytes: &[u8]) -> Result<P256Point, CoseKeyError> {
    if bytes.len() != COSE_KEY_LEN {
        return Err(CoseKeyError::WrongLength(bytes.len()));
    }
    let coordinates = || {
        let (x, rest) = bytes.strip_prefix(&HEAD)?.split_first_chunk::<32>()?;
        let y = <[u8; 32]>::try_from(rest.strip_prefix(&Y_HEAD)?).ok()?;
        Some(P256Point { x: *x, y })
    };
    coordinates().ok_or(CoseKeyError::NotCanonicalEs256)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALG_OFFSET: usize = 4;
    const EDDSA: u8 = 0x27;

    fn point() -> P256Point {
        P256Point {
            x: std::array::from_fn(|i| 0x01 + i as u8),
            y: std::array::from_fn(|i| 0x41 + i as u8),
        }
    }

    #[test]
    fn a_key_encodes_to_the_ctap2_canonical_bytes() {
        let expected: Vec<u8> = [
            &[0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20][..],
            &point().x,
            &[0x22, 0x58, 0x20],
            &point().y,
        ]
        .concat();
        assert_eq!(expected.len(), 77);
        assert_eq!(encode(&point()).to_vec(), expected);
    }

    #[test]
    fn an_encoded_key_parses_back_to_its_point() {
        assert_eq!(parse(&encode(&point())), Ok(point()));
    }

    #[test]
    fn a_76_byte_key_is_refused() {
        let key = encode(&point());
        assert_eq!(parse(&key[..76]), Err(CoseKeyError::WrongLength(76)));
    }

    #[test]
    fn a_78_byte_key_is_refused() {
        let mut key = encode(&point()).to_vec();
        key.push(0x00);
        assert_eq!(parse(&key), Err(CoseKeyError::WrongLength(78)));
    }

    #[test]
    fn a_key_of_another_alg_is_refused() {
        let mut key = encode(&point());
        key[ALG_OFFSET] = EDDSA;
        assert_eq!(parse(&key), Err(CoseKeyError::NotCanonicalEs256));
    }

    #[test]
    fn every_byte_outside_the_coordinates_is_checked() {
        let fixed = (0..10).chain(42..45);
        for offset in fixed {
            let mut key = encode(&point());
            key[offset] ^= 0x01;
            assert_eq!(
                parse(&key),
                Err(CoseKeyError::NotCanonicalEs256),
                "byte {offset} must be checked"
            );
        }
    }

    #[test]
    fn the_sec1_form_is_the_tag_then_x_then_y() {
        let sec1 = point().to_sec1_uncompressed();
        let expected: Vec<u8> = [&[0x04][..], &point().x, &point().y].concat();
        assert_eq!(sec1.to_vec(), expected);
        assert_eq!(P256Point::from_sec1_uncompressed(&sec1), Some(point()));
    }

    #[test]
    fn only_the_65_byte_uncompressed_sec1_form_is_read() {
        let sec1 = point().to_sec1_uncompressed();
        let mut compressed = sec1;
        compressed[0] = 0x02;
        let mut long = sec1.to_vec();
        long.push(0x00);

        assert_eq!(P256Point::from_sec1_uncompressed(&compressed), None);
        assert_eq!(P256Point::from_sec1_uncompressed(&sec1[..64]), None);
        assert_eq!(P256Point::from_sec1_uncompressed(&long), None);
        assert_eq!(P256Point::from_sec1_uncompressed(&[]), None);
    }
}
