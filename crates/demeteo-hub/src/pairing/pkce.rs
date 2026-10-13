//! PKCE `S256` (RFC 7636), the only method the Hub accepts. There is no
//! `plain` branch to fall back to: a verifier is only ever checked against the
//! digest of itself.

use super::is_unreserved;
use crate::auth::challenge::base64url_encode;
use aws_lc_rs::constant_time::verify_slices_are_equal;
use sha2::{Digest, Sha256};

const MIN_VERIFIER_LEN: usize = 43;
const MAX_VERIFIER_LEN: usize = 128;

/// Whether `code_verifier` is the one `code_challenge` was derived from.
///
/// A verifier outside RFC 7636 §4.1 — 43 to 128 characters of the unreserved
/// set — is refused even when it hashes to the challenge. A challenge says
/// nothing about the length of what it was derived from, so this is the only
/// place a client that chose a guessable verifier is turned away.
pub fn verifier_matches(code_verifier: &str, code_challenge: &str) -> bool {
    if !is_well_formed(code_verifier) {
        return false;
    }
    let derived = base64url_encode(&Sha256::digest(code_verifier.as_bytes()));
    verify_slices_are_equal(derived.as_bytes(), code_challenge.as_bytes()).is_ok()
}

fn is_well_formed(code_verifier: &str) -> bool {
    (MIN_VERIFIER_LEN..=MAX_VERIFIER_LEN).contains(&code_verifier.len())
        && code_verifier.bytes().all(is_unreserved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 Appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    /// Spelled here and not read from the constant under test: a test built
    /// on `MIN_VERIFIER_LEN` follows the bound wherever it is moved.
    const SHORTEST: usize = 43;

    /// The challenge `verifier` could only be refused against for its length
    /// or its alphabet. Deriving it here pins neither the hash nor the
    /// encoding; [`CHALLENGE`] does that.
    fn challenge_of(verifier: &str) -> String {
        base64url_encode(&Sha256::digest(verifier.as_bytes()))
    }

    fn matches_its_own_challenge(verifier: &str) -> bool {
        verifier_matches(verifier, &challenge_of(verifier))
    }

    #[test]
    fn the_rfc_7636_verifier_matches_its_challenge() {
        assert!(verifier_matches(VERIFIER, CHALLENGE));
    }

    #[test]
    fn a_verifier_is_refused_against_another_verifiers_challenge() {
        let other = "a".repeat(SHORTEST);
        assert!(matches_its_own_challenge(&other));
        assert!(!verifier_matches(&other, CHALLENGE));
        assert!(!verifier_matches(VERIFIER, &challenge_of(&other)));
    }

    #[test]
    fn a_verifier_of_43_to_128_characters_is_accepted_and_one_beyond_either_end_is_refused() {
        for accepted in [43, 44, 127, 128] {
            assert!(
                matches_its_own_challenge(&"a".repeat(accepted)),
                "{accepted}"
            );
        }
        for refused in [0, 1, 42, 129] {
            assert!(
                !matches_its_own_challenge(&"a".repeat(refused)),
                "{refused}"
            );
        }
    }

    #[test]
    fn every_unreserved_character_is_accepted() {
        let unreserved: String = ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .chain("-._~".chars())
            .collect();
        assert_eq!(unreserved.len(), 66);
        assert!(matches_its_own_challenge(&unreserved));
    }

    #[test]
    fn a_verifier_with_a_character_outside_the_unreserved_set_is_refused() {
        let filler = "a".repeat(SHORTEST);
        assert!(matches_its_own_challenge(&filler));
        for outsider in [
            ' ', '+', '/', '=', '%', '*', '!', ',', ':', '@', '\0', '\n', '\u{7f}', 'é', 'Ａ',
        ] {
            for position in [0, SHORTEST / 2, SHORTEST - 1] {
                let mut characters: Vec<char> = filler.chars().collect();
                characters[position] = outsider;
                let verifier: String = characters.into_iter().collect();
                assert!(
                    !matches_its_own_challenge(&verifier),
                    "{outsider:?} at {position}"
                );
            }
        }
    }

    #[test]
    fn the_right_challenge_cut_short_or_extended_is_refused() {
        assert!(!verifier_matches(
            VERIFIER,
            &CHALLENGE[..CHALLENGE.len() - 1]
        ));
        assert!(!verifier_matches(VERIFIER, &format!("{CHALLENGE}A")));
        assert!(!verifier_matches(VERIFIER, &format!("{CHALLENGE}=")));
        assert!(!verifier_matches(VERIFIER, ""));
    }
}
