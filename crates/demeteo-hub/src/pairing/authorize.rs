//! What an `/authorize` request has to be before a user is asked about it.
//!
//! `/authorize` is reachable signed out, so it holds nothing: a request that
//! passes [`validate`] is handed to the approval page as a URL fragment
//! segment, and is parked only once a signed-in user opens it. The segment is
//! therefore the caller's to forge. It carries no secret and no signature, and
//! needs neither, because [`decode`] reads it back into the same
//! [`AuthorizeParams`] the query arrived as and runs the same [`validate`]:
//! there is no second set of rules for the two to disagree on, and nothing a
//! hand-built segment can ask for that a hand-built query could not.

use super::is_unreserved;
use crate::auth::challenge::{base64url_decode, base64url_encode};
use demeteo_hub_protocol::Scope;
use serde::de::value::{Error as ValueError, StrDeserializer};
use serde::de::IntoDeserializer;
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

const RESPONSE_TYPE: &str = "code";
const CHALLENGE_METHOD: &str = "S256";
const REDIRECT_SCHEME: &str = "http://";
const SHA256_LEN: usize = 32;
const MAX_REDIRECT_URI_LEN: usize = 255;
const MAX_STATE_LEN: usize = 128;
const MAX_INSTANCE_NAME_CHARS: usize = 64;
const MAX_PLATFORM_LABEL_CHARS: usize = 32;

/// The query as it arrived. A parameter this does not name is ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizeParams {
    pub response_type: Option<String>,
    pub code_challenge_method: Option<String>,
    pub code_challenge: Option<String>,
    pub redirect_uri: Option<String>,
    pub dpop_jkt: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub instance_name: Option<String>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub app_version: Option<String>,
}

/// A request that passed [`validate`]. The four labels are what the caller
/// said about itself, trimmed, and are proof of nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub code_challenge: String,
    pub redirect_uri: String,
    pub dpop_jkt: String,
    pub scopes: Vec<Scope>,
    pub state: String,
    pub instance_name: String,
    pub os: String,
    pub arch: String,
    pub app_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthorizeRefusal {
    #[error("`response_type` is not `code`")]
    ResponseType,
    #[error("`code_challenge_method` is not `S256`")]
    ChallengeMethod,
    #[error("`code_challenge` is not unpadded base64url of 32 bytes")]
    Challenge,
    #[error("`redirect_uri` is not http to the loopback address literal with a port")]
    RedirectUri,
    #[error("`dpop_jkt` is not unpadded base64url of 32 bytes")]
    DeviceKey,
    #[error("`scope` is empty, repeats a scope or names one the Hub does not have")]
    Scope,
    #[error("`state` is not 1 to 128 unreserved characters")]
    State,
    #[error("a label is empty, too long or holds a character that is not shown")]
    Label,
}

pub fn validate(params: &AuthorizeParams) -> Result<AuthorizeRequest, AuthorizeRefusal> {
    if params.response_type.as_deref() != Some(RESPONSE_TYPE) {
        return Err(AuthorizeRefusal::ResponseType);
    }
    if params.code_challenge_method.as_deref() != Some(CHALLENGE_METHOD) {
        return Err(AuthorizeRefusal::ChallengeMethod);
    }
    Ok(AuthorizeRequest {
        code_challenge: accepted(&params.code_challenge, is_sha256_text)
            .ok_or(AuthorizeRefusal::Challenge)?,
        redirect_uri: accepted(&params.redirect_uri, is_loopback_redirect)
            .ok_or(AuthorizeRefusal::RedirectUri)?,
        dpop_jkt: accepted(&params.dpop_jkt, is_sha256_text).ok_or(AuthorizeRefusal::DeviceKey)?,
        scopes: params
            .scope
            .as_deref()
            .and_then(requested_scopes)
            .ok_or(AuthorizeRefusal::Scope)?,
        state: accepted(&params.state, is_state).ok_or(AuthorizeRefusal::State)?,
        instance_name: label(&params.instance_name, MAX_INSTANCE_NAME_CHARS)?,
        os: label(&params.os, MAX_PLATFORM_LABEL_CHARS)?,
        arch: label(&params.arch, MAX_PLATFORM_LABEL_CHARS)?,
        app_version: label(&params.app_version, MAX_PLATFORM_LABEL_CHARS)?,
    })
}

/// The opaque segment of the approval page's URL: base64url characters only,
/// so it needs no escaping in a fragment.
pub fn encode(request: &AuthorizeRequest) -> String {
    let scope: Vec<&str> = request.scopes.iter().copied().map(scope_name).collect();
    let params = AuthorizeParams {
        response_type: Some(RESPONSE_TYPE.to_owned()),
        code_challenge_method: Some(CHALLENGE_METHOD.to_owned()),
        code_challenge: Some(request.code_challenge.clone()),
        redirect_uri: Some(request.redirect_uri.clone()),
        dpop_jkt: Some(request.dpop_jkt.clone()),
        scope: Some(scope.join(" ")),
        state: Some(request.state.clone()),
        instance_name: Some(request.instance_name.clone()),
        os: Some(request.os.clone()),
        arch: Some(request.arch.clone()),
        app_version: Some(request.app_version.clone()),
    };
    // Writing a struct of strings to memory has no failing case. Were there
    // one, what it leaves is the empty segment, which `decode` refuses.
    base64url_encode(&serde_json::to_vec(&params).unwrap_or_default())
}

/// A segment that is not the text [`encode`] writes holds no parameters, and
/// is refused as a query with none would be.
pub fn decode(segment: &str) -> Result<AuthorizeRequest, AuthorizeRefusal> {
    let params = base64url_decode(segment)
        .and_then(|json| serde_json::from_slice::<AuthorizeParams>(&json).ok())
        .unwrap_or_default();
    validate(&params)
}

fn accepted(value: &Option<String>, rule: fn(&str) -> bool) -> Option<String> {
    value
        .as_deref()
        .filter(|text| rule(text))
        .map(str::to_owned)
}

fn is_sha256_text(text: &str) -> bool {
    base64url_decode(text).is_some_and(|bytes| bytes.len() == SHA256_LEN)
}

/// Whether `uri` is, byte for byte, `http://`, the loopback address literal,
/// `:`, a port and a path. This is a comparison and not a parse on purpose.
///
/// The authorization code leaves in this redirect, so whatever this accepts
/// decides who receives it, and the desktop's listener is the only intended
/// receiver. A URL parser normalises before it answers: it lowercases the
/// scheme, reads `127.1` and `2130706433` as this address, takes the host
/// from after an `@`, and reads `%2e%2e` as `..`. Each of those is a spelling
/// the browser that follows the redirect may resolve differently from the
/// parser that approved it, and the gap between the two is where a code is
/// sent somewhere else. Comparing bytes leaves no second reading.
///
/// Do not widen this toward `domain/oauth/registration.rs` in `demeteo-core`,
/// which also takes `https`, private-use schemes, `[::1]` and the loopback
/// hostname. That is a dynamic-registration rule for clients of a listener on
/// their own machine. Here a private-use scheme goes to whichever application
/// claimed it, the hostname goes wherever name resolution sends it (RFC 8252
/// §8.3), and any other host takes the code off the machine. The port is
/// required because the desktop always binds one for this callback; a URI
/// without one names port 80, which it never holds.
///
/// Query and fragment are refused with everything else outside the unreserved
/// set, so the code and `state` can be appended with no encoder and no
/// question of which separator comes first.
fn is_loopback_redirect(uri: &str) -> bool {
    let origin = format!("{REDIRECT_SCHEME}{}:", Ipv4Addr::LOCALHOST);
    let Some(port_and_path) = uri.strip_prefix(origin.as_str()) else {
        return false;
    };
    let Some(path_starts) = port_and_path.find('/') else {
        return false;
    };
    let (port, path) = port_and_path.split_at(path_starts);
    uri.len() <= MAX_REDIRECT_URI_LEN
        && is_port(port)
        && path.bytes().all(|byte| byte == b'/' || is_unreserved(byte))
}

/// Digits only, because `u16`'s parser also takes a leading `+`; no leading
/// zero, which also rules out port 0 and leaves each port one spelling.
fn is_port(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_digit())
        && !text.starts_with('0')
        && text.parse::<u16>().is_ok()
}

/// Split on the space character alone: a tab, a doubled space or a space at
/// either end leaves a name that is no scope, and the list is refused.
fn requested_scopes(list: &str) -> Option<Vec<Scope>> {
    let mut scopes = Vec::new();
    for name in list.split(' ') {
        let scope = scope_named(name)?;
        if scopes.contains(&scope) {
            return None;
        }
        scopes.push(scope);
    }
    Some(scopes)
}

pub(crate) fn scope_named(name: &str) -> Option<Scope> {
    let name: StrDeserializer<'_, ValueError> = name.into_deserializer();
    Scope::deserialize(name).ok()
}

pub(crate) fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Read => "read",
        Scope::Spend => "spend",
        Scope::Gates => "gates",
    }
}

fn is_state(text: &str) -> bool {
    (1..=MAX_STATE_LEN).contains(&text.len()) && text.bytes().all(is_unreserved)
}

fn label(typed: &Option<String>, max_chars: usize) -> Result<String, AuthorizeRefusal> {
    let label = typed.as_deref().unwrap_or_default().trim();
    let shown_as_sent = (1..=max_chars).contains(&label.chars().count())
        && !label.contains(|c: char| c.is_control() || is_unseen(c));
    if shown_as_sent {
        Ok(label.to_owned())
    } else {
        Err(AuthorizeRefusal::Label)
    }
}

/// Zero-width characters, the bidirectional embeddings, overrides and
/// isolates, and the byte-order mark. The labels are put on the consent
/// screen as the name of the machine being approved, by whoever built the
/// link: with these, the name a user reads is not the characters that were
/// sent, in another order or with some of them missing.
fn is_unseen(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// RFC 7636 Appendix B's challenge and the thumbprint of the RFC 9449
    /// §4.1 key: two real SHA-256 digests in the form a desktop sends them.
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    const JKT: &str = "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I";
    const REDIRECT: &str = "http://127.0.0.1:8080/cb";

    type Field = fn(&mut AuthorizeParams) -> &mut Option<String>;

    /// The bounds are spelled here and not read from the constants under
    /// test, which a test built on them would follow wherever they moved.
    const LABELS: [(&str, usize, Field); 4] = [
        ("instance_name", 64, |p| &mut p.instance_name),
        ("os", 32, |p| &mut p.os),
        ("arch", 32, |p| &mut p.arch),
        ("app_version", 32, |p| &mut p.app_version),
    ];

    fn valid() -> AuthorizeParams {
        let text = |value: &str| Some(value.to_owned());
        AuthorizeParams {
            response_type: text("code"),
            code_challenge_method: text("S256"),
            code_challenge: text(CHALLENGE),
            redirect_uri: text(REDIRECT),
            dpop_jkt: text(JKT),
            scope: text("read spend"),
            state: text("xyz-123"),
            instance_name: text("Workstation"),
            os: text("linux"),
            arch: text("x86_64"),
            app_version: text("1.4.0"),
        }
    }

    /// The valid request with one parameter replaced, or removed by `None`.
    /// Every refusal below is of a request that is accepted once that
    /// parameter is put back, which
    /// [`a_request_with_every_parameter_valid_is_accepted_as_what_it_asked_for`]
    /// holds.
    fn with(field: Field, value: Option<&str>) -> AuthorizeParams {
        let mut params = valid();
        *field(&mut params) = value.map(str::to_owned);
        params
    }

    fn refusal(field: Field, value: Option<&str>) -> Option<AuthorizeRefusal> {
        validate(&with(field, value)).err()
    }

    fn of_length(len: usize) -> String {
        base64url_encode(&vec![0x5a; len])
    }

    fn segment_of(params: &AuthorizeParams) -> String {
        base64url_encode(&serde_json::to_vec(params).unwrap())
    }

    #[test]
    fn a_request_with_every_parameter_valid_is_accepted_as_what_it_asked_for() {
        assert_eq!(
            validate(&valid()),
            Ok(AuthorizeRequest {
                code_challenge: CHALLENGE.to_owned(),
                redirect_uri: REDIRECT.to_owned(),
                dpop_jkt: JKT.to_owned(),
                scopes: vec![Scope::Read, Scope::Spend],
                state: "xyz-123".to_owned(),
                instance_name: "Workstation".to_owned(),
                os: "linux".to_owned(),
                arch: "x86_64".to_owned(),
                app_version: "1.4.0".to_owned(),
            })
        );
    }

    #[test]
    fn a_response_type_other_than_code_is_refused() {
        for value in [None, Some(""), Some("token"), Some("Code"), Some("code ")] {
            assert_eq!(
                refusal(|p| &mut p.response_type, value),
                Some(AuthorizeRefusal::ResponseType),
                "{value:?}"
            );
        }
    }

    #[test]
    fn a_challenge_method_that_is_absent_or_plain_is_refused_and_s256_is_accepted() {
        assert!(validate(&with(|p| &mut p.code_challenge_method, Some("S256"))).is_ok());
        for value in [None, Some("plain"), Some(""), Some("s256"), Some("S256 ")] {
            assert_eq!(
                refusal(|p| &mut p.code_challenge_method, value),
                Some(AuthorizeRefusal::ChallengeMethod),
                "{value:?}"
            );
        }
    }

    #[test]
    fn a_challenge_or_device_key_that_is_not_base64url_of_32_bytes_is_refused() {
        let digests: [(Field, AuthorizeRefusal, &str); 2] = [
            (
                |p| &mut p.code_challenge,
                AuthorizeRefusal::Challenge,
                CHALLENGE,
            ),
            (|p| &mut p.dpop_jkt, AuthorizeRefusal::DeviceKey, JKT),
        ];
        for (field, refused_as, real) in digests {
            assert!(validate(&with(field, Some(&of_length(32)))).is_ok());
            assert_eq!(refusal(field, None), Some(refused_as));
            for len in [0, 1, 31, 33, 64] {
                assert_eq!(
                    refusal(field, Some(&of_length(len))),
                    Some(refused_as),
                    "{len} bytes"
                );
            }
            let other_forms = [
                format!("{real}="),
                real.replace('-', "+"),
                format!(" {real}"),
            ];
            for form in other_forms {
                assert_ne!(form, real);
                assert_eq!(refusal(field, Some(&form)), Some(refused_as), "{form:?}");
            }
        }
    }

    #[test]
    fn every_redirect_that_is_not_http_to_the_loopback_literal_with_a_port_is_refused() {
        let near_misses = [
            "https://127.0.0.1:8080/cb",
            "HTTP://127.0.0.1:8080/cb",
            "http://localhost:8080/cb",
            "http://127.0.0.1/cb",
            "http://127.0.0.1:8080",
            "http://127.0.0.1:0/cb",
            "http://127.0.0.1:65536/cb",
            "http://127.0.0.1:08080/cb",
            "http://127.1:8080/cb",
            "http://127.0.0.2:8080/cb",
            "http://2130706433:8080/cb",
            "http://0.0.0.0:8080/cb",
            "http://[::1]:8080/cb",
            "http://127.0.0.1.evil.example:8080/cb",
            "http://127.0.0.1:8080@evil.example/cb",
            "http://evil.example@127.0.0.1:8080/cb",
            "http://evil.example:8080/cb",
            "http://127.0.0.1:8080/cb?x=1",
            "http://127.0.0.1:8080/cb#f",
            "http://127.0.0.1:8080/c b",
            " http://127.0.0.1:8080/cb",
            "http://127.0.0.1:8080\\cb",
            "http://127.0.0.1:8080/%2e%2e",
            "demeteo://callback",
            "",
            "http://127.0.0.1:/cb",
            "http://127.0.0.1:+8080/cb",
            "http://127.0.0.1:123456/cb",
            "http://127.0.0.1:80a/cb",
            "http://127.0.0.1:８０８０/cb",
            "http://127.0.0.1:8080/cb\n",
            "http://127.0.0.1:8080/cb;x",
            "http://127.0.0.1:8080/é",
            "http:/127.0.0.1:8080/cb",
            "//127.0.0.1:8080/cb",
        ];
        for uri in near_misses {
            assert_eq!(
                refusal(|p| &mut p.redirect_uri, Some(uri)),
                Some(AuthorizeRefusal::RedirectUri),
                "{uri:?}"
            );
        }
        assert_eq!(
            refusal(|p| &mut p.redirect_uri, None),
            Some(AuthorizeRefusal::RedirectUri)
        );
    }

    #[test]
    fn a_loopback_redirect_is_accepted_on_any_port_and_kept_as_it_was_sent() {
        let accepted = [
            "http://127.0.0.1:8080/cb",
            "http://127.0.0.1:1/",
            "http://127.0.0.1:65535/oauth/callback",
            "http://127.0.0.1:9/A-z.0_9~/",
        ];
        for uri in accepted {
            let request = validate(&with(|p| &mut p.redirect_uri, Some(uri)));
            assert_eq!(request.map(|r| r.redirect_uri), Ok(uri.to_owned()), "{uri}");
        }
    }

    #[test]
    fn a_redirect_of_255_bytes_is_accepted_and_one_of_256_is_refused() {
        let of_len = |len: usize| format!("{REDIRECT}{}", "a".repeat(len - REDIRECT.len()));
        assert_eq!(of_len(255).len(), 255);
        assert!(validate(&with(|p| &mut p.redirect_uri, Some(&of_len(255)))).is_ok());
        assert_eq!(
            refusal(|p| &mut p.redirect_uri, Some(&of_len(256))),
            Some(AuthorizeRefusal::RedirectUri)
        );
    }

    #[test]
    fn scopes_are_accepted_in_the_order_they_were_asked_for() {
        let asked = [
            ("read", vec![Scope::Read]),
            ("gates read", vec![Scope::Gates, Scope::Read]),
            (
                "read spend gates",
                vec![Scope::Read, Scope::Spend, Scope::Gates],
            ),
        ];
        for (scope, expected) in asked {
            let request = validate(&with(|p| &mut p.scope, Some(scope)));
            assert_eq!(request.map(|r| r.scopes), Ok(expected), "{scope:?}");
        }
    }

    #[test]
    fn a_scope_list_that_is_empty_repeats_a_scope_or_names_an_unknown_one_is_refused() {
        let refused = [
            None,
            Some(""),
            Some(" "),
            Some("read read"),
            Some("read spend read"),
            Some("read  spend"),
            Some(" read"),
            Some("read "),
            Some("read\tspend"),
            Some("read,spend"),
            Some("read+spend"),
            Some("READ"),
            Some("logs"),
            Some("configure"),
            Some("read logs"),
        ];
        for scope in refused {
            assert_eq!(
                refusal(|p| &mut p.scope, scope),
                Some(AuthorizeRefusal::Scope),
                "{scope:?}"
            );
        }
    }

    #[test]
    fn a_state_of_1_to_128_unreserved_characters_is_accepted_and_one_beyond_either_end_is_refused()
    {
        let unreserved: String = ('A'..='Z')
            .chain('a'..='z')
            .chain('0'..='9')
            .chain("-._~".chars())
            .collect();
        for state in ["a".to_owned(), "a".repeat(127), "a".repeat(128), unreserved] {
            let request = validate(&with(|p| &mut p.state, Some(&state)));
            assert_eq!(request.map(|r| r.state), Ok(state));
        }
        for state in [None, Some(String::new()), Some("a".repeat(129))] {
            assert_eq!(
                refusal(|p| &mut p.state, state.as_deref()),
                Some(AuthorizeRefusal::State),
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_state_with_a_character_outside_the_unreserved_set_is_refused() {
        for outsider in [' ', '&', '=', '#', '%', '+', '/', '?', '\n', '\0', 'é'] {
            let state = format!("ab{outsider}cd");
            assert_eq!(
                refusal(|p| &mut p.state, Some(&state)),
                Some(AuthorizeRefusal::State),
                "{outsider:?}"
            );
        }
    }

    #[test]
    fn a_label_is_bounded_in_characters_after_trimming() {
        for (name, max, field) in LABELS {
            let longest = "é".repeat(max);
            assert!(longest.len() > max);
            for accepted in ["a".to_owned(), longest.clone()] {
                let mut request = with(field, Some(&accepted));
                assert!(validate(&request).is_ok(), "{name} = {accepted:?}");
                assert_eq!(*field(&mut request), Some(accepted));
            }
            let padded = format!("  {longest}\t\n");
            assert!(validate(&with(field, Some(&padded))).is_ok(), "{name}");
            let one_over = "é".repeat(max + 1);
            for refused in [None, Some(""), Some(" \t "), Some(one_over.as_str())] {
                assert_eq!(
                    refusal(field, refused),
                    Some(AuthorizeRefusal::Label),
                    "{name} = {refused:?}"
                );
            }
        }
    }

    #[test]
    fn each_label_is_kept_trimmed_and_in_its_own_field() {
        let mut params = valid();
        params.instance_name = Some("  Büro-PC 東京 \n".to_owned());
        params.os = Some(" macos".to_owned());
        params.arch = Some("aarch64 ".to_owned());
        params.app_version = Some("\t2.0.0-rc.1".to_owned());
        let request = validate(&params).unwrap();
        assert_eq!(request.instance_name, "Büro-PC 東京");
        assert_eq!(request.os, "macos");
        assert_eq!(request.arch, "aarch64");
        assert_eq!(request.app_version, "2.0.0-rc.1");
    }

    #[test]
    fn a_label_holding_a_control_or_unseen_character_is_refused() {
        let refused = [
            '\0', '\u{1b}', '\n', '\u{7f}', '\u{85}', '\u{200b}', '\u{200d}', '\u{200f}',
            '\u{202a}', '\u{202e}', '\u{2060}', '\u{2066}', '\u{2069}', '\u{feff}',
        ];
        for (name, _, field) in LABELS {
            for character in refused {
                let label = format!("a{character}b");
                assert_eq!(
                    refusal(field, Some(&label)),
                    Some(AuthorizeRefusal::Label),
                    "{name} holding {character:?}"
                );
            }
        }
    }

    #[test]
    fn a_parameter_the_hub_does_not_name_is_ignored() {
        let mut query = serde_json::to_value(valid()).unwrap();
        query["client_id"] = json!("demeteo");
        query["nonce"] = json!("n");
        let params: AuthorizeParams = serde_json::from_value(query).unwrap();
        assert_eq!(params, valid());
        assert!(validate(&params).is_ok());
    }

    /// Three of `~` in a row put one where its low six bits are a whole
    /// base64 digit, the 62nd, and three of `?` do the same for the 63rd: the
    /// two digits the URL-safe alphabet spells differently.
    #[test]
    fn decoding_an_encoded_request_gives_the_request_back() {
        let mut params = valid();
        params.scope = Some("gates read spend".to_owned());
        params.state = Some("a~~~b".to_owned());
        params.instance_name = Some(" Büro-PC \"東京\" ??? ".to_owned());
        params.redirect_uri = Some("http://127.0.0.1:65535/oauth/callback".to_owned());
        let request = validate(&params).unwrap();

        let segment = encode(&request);

        assert_eq!(decode(&segment), Ok(request));
        assert!(segment.contains('-') && segment.contains('_'), "{segment}");
        assert!(segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'));
    }

    #[test]
    fn a_hand_built_segment_is_refused_as_the_same_query_would_be() {
        assert_eq!(decode(&segment_of(&valid())), validate(&valid()));
        let hand_built: [(Field, Option<&str>); 6] = [
            (|p| &mut p.code_challenge_method, Some("plain")),
            (|p| &mut p.code_challenge_method, None),
            (|p| &mut p.redirect_uri, Some("http://localhost:8080/cb")),
            (|p| &mut p.redirect_uri, Some("https://evil.example/cb")),
            (|p| &mut p.scope, Some("read logs")),
            (|p| &mut p.instance_name, Some("a\u{202e}b")),
        ];
        for (field, value) in hand_built {
            let params = with(field, value);
            let refused = validate(&params);
            assert!(refused.is_err(), "{value:?}");
            assert_eq!(decode(&segment_of(&params)), refused, "{value:?}");
        }
    }

    #[test]
    fn a_segment_that_is_not_what_encode_writes_is_refused_as_a_query_with_no_parameters() {
        let empty = validate(&AuthorizeParams::default());
        assert_eq!(empty, Err(AuthorizeRefusal::ResponseType));
        let real = segment_of(&valid());
        let written = serde_json::to_vec(&valid()).unwrap();
        let unreadable = [
            String::new(),
            "!".to_owned(),
            format!("{real}="),
            format!("{real}.{real}"),
            base64url_encode(&[written.as_slice(), b"{}"].concat()),
            base64url_encode(b"[]"),
            base64url_encode(b"null"),
            base64url_encode(b"not json"),
            base64url_encode(&[0xff, 0xfe]),
            base64url_encode(REDIRECT.as_bytes()),
        ];
        for segment in unreadable {
            assert_eq!(decode(&segment), empty, "{segment:?}");
        }
    }

    fn with_one_character_altered(segment: &str, position: usize) -> String {
        let mut bytes = segment.as_bytes().to_vec();
        bytes[position] = if bytes[position] == b'A' { b'B' } else { b'A' };
        String::from_utf8(bytes).unwrap()
    }

    /// The text `encode` writes opens with the two parameters whose values
    /// are fixed, so every character over that opening spells a name or a
    /// value with one accepted form, and altering it leaves a request that
    /// lacks one of the two.
    #[test]
    fn a_segment_altered_where_it_spells_the_response_type_or_the_challenge_method_is_refused() {
        let request = validate(&valid()).unwrap();
        let segment = encode(&request);
        let fixed = "{\"response_type\":\"code\",\"code_challenge_method\":\"S256\",";
        let written = base64url_decode(&segment).unwrap();
        assert!(written.starts_with(fixed.as_bytes()));

        let characters_over_it = fixed.len() / 3 * 4;
        assert_eq!(characters_over_it, 72);
        for position in 0..characters_over_it {
            let altered = with_one_character_altered(&segment, position);
            assert_ne!(altered, segment);
            let refused = decode(&altered).err();
            assert!(
                matches!(
                    refused,
                    Some(AuthorizeRefusal::ResponseType | AuthorizeRefusal::ChallengeMethod)
                ),
                "altered at {position}: {refused:?}"
            );
        }
    }

    /// A segment is not signed, so a character altered inside a label is a
    /// request for another label, as another query would be. What no altered
    /// segment may do is pass as the one it was made from.
    #[test]
    fn a_segment_altered_anywhere_is_never_taken_for_the_request_it_was_made_from() {
        let request = validate(&valid()).unwrap();
        let segment = encode(&request);
        let mut refused = 0;
        for position in 0..segment.len() {
            match decode(&with_one_character_altered(&segment, position)) {
                Ok(other) => assert_ne!(other, request, "altered at {position}"),
                Err(_) => refused += 1,
            }
        }
        assert!(
            refused > segment.len() / 2,
            "{refused} of {}",
            segment.len()
        );
    }
}
