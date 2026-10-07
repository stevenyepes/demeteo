// Tests extracted from `crates/demeteo-core/src/domain/attachment.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn sanitize_strips_path_separators() {
    // Each non-alphanumeric, non-`-`/`_` char becomes `_`.
    // `..` → `__`, `/` → `_`, `\..evil.txt` → `_` + alphanumeric + `_txt`
    assert_eq!(
        sanitize_attachment_filename("../etc/passwd"),
        "___etc_passwd"
    );
    assert_eq!(sanitize_attachment_filename("..\\evil.txt"), "___evil_txt");
}

#[test]
fn sanitize_strips_null_bytes() {
    assert_eq!(sanitize_attachment_filename("name\0.png"), "name__png");
}

#[test]
fn sanitize_keeps_unicode() {
    // Unicode chars are mapped to '_' (non-ascii alphanumeric).
    assert_eq!(sanitize_attachment_filename("café.png"), "caf__png");
}

#[test]
fn sanitize_handles_empty_input() {
    assert_eq!(sanitize_attachment_filename(""), "attachment");
    assert_eq!(sanitize_attachment_filename("..."), "___");
    // Leading dash replaced
    assert!(sanitize_attachment_filename("-evil").starts_with('_'));
}

#[test]
fn mime_for_ext_known_types() {
    assert_eq!(mime_for_ext("png"), Some("image/png"));
    assert_eq!(mime_for_ext("JPG"), Some("image/jpeg"));
    assert_eq!(mime_for_ext("TIFF"), Some("image/tiff"));
    assert_eq!(mime_for_ext("tif"), Some("image/tiff"));
    assert_eq!(mime_for_ext("xyz"), None);
}

#[test]
fn compute_sha256_known_vector() {
    // "abc" → ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
    let hex = compute_sha256_hex(b"abc");
    assert_eq!(
        hex,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn compute_sha256_empty_input() {
    // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
    let hex = compute_sha256_hex(b"");
    assert_eq!(
        hex,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn compute_sha256_long_input() {
    // Million-a test: compute SHA-256 over a million 'a' bytes.
    // Verified against Python's hashlib (NIST reference).
    let buf = vec![b'a'; 1_000_000];
    let hex = compute_sha256_hex(&buf);
    assert_eq!(
        hex,
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn attached_file_on_disk_path() {
    let f = AttachedFile {
        id: "at-1".into(),
        name: "shot".into(),
        mime: "image/png".into(),
        sha256: "abc123".into(),
        size: 42,
        source_filename: "shot.png".into(),
    };
    assert_eq!(
        f.on_disk_path(std::path::Path::new("/tmp/att"), "png"),
        std::path::PathBuf::from("/tmp/att/abc123.png")
    );
}

#[test]
fn attached_file_serde_roundtrip() {
    let f = AttachedFile {
        id: "at-1".into(),
        name: "shot".into(),
        mime: "image/png".into(),
        sha256: "abc123".into(),
        size: 42,
        source_filename: "shot.png".into(),
    };
    let json = serde_json::to_string(&f).unwrap();
    let back: AttachedFile = serde_json::from_str(&json).unwrap();
    assert_eq!(back, f);
}

// ── resolved_ext / worktree_display_path ─────────────────────────────────────
//
// The ladder and the path choice each existed in three and two byte-identical
// copies, one of them inside an `async fn` where no test could reach it. Both
// decide the `<sha256>.<ext>` filename a prompt tells the agent to `Read`.

fn att(mime: &str, source_filename: &str) -> AttachedFile {
    AttachedFile {
        id: "at-1".to_string(),
        name: "shot".to_string(),
        mime: mime.to_string(),
        sha256: "abc123".to_string(),
        size: 10,
        source_filename: source_filename.to_string(),
    }
}

#[test]
fn a_known_mime_decides_the_extension() {
    assert_eq!(resolved_ext(&att("image/png", "whatever.jpeg")), "png");
}

#[test]
fn an_unknown_mime_falls_back_to_the_filenames_own_extension_lowercased() {
    assert_eq!(
        resolved_ext(&att("application/x-thing", "Report.MD")),
        "md",
        "the on-disk name is content-addressed and lowercase; an upper-case ext \
         would produce a path nothing wrote"
    );
}

#[test]
fn an_unknown_mime_and_no_extension_is_bin() {
    assert_eq!(resolved_ext(&att("application/x-thing", "README")), "bin");
}

#[test]
fn a_worktree_dir_wins_over_the_host_local_store_path() {
    let stored = std::path::Path::new("/home/u/.local/share/demeteo/attachments/f-1/abc123.png");
    assert_eq!(
        worktree_display_path(
            &att("image/png", "shot.png"),
            "png",
            Some("/wt/artifacts/_context"),
            stored
        ),
        std::path::Path::new("/wt/artifacts/_context")
            .join("attachments")
            .join("abc123.png")
            .to_string_lossy()
            .to_string(),
        "the copy inside the fence is the only path `external_directory: deny` accepts"
    );
}

#[test]
fn with_no_worktree_dir_the_stored_path_is_all_there_is() {
    let stored = std::path::Path::new("/store/f-1/abc123.png");
    assert_eq!(
        worktree_display_path(&att("image/png", "shot.png"), "png", None, stored),
        "/store/f-1/abc123.png"
    );
}

fn named(mime: &str, name: &str) -> AttachedFile {
    AttachedFile {
        name: name.to_string(),
        ..att(mime, name)
    }
}

/// One spelling, because the resolver scans for `[attachment`. A second
/// phrasing would be a name nothing resolves to a path.
#[test]
fn every_attachment_is_named_the_one_way_the_resolver_reads() {
    let block = attachment_block(
        &[
            named("text/markdown", "SPEC.md"),
            named("image/png", "wire.png"),
        ],
        true,
    )
    .unwrap();
    assert_eq!(
        block,
        "Attached: [attachment -- SPEC.md], [attachment -- wire.png]"
    );
}

/// The warning is about images specifically: a blind model still reads a
/// markdown file perfectly well, so saying otherwise would be false.
#[test]
fn a_blind_model_is_warned_only_when_an_image_is_actually_attached() {
    let with_image = attachment_block(&[named("image/png", "wire.png")], false).unwrap();
    assert!(with_image.contains("does not read images"));

    let text_only = attachment_block(&[named("text/markdown", "SPEC.md")], false).unwrap();
    assert!(!text_only.contains("does not read images"));

    let seeing = attachment_block(&[named("image/png", "wire.png")], true).unwrap();
    assert!(!seeing.contains("does not read images"));
}

#[test]
fn nothing_attached_renders_no_heading() {
    assert!(attachment_block(&[], true).is_none());
}

// ── normalize_mime / content_matches_mime ────────────────────────────────────

const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

fn riff_webp() -> Vec<u8> {
    let mut v = b"RIFF".to_vec();
    v.extend_from_slice(&[0x24, 0, 0, 0]);
    v.extend_from_slice(b"WEBPVP8 ");
    v
}

#[test]
fn normalize_mime_drops_parameters_case_and_padding() {
    assert_eq!(normalize_mime("Text/Plain; charset=utf-8"), "text/plain");
    assert_eq!(normalize_mime("  IMAGE/PNG  "), "image/png");
    assert_eq!(normalize_mime("application/json"), "application/json");
    assert_eq!(normalize_mime(""), "");
    assert_eq!(normalize_mime(";charset=x"), "");
}

#[test]
fn every_binary_signature_is_accepted() {
    let mut png = PNG_MAGIC.to_vec();
    png.extend_from_slice(b"rest");
    assert!(content_matches_mime("image/png", &png));
    assert!(content_matches_mime(
        "image/jpeg",
        &[0xFF, 0xD8, 0xFF, 0xE0, 0x00]
    ));
    assert!(content_matches_mime("image/gif", b"GIF87a\x01\x00"));
    assert!(content_matches_mime("image/gif", b"GIF89a\x01\x00"));
    assert!(content_matches_mime("image/webp", &riff_webp()));
    assert!(content_matches_mime("image/tiff", b"II*\0\x08\0\0\0"));
    assert!(content_matches_mime("image/tiff", b"MM\0*\0\0\0\x08"));
    assert!(content_matches_mime("application/pdf", b"%PDF-1.7\n"));
}

#[test]
fn a_truncated_or_altered_signature_is_rejected() {
    assert!(!content_matches_mime("image/png", &PNG_MAGIC[..7]));
    let mut bad_png = PNG_MAGIC;
    bad_png[7] = 0x0B;
    assert!(!content_matches_mime("image/png", &bad_png));
    assert!(!content_matches_mime("image/jpeg", &[0xFF, 0xD8]));
    assert!(!content_matches_mime("image/gif", b"GIF88a"));
    assert!(!content_matches_mime("image/gif", b"GIF89"));
    assert!(!content_matches_mime("image/tiff", b"II\0*"));
    assert!(!content_matches_mime("image/tiff", b"MM*\0"));
}

#[test]
fn webp_needs_both_riff_and_the_webp_tag() {
    let mut wave = b"RIFF".to_vec();
    wave.extend_from_slice(&[0x24, 0, 0, 0]);
    wave.extend_from_slice(b"WAVEfmt ");
    assert!(!content_matches_mime("image/webp", &wave));
    assert!(!content_matches_mime("image/webp", b"RIFF"));
    assert!(!content_matches_mime("image/webp", b"RIFF\x24\0\0\0WEB"));
    assert!(!content_matches_mime(
        "image/webp",
        b"XXXX\x24\0\0\0WEBPVP8 "
    ));
}

#[test]
fn the_pdf_marker_must_start_within_the_first_1024_bytes() {
    let at = |offset: usize| {
        let mut v = vec![b' '; offset];
        v.extend_from_slice(b"%PDF-");
        v
    };
    assert!(content_matches_mime("application/pdf", &at(0)));
    assert!(content_matches_mime("application/pdf", &at(1023)));
    assert!(!content_matches_mime("application/pdf", &at(1024)));
    assert!(!content_matches_mime("application/pdf", &at(1025)));
    assert!(!content_matches_mime("application/pdf", b"%PDF"));
}

#[test]
fn text_types_must_be_utf8_without_nul() {
    for mime in ["text/plain", "text/markdown", "application/json"] {
        assert!(content_matches_mime(mime, "héllo\n".as_bytes()), "{mime}");
        assert!(content_matches_mime(mime, b""), "{mime}");
        assert!(!content_matches_mime(mime, b"ab\0cd"), "{mime}");
        assert!(!content_matches_mime(mime, &[0x66, 0xFF, 0xFE]), "{mime}");
    }
}

#[test]
fn content_is_never_vouched_for_under_the_wrong_or_an_unknown_mime() {
    assert!(!content_matches_mime("image/jpeg", &PNG_MAGIC));
    assert!(!content_matches_mime("application/pdf", &PNG_MAGIC));
    assert!(!content_matches_mime("text/plain", &PNG_MAGIC));
    assert!(!content_matches_mime("application/octet-stream", b"hello"));
    assert!(!content_matches_mime("", b"hello"));
}

#[test]
fn a_tiff_by_path_takes_its_extension_from_the_file_not_the_callers_filename() {
    assert_eq!(
        path_form_stored_name("image/tiff", "a.tiff", Some("anything.zzz")),
        "anything.tiff"
    );
    assert_eq!(
        path_form_stored_name("image/tiff", "A.TIF", Some("scan")),
        "scan.tif"
    );
}

#[test]
fn no_separator_from_the_callers_filename_reaches_a_tiffs_extension() {
    let name = path_form_stored_name("image/tiff", "a.tiff", Some("x.a\\b"));
    let ext = Path::new(&name).extension().unwrap().to_string_lossy();
    assert_eq!(ext, "tiff");
    assert!(!ext.contains('\\') && !ext.contains('/'), "{name}");

    let name = path_form_stored_name("image/tiff", "a.tiff", Some("x.a/b"));
    assert_eq!(name, "b.tiff");
}

#[test]
fn a_basename_extension_with_a_separator_is_dropped_not_stored() {
    let name = path_form_stored_name("image/tiff", "a.ti\\ff", Some("shot.zzz"));
    assert_eq!(name, "shot");
}

#[test]
fn a_type_with_a_canonical_extension_keeps_the_callers_filename_unchanged() {
    assert_eq!(
        path_form_stored_name("image/png", "a.png", Some("shot.zzz")),
        "shot.zzz"
    );
}

#[test]
fn a_tiff_by_path_with_no_filename_is_its_own_basename() {
    assert_eq!(
        path_form_stored_name("image/tiff", "a.tiff", None),
        "a.tiff"
    );
    assert_eq!(path_form_stored_name("image/png", "a.png", None), "a.png");
}
