// Tests extracted from `crates/demeteo-core/src/application/attachments.rs`
// (mirrored-tests convention). `super` = that module.

use std::sync::Arc;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::models::{Feature, Project};
use crate::state::AppContext;

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\n-not-a-real-image";

fn fixture(tag: &str) -> AppContext {
    let dir = crate::support::test_dir::scratch(&format!("demeteo-attachments-{tag}"));
    build_core_context(
        CoreConfig {
            app_data_dir: dir,
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    )
}

fn feature_in(ctx: &AppContext, id: &str) {
    let project_id = crate::domain::ids::ProjectId::from("p-1".to_string());
    let _ = ctx.projects.add(Project {
        id: project_id.clone(),
        name: "project".to_string(),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: 0,
    });
    ctx.features
        .add(Feature {
            id: FeatureId::from(id.to_string()),
            project_id,
            workflow_id: None,
            workflow_version_id: None,
            title: format!("feature {id}"),
            description: String::new(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: None,
            resolved_branch: None,
        })
        .unwrap();
}

struct Input<'a> {
    source_path: &'a str,
    mime: Option<&'a str>,
    source_filename: Option<&'a str>,
    bytes: Option<&'a [u8]>,
}

fn commit(ctx: &AppContext, tag: &str, i: &Input) -> Result<AttachedFile, AppError> {
    feature_in(ctx, tag);
    commit_attachment_inner(
        &ctx.features,
        &ctx.attachment_json,
        &ctx.attachments,
        tag,
        i.source_path,
        i.mime,
        i.source_filename,
        i.bytes.map(<[u8]>::to_vec),
    )
}

fn stage(ctx: &AppContext, tag: &str, i: &Input) -> Result<AttachedFile, String> {
    let staged = stage_on_owner(
        ctx.attachments.as_ref(),
        tag,
        "ticket",
        Vec::new(),
        StagedAttachmentInput {
            source_path: i.source_path.to_string(),
            mime: i.mime.map(str::to_string),
            source_filename: i.source_filename.map(str::to_string),
            bytes: i.bytes.map(<[u8]>::to_vec),
        },
    )?;
    match staged {
        Staged::Added { file, .. } | Staged::Unchanged(file) => Ok(file),
    }
}

fn stored_ext(ctx: &AppContext, owner: &str, file: &AttachedFile) -> String {
    let path = ctx
        .attachments
        .lookup_path(owner, &file.sha256, &resolved_ext(file));
    assert!(path.is_file(), "{} should hold the bytes", path.display());
    path.extension().unwrap().to_string_lossy().into_owned()
}

#[tokio::test]
async fn a_png_path_resolves_to_png_in_both_callers() {
    let ctx = fixture("png-path");
    let dir = crate::support::test_dir::scratch("demeteo-attachments-png-src");
    let src = dir.join("shot.png");
    std::fs::write(&src, PNG_BYTES).unwrap();
    let source_path = src.to_string_lossy().into_owned();
    let input = Input {
        source_path: &source_path,
        mime: None,
        source_filename: None,
        bytes: None,
    };

    let committed = commit(&ctx, "f-png", &input).unwrap();
    let staged = stage(&ctx, "t-png", &input).unwrap();

    assert_eq!(committed.mime, "image/png");
    assert_eq!(staged.mime, committed.mime);
    assert_eq!(stored_ext(&ctx, "f-png", &committed), "png");
    assert_eq!(stored_ext(&ctx, "t-png", &staged), "png");
}

#[tokio::test]
async fn a_mime_with_no_filename_still_picks_the_extension_from_the_mime() {
    let ctx = fixture("mime-only");
    let input = Input {
        source_path: "",
        mime: Some("application/pdf"),
        source_filename: None,
        bytes: Some(b"%PDF-1.4 body"),
    };

    let committed = commit(&ctx, "f-pdf", &input).unwrap();
    let staged = stage(&ctx, "t-pdf", &input).unwrap();

    assert_eq!(committed.mime, "application/pdf");
    assert_eq!(staged.mime, committed.mime);
    assert_eq!(stored_ext(&ctx, "f-pdf", &committed), "pdf");
    assert_eq!(stored_ext(&ctx, "t-pdf", &staged), "pdf");
}

#[tokio::test]
async fn an_unknown_mime_falls_back_to_the_filename_extension_in_both_callers() {
    let ctx = fixture("ext-fallback");
    let input = Input {
        source_path: "",
        mime: Some("text/x-patch"),
        source_filename: Some("Change.TXT"),
        bytes: Some(b"diff --git a b"),
    };

    let committed = commit(&ctx, "f-patch", &input).unwrap();
    let staged = stage(&ctx, "t-patch", &input).unwrap();

    assert_eq!(committed.mime, "text/x-patch");
    assert_eq!(staged.mime, committed.mime);
    assert_eq!(stored_ext(&ctx, "f-patch", &committed), "txt");
    assert_eq!(stored_ext(&ctx, "t-patch", &staged), "txt");
}

#[tokio::test]
async fn an_unsupported_type_is_refused_with_the_same_prefix_by_both_callers() {
    let ctx = fixture("unsupported");
    let input = Input {
        source_path: "",
        mime: None,
        source_filename: Some("tool.EXE"),
        bytes: Some(b"MZ"),
    };

    let committed = commit(&ctx, "f-exe", &input).unwrap_err().to_string();
    let staged = stage(&ctx, "t-exe", &input).unwrap_err();

    assert!(
        committed.contains(
            "unsupported attachment type: mime=application/octet-stream ext=exe (allowed: png, jpg, gif, webp, tiff, pdf, txt, md, json)"
        ),
        "{committed}"
    );
    assert_eq!(
        staged,
        "unsupported attachment type: mime=application/octet-stream ext=exe"
    );
}

#[test]
fn the_ladder_prefers_the_mime_then_the_filename_then_the_path_then_bin() {
    let resolve = |mime, name, path: &str| {
        let r = resolve_attachment_type(mime, name, Path::new(path));
        (r.mime, r.ext)
    };
    let pair = |m: &str, e: &str| (m.to_string(), e.to_string());

    assert_eq!(
        resolve(Some("image/png"), Some("a.txt"), ""),
        pair("image/png", "png")
    );
    assert_eq!(
        resolve(None, Some("a.md"), "b.txt"),
        pair("text/markdown", "md")
    );
    assert_eq!(
        resolve(None, None, "b.json"),
        pair("application/json", "json")
    );
    assert_eq!(
        resolve(Some("text/x-patch"), Some("a.PATCH"), "b.txt"),
        pair("text/x-patch", "patch")
    );
    assert_eq!(
        resolve(None, None, ""),
        pair("application/octet-stream", "bin")
    );
}

mod agent_item {
    use super::*;
    use crate::support::test_dir::TestDir;
    use base64::Engine;

    const PDF_BYTES: &[u8] = b"%PDF-1.4\n%body";

    fn std_b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    pub(super) fn path_item(path: &Path) -> AgentAttachment {
        AgentAttachment {
            path: Some(path.to_string_lossy().into_owned()),
            ..AgentAttachment::default()
        }
    }

    pub(super) fn inline_item(bytes: &[u8], filename: &str) -> AgentAttachment {
        AgentAttachment {
            content_base64: Some(std_b64(bytes)),
            filename: Some(filename.to_string()),
            ..AgentAttachment::default()
        }
    }

    fn file_in(dir: &TestDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn refusal(item: &AgentAttachment) -> String {
        match resolve_agent_attachment(3, item, &[]) {
            Ok(_) => panic!("expected a refusal"),
            Err(e) => e,
        }
    }

    fn assert_refused(item: &AgentAttachment, cause: &str) {
        let e = refusal(item);
        assert!(e.starts_with("attachments[3]: "), "{e}");
        assert!(e.contains(cause), "{e:?} lacks {cause:?}");
    }

    #[test]
    fn a_png_by_path_stages_as_bytes_with_the_basename() {
        let dir = TestDir::new("agent-item-path");
        let p = file_in(&dir, "shot.png", PNG_BYTES);

        let staged = resolve_agent_attachment(0, &path_item(&p), &[]).unwrap();

        assert_eq!(staged.source_path, "");
        assert_eq!(staged.bytes.as_deref(), Some(PNG_BYTES));
        assert_eq!(staged.mime.as_deref(), Some("image/png"));
        assert_eq!(staged.source_filename.as_deref(), Some("shot.png"));
    }

    #[test]
    fn a_png_inline_stages_as_bytes_with_the_given_filename() {
        let staged = resolve_agent_attachment(0, &inline_item(PNG_BYTES, "shot.png"), &[]).unwrap();

        assert_eq!(staged.source_path, "");
        assert_eq!(staged.bytes.as_deref(), Some(PNG_BYTES));
        assert_eq!(staged.mime.as_deref(), Some("image/png"));
        assert_eq!(staged.source_filename.as_deref(), Some("shot.png"));
    }

    #[test]
    fn a_supplied_filename_overrides_the_path_basename() {
        let dir = TestDir::new("agent-item-rename");
        let p = file_in(&dir, "a.png", PNG_BYTES);
        let item = AgentAttachment {
            filename: Some("renamed.png".to_string()),
            ..path_item(&p)
        };

        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();

        assert_eq!(staged.source_filename.as_deref(), Some("renamed.png"));
    }

    #[test]
    fn a_mime_with_parameters_and_odd_case_is_accepted() {
        let item = AgentAttachment {
            mime: Some("Text/Plain; charset=utf-8".to_string()),
            ..inline_item(b"hello", "notes")
        };

        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();

        assert_eq!(staged.mime.as_deref(), Some("text/plain"));
    }

    #[test]
    fn a_pdf_declared_by_mime_is_accepted() {
        let item = AgentAttachment {
            mime: Some("application/pdf".to_string()),
            ..inline_item(PDF_BYTES, "doc")
        };

        assert!(resolve_agent_attachment(0, &item, &[]).is_ok());
    }

    #[test]
    fn a_missing_file_is_refused_without_echoing_the_path() {
        let dir = TestDir::new("agent-item-missing");
        let missing = dir.path().join("nope.png");

        let e = refusal(&path_item(&missing));

        assert_eq!(e, "attachments[3]: could not read the file at path");
    }

    #[test]
    fn a_missing_file_and_a_directory_get_the_same_message() {
        let dir = TestDir::new("agent-item-oracle");
        let missing = dir.path().join("x.png");
        let as_dir = dir.path().join("dir").join("x.png");
        std::fs::create_dir_all(&as_dir).unwrap();

        assert_eq!(refusal(&path_item(&missing)), refusal(&path_item(&as_dir)));
    }

    #[test]
    fn an_unsupported_name_is_refused_the_same_whether_or_not_the_file_exists() {
        let dir = TestDir::new("agent-item-name-oracle");
        let present = file_in(&dir, "tool.exe", b"MZ\x90\x00");
        let absent = dir.path().join("gone").join("tool.exe");

        let present_err = refusal(&path_item(&present));

        assert_eq!(present_err, refusal(&path_item(&absent)));
        assert!(
            present_err.contains("unsupported attachment type"),
            "{present_err}"
        );
    }

    #[test]
    fn content_that_does_not_match_the_name_is_still_told_apart() {
        let dir = TestDir::new("agent-item-sniff");
        let p = file_in(&dir, "shot.png", b"just words");

        assert_refused(&path_item(&p), "content does not look like image/png");
    }

    #[test]
    fn a_filename_cannot_launder_a_file_with_no_accepted_extension() {
        let dir = TestDir::new("agent-item-launder");
        let p = file_in(&dir, "credentials", b"aws_secret_access_key = x");
        let item = AgentAttachment {
            filename: Some("x.txt".to_string()),
            ..path_item(&p)
        };

        assert_refused(
            &item,
            "unsupported attachment type: mime=application/octet-stream",
        );
    }

    #[test]
    fn a_mime_that_disagrees_with_the_real_name_is_refused() {
        let dir = TestDir::new("agent-item-mime-mismatch");
        let p = file_in(&dir, "notes.txt", b"just words");
        let item = AgentAttachment {
            mime: Some("image/png".to_string()),
            ..path_item(&p)
        };

        assert_refused(
            &item,
            "mime image/png does not match the file's type text/plain",
        );
    }

    #[test]
    fn a_mime_that_agrees_with_the_real_name_is_accepted() {
        let dir = TestDir::new("agent-item-mime-agrees");
        let p = file_in(&dir, "shot.png", PNG_BYTES);
        let item = AgentAttachment {
            mime: Some("Image/PNG".to_string()),
            ..path_item(&p)
        };

        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();

        assert_eq!(staged.mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn a_filename_on_the_path_form_names_the_manifest_entry_and_nothing_else() {
        let dir = TestDir::new("agent-item-display-name");
        let p = file_in(&dir, "shot.png", PNG_BYTES);
        let item = AgentAttachment {
            filename: Some("anything.txt".to_string()),
            ..path_item(&p)
        };

        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();

        assert_eq!(staged.mime.as_deref(), Some("image/png"));
        assert_eq!(staged.source_filename.as_deref(), Some("anything.txt"));
    }

    #[test]
    fn a_directory_is_refused() {
        let dir = TestDir::new("agent-item-dir");
        let as_dir = dir.path().join("x.png");
        std::fs::create_dir_all(&as_dir).unwrap();

        assert_refused(&path_item(&as_dir), "could not read the file at path");
    }

    #[test]
    fn a_relative_path_is_refused() {
        let item = AgentAttachment {
            path: Some(
                Path::new("shots")
                    .join("a.png")
                    .to_string_lossy()
                    .into_owned(),
            ),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "path must be absolute");
    }

    #[test]
    fn an_empty_file_is_refused() {
        let dir = TestDir::new("agent-item-empty");
        let p = file_in(&dir, "empty.png", b"");

        assert_refused(&path_item(&p), "attachment bytes are empty");
    }

    #[test]
    fn an_empty_inline_string_counts_as_absent() {
        let item = AgentAttachment {
            content_base64: Some("  ".to_string()),
            filename: Some("a.png".to_string()),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "exactly one of path or content_base64");
    }

    #[test]
    fn a_file_over_the_limit_is_refused_from_its_length_alone() {
        let dir = TestDir::new("agent-item-big");
        let p = dir.path().join("big.png");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(MAX_ATTACHMENT_BYTES + 1).unwrap();

        assert_refused(
            &path_item(&p),
            &format!(
                "attachment too large: {} bytes (max {})",
                MAX_ATTACHMENT_BYTES + 1,
                MAX_ATTACHMENT_BYTES
            ),
        );
    }

    #[test]
    fn the_encoded_length_bound_sits_where_the_decoded_limit_does() {
        let bound = 4 * MAX_ATTACHMENT_BYTES.div_ceil(3);

        assert!(!inline_encoded_len_exceeds_limit(0));
        assert!(!inline_encoded_len_exceeds_limit(bound));
        assert!(inline_encoded_len_exceeds_limit(bound + 1));
        assert!(!inline_encoded_len_exceeds_limit(MAX_ATTACHMENT_BYTES));
    }

    #[test]
    fn an_oversize_inline_string_is_refused_before_decoding() {
        let bound = 4 * MAX_ATTACHMENT_BYTES.div_ceil(3);
        let item = AgentAttachment {
            content_base64: Some("A".repeat(bound as usize + 4)),
            filename: Some("a.png".to_string()),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "content_base64 is too large");
    }

    #[test]
    fn an_exe_is_refused_even_with_valid_content() {
        let dir = TestDir::new("agent-item-exe");
        let p = file_in(&dir, "tool.exe", b"MZ\x90\x00");

        assert_refused(&path_item(&p), "unsupported attachment type");
    }

    #[test]
    fn a_zip_mime_is_refused() {
        let item = AgentAttachment {
            mime: Some("application/zip".to_string()),
            ..inline_item(b"PK\x03\x04", "a.zip")
        };

        assert_refused(&item, "unsupported attachment type: mime=application/zip");
    }

    #[test]
    fn octet_stream_is_not_rescued_by_a_png_extension() {
        let item = AgentAttachment {
            mime: Some("application/octet-stream".to_string()),
            ..inline_item(PNG_BYTES, "shot.png")
        };

        assert_refused(
            &item,
            "unsupported attachment type: mime=application/octet-stream ext=png",
        );
    }

    #[test]
    fn text_declared_as_an_image_is_refused() {
        let item = AgentAttachment {
            mime: Some("image/png".to_string()),
            ..inline_item(b"just words", "a.png")
        };

        assert_refused(&item, "content does not look like image/png");
    }

    #[test]
    fn png_bytes_declared_as_pdf_are_refused() {
        let item = AgentAttachment {
            mime: Some("application/pdf".to_string()),
            ..inline_item(PNG_BYTES, "a.pdf")
        };

        assert_refused(&item, "content does not look like application/pdf");
    }

    #[test]
    fn text_with_a_nul_byte_is_refused() {
        assert_refused(
            &inline_item(b"abc\0def", "a.txt"),
            "content does not look like text/plain",
        );
    }

    #[test]
    fn invalid_base64_is_refused() {
        let item = AgentAttachment {
            content_base64: Some("not base64 !!".to_string()),
            filename: Some("a.png".to_string()),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "content_base64 is not valid base64");
    }

    #[test]
    fn url_safe_base64_is_refused() {
        let bytes = [0xfbu8, 0xff, 0xfe, 0x89];
        let url_safe = base64::engine::general_purpose::URL_SAFE.encode(bytes);
        assert!(url_safe.contains('_') || url_safe.contains('-'));
        let item = AgentAttachment {
            content_base64: Some(url_safe),
            filename: Some("a.png".to_string()),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "content_base64 is not valid base64");
    }

    #[test]
    fn unpadded_base64_is_refused() {
        let item = AgentAttachment {
            content_base64: Some(base64::engine::general_purpose::STANDARD_NO_PAD.encode(b"ab")),
            filename: Some("a.txt".to_string()),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "content_base64 is not valid base64");
    }

    #[test]
    fn neither_and_both_fields_are_refused() {
        let dir = TestDir::new("agent-item-both");
        let p = file_in(&dir, "a.png", PNG_BYTES);
        let both = AgentAttachment {
            content_base64: Some(std_b64(PNG_BYTES)),
            ..path_item(&p)
        };

        assert_refused(
            &AgentAttachment::default(),
            "exactly one of path or content_base64",
        );
        assert_refused(&both, "exactly one of path or content_base64");
    }

    #[test]
    fn inline_without_a_filename_is_refused() {
        let item = AgentAttachment {
            content_base64: Some(std_b64(PNG_BYTES)),
            ..AgentAttachment::default()
        };

        assert_refused(&item, "filename is required");
    }

    #[test]
    fn the_index_in_the_prefix_is_the_one_passed() {
        let e = resolve_agent_attachment(7, &AgentAttachment::default(), &[]).unwrap_err();

        assert!(e.starts_with("attachments[7]: "), "{e}");
    }
}

mod agent_batch {
    use super::agent_item::{inline_item, path_item};
    use super::*;
    use crate::support::test_dir::TestDir;

    fn distinct_png(n: u8) -> AgentAttachment {
        let mut bytes = PNG_BYTES.to_vec();
        bytes.push(n);
        inline_item(&bytes, &format!("shot-{n}.png"))
    }

    #[test]
    fn an_empty_list_resolves_to_an_empty_batch() {
        assert!(resolve_agent_attachments(Vec::new(), &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn the_same_bytes_twice_resolve_to_one_item_keeping_the_first() {
        let items = vec![
            inline_item(PNG_BYTES, "first.png"),
            inline_item(PNG_BYTES, "second.png"),
        ];

        let staged = resolve_agent_attachments(items, &[]).unwrap();

        assert_eq!(staged.len(), 1);
        assert_eq!(staged[0].source_filename.as_deref(), Some("first.png"));
    }

    #[test]
    fn eleven_distinct_items_are_refused_without_an_index() {
        let items = (0..11).map(distinct_png).collect();

        let e = resolve_agent_attachments(items, &[]).unwrap_err();

        assert!(e.contains("11 attachments (max 10)"), "{e}");
        assert!(!e.contains("attachments["), "{e}");
    }

    #[test]
    fn ten_distinct_items_are_accepted() {
        let items = (0..10).map(distinct_png).collect();

        assert_eq!(resolve_agent_attachments(items, &[]).unwrap().len(), 10);
    }

    #[test]
    fn eleven_items_that_dedup_to_ten_are_accepted() {
        let mut items: Vec<_> = (0..10).map(distinct_png).collect();
        items.push(distinct_png(3));

        assert_eq!(resolve_agent_attachments(items, &[]).unwrap().len(), 10);
    }

    #[test]
    fn one_valid_and_one_invalid_item_refuse_everything() {
        let items = vec![
            inline_item(PNG_BYTES, "ok.png"),
            inline_item(b"MZ\x90\x00", "tool.exe"),
        ];

        let e = resolve_agent_attachments(items, &[]).unwrap_err();

        assert!(e.starts_with("attachments[1]: "), "{e}");
    }

    #[test]
    fn the_first_error_wins() {
        let items = vec![
            AgentAttachment::default(),
            inline_item(b"MZ\x90\x00", "tool.exe"),
        ];

        let e = resolve_agent_attachments(items, &[]).unwrap_err();

        assert!(e.starts_with("attachments[0]: "), "{e}");
    }

    #[test]
    fn a_path_that_overflows_the_batch_cap_is_refused_after_one_byte_past_it() {
        let dir = TestDir::new("agent-batch-cap");
        let p = dir.path().join("big.png");
        let f = std::fs::File::create(&p).unwrap();
        f.set_len(20).unwrap();

        let e = resolve_agent_attachments_capped(vec![path_item(&p)], &[], 10).unwrap_err();

        assert!(e.contains("too large together: 11 bytes (max 10)"), "{e}");
        assert!(!e.contains("attachments["), "{e}");
    }

    #[test]
    fn inline_items_accumulate_toward_the_batch_cap() {
        let cap = PNG_BYTES.len() as u64 + 1;
        let items = vec![distinct_png(1), distinct_png(2)];

        let e = resolve_agent_attachments_capped(items, &[], cap).unwrap_err();

        assert!(e.contains("too large together"), "{e}");
    }

    #[test]
    fn duplicates_count_toward_the_batch_cap_because_each_is_read() {
        let cap = PNG_BYTES.len() as u64;
        let items = vec![
            inline_item(PNG_BYTES, "a.png"),
            inline_item(PNG_BYTES, "b.png"),
        ];

        let e = resolve_agent_attachments_capped(items, &[], cap).unwrap_err();

        assert!(e.contains("too large together"), "{e}");
    }

    #[test]
    fn more_items_than_the_ceiling_are_refused_before_any_file_is_opened() {
        let dir = TestDir::new("agent-batch-ceiling");
        let gone = dir.path().join("gone.png");
        let items = (0..=MAX_AGENT_ATTACHMENT_ITEMS)
            .map(|_| path_item(&gone))
            .collect();

        let e = resolve_agent_attachments(items, &[]).unwrap_err();

        assert!(e.contains("too many attachment items"), "{e}");
        assert!(!e.contains("could not stat"), "{e}");
    }

    #[test]
    fn repeating_one_file_is_bounded_by_the_bytes_read_budget() {
        let dir = TestDir::new("agent-batch-repeat");
        let p = dir.path().join("big.png");
        let mut bytes = PNG_BYTES.to_vec();
        bytes.resize(100, 0);
        std::fs::write(&p, &bytes).unwrap();
        let items = (0..4).map(|_| path_item(&p)).collect();

        let e = resolve_agent_attachments_capped(items, &[], 250).unwrap_err();

        assert!(e.contains("too large together: 251 bytes (max 250)"), "{e}");
    }

    #[test]
    fn the_eleventh_distinct_item_stops_the_batch_before_later_items_are_read() {
        let dir = TestDir::new("agent-batch-eleventh");
        let gone = dir.path().join("gone.png");
        let mut items: Vec<_> = (0..11).map(distinct_png).collect();
        items.push(path_item(&gone));

        let e = resolve_agent_attachments(items, &[]).unwrap_err();

        assert!(e.contains("11 attachments (max 10)"), "{e}");
        assert!(!e.contains("could not stat"), "{e}");
    }

    #[test]
    fn a_padded_path_is_counted_and_read_as_the_trimmed_path() {
        let dir = TestDir::new("agent-batch-trim");
        let p = dir.path().join("shot.png");
        std::fs::write(&p, PNG_BYTES).unwrap();
        let padded = AgentAttachment {
            path: Some(format!("  {}\n", p.display())),
            ..AgentAttachment::default()
        };

        let e = resolve_agent_attachments_capped(vec![padded.clone()], &[], 5).unwrap_err();
        assert!(e.contains("too large together: 6 bytes (max 5)"), "{e}");

        let staged = resolve_agent_attachments(vec![padded], &[]).unwrap();
        assert_eq!(staged[0].bytes.as_deref(), Some(PNG_BYTES));
        assert_eq!(staged[0].source_filename.as_deref(), Some("shot.png"));
    }

    fn virtual_png_path(dir: &TestDir, name: &str) -> AgentAttachment {
        let p = dir.path().join(name);
        std::fs::write(&p, PNG_BYTES).unwrap();
        path_item(&p)
    }

    #[test]
    fn a_path_whose_stat_is_zero_is_charged_the_bytes_its_read_yielded() {
        let dir = TestDir::new("agent-batch-virtual");
        let items: Vec<_> = ["first.png", "second.png", "third.png"]
            .iter()
            .map(|name| {
                let p = dir.path().join(name);
                std::fs::File::create(&p).unwrap();
                assert_eq!(std::fs::metadata(&p).unwrap().len(), 0);
                path_item(&p)
            })
            .collect();
        let third_called = std::cell::Cell::new(false);
        let read = |path: &Path, _: &str, _: u64| -> Result<Vec<u8>, String> {
            third_called.set(third_called.get() || path.ends_with("third.png"));
            let mut bytes = PNG_BYTES.to_vec();
            bytes.resize(100, 0);
            Ok(bytes)
        };

        let e = resolve_agent_attachments_with(items, 150, &read).unwrap_err();

        assert!(e.contains("too large together: 200 bytes (max 150)"), "{e}");
        assert!(!third_called.get(), "a refused batch read a later item");
    }

    #[test]
    fn a_read_is_handed_only_what_is_left_of_the_batch() {
        let dir = TestDir::new("agent-batch-remaining");
        let items = vec![
            virtual_png_path(&dir, "a.png"),
            virtual_png_path(&dir, "b.png"),
        ];
        let limits = std::cell::RefCell::new(Vec::new());
        let read = |_: &Path, _: &str, limit: u64| -> Result<Vec<u8>, String> {
            limits.borrow_mut().push(limit);
            Ok(PNG_BYTES.to_vec())
        };
        let cap = PNG_BYTES.len() as u64 * 2 + 5;

        resolve_agent_attachments_with(items, cap, &read).unwrap();

        assert_eq!(
            *limits.borrow(),
            vec![
                MAX_ATTACHMENT_BYTES.min(cap),
                MAX_ATTACHMENT_BYTES.min(cap - PNG_BYTES.len() as u64)
            ]
        );
    }

    #[test]
    fn a_read_past_the_per_file_limit_keeps_the_per_file_wording() {
        let dir = TestDir::new("agent-batch-perfile");
        let item = virtual_png_path(&dir, "huge.png");
        let read = |_: &Path, _: &str, _: u64| -> Result<Vec<u8>, String> {
            Ok(vec![0u8; MAX_ATTACHMENT_BYTES as usize + 1])
        };

        let e = resolve_agent_attachments_with(vec![item], u64::MAX, &read).unwrap_err();

        assert!(e.starts_with("attachments[0]: attachment too large"), "{e}");
    }

    #[tokio::test]
    async fn the_resolved_batch_commits_as_one_attached_file() {
        let ctx = fixture("agent-batch-roundtrip");
        feature_in(&ctx, "f-rt");
        let items = vec![
            inline_item(PNG_BYTES, "shot.png"),
            inline_item(PNG_BYTES, "again.png"),
        ];

        let staged = resolve_agent_attachments(items, &[]).unwrap();
        let committed = commit_staged_attachments(
            &ctx.features,
            &ctx.attachment_json,
            &ctx.attachments,
            "f-rt",
            staged,
        )
        .unwrap();

        let listed = ctx
            .attachment_json
            .get_attachments(&FeatureId::from("f-rt".to_string()))
            .unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].mime, "image/png");
        assert_eq!(listed[0].sha256, compute_sha256_hex(PNG_BYTES));
        assert_eq!(listed[0].size, PNG_BYTES.len() as u64);
    }
}

mod agent_type_policy {
    use super::*;

    fn path_type(basename: &str, supplied: Option<&str>) -> Result<String, String> {
        agent_attachment_mime(AgentSource::Path { basename }, supplied)
    }

    fn inline_type(filename: &str, supplied: Option<&str>) -> Result<String, String> {
        agent_attachment_mime(AgentSource::Inline { filename }, supplied)
    }

    #[test]
    fn the_path_form_takes_its_type_from_the_basename_alone() {
        assert_eq!(path_type("shot.png", None).unwrap(), "image/png");
        assert_eq!(
            path_type("shot.png", Some("image/png")).unwrap(),
            "image/png"
        );
        assert!(path_type("credentials", None)
            .unwrap_err()
            .contains("unsupported attachment type"));
        assert!(path_type("notes.txt", Some("image/png"))
            .unwrap_err()
            .contains("does not match the file's type text/plain"));
    }

    #[test]
    fn the_inline_form_lets_the_declared_mime_win() {
        assert_eq!(
            inline_type("notes", Some("text/plain")).unwrap(),
            "text/plain"
        );
        assert_eq!(
            inline_type("a.txt", Some("image/png")).unwrap(),
            "image/png"
        );
        assert!(inline_type("notes", None)
            .unwrap_err()
            .contains("unsupported attachment type"));
    }

    #[test]
    fn content_is_judged_against_the_resolved_mime() {
        assert!(check_agent_content("image/png", PNG_BYTES).is_ok());
        assert_eq!(
            check_agent_content("image/png", b"words").unwrap_err(),
            "content does not look like image/png"
        );
    }
}

mod agent_read_bound {
    use super::*;

    struct Counting<R> {
        inner: R,
        served: u64,
    }

    impl<R: Read> Read for Counting<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.served += n as u64;
            Ok(n)
        }
    }

    #[test]
    fn a_source_that_never_ends_is_refused_after_one_byte_past_the_limit() {
        let mut source = Counting {
            inner: std::io::repeat(b'a'),
            served: 0,
        };

        let bytes = read_up_to_one_past(&mut source, 64).unwrap();

        assert_eq!(bytes.len(), 65);
        assert_eq!(source.served, 65);
    }

    #[test]
    fn a_source_of_exactly_the_limit_is_read_whole() {
        let bytes = read_up_to_one_past(std::io::Cursor::new(vec![7u8; 64]), 64).unwrap();

        assert_eq!(bytes.len(), 64);
    }
}

mod agent_forbidden_roots {
    use super::agent_item::path_item;
    use super::*;
    use crate::support::test_dir::TestDir;

    fn file_under(dir: &Path, parts: &[&str], name: &str, bytes: &[u8]) -> PathBuf {
        let mut at = dir.to_path_buf();
        for part in parts {
            at.push(part);
        }
        std::fs::create_dir_all(&at).unwrap();
        let file = at.join(name);
        std::fs::write(&file, bytes).unwrap();
        file
    }

    fn refusal(item: &AgentAttachment, forbidden: &[PathBuf]) -> String {
        resolve_agent_attachment(0, item, forbidden).unwrap_err()
    }

    #[test]
    fn a_path_inside_a_forbidden_root_is_refused() {
        let data = TestDir::new("agent-forbidden-data");
        let p = file_under(data.path(), &["attachments", "f-1"], "notes.txt", b"secret");

        let e = refusal(&path_item(&p), &[data.path().to_path_buf()]);

        assert!(e.contains("Demeteo's own data directory"), "{e}");
    }

    #[test]
    fn a_path_outside_every_forbidden_root_is_accepted() {
        let data = TestDir::new("agent-forbidden-data");
        let elsewhere = TestDir::new("agent-forbidden-elsewhere");
        let p = file_under(elsewhere.path(), &[], "shot.png", PNG_BYTES);

        assert!(resolve_agent_attachment(0, &path_item(&p), &[data.path().to_path_buf()]).is_ok());
    }

    #[test]
    fn a_sibling_that_shares_the_roots_name_as_a_prefix_is_not_inside_it() {
        let parent = TestDir::new("agent-forbidden-sibling");
        let root = parent.path().join("data");
        std::fs::create_dir_all(&root).unwrap();
        let p = file_under(parent.path(), &["data-backup"], "shot.png", PNG_BYTES);

        assert!(resolve_agent_attachment(0, &path_item(&p), &[root]).is_ok());
    }

    #[test]
    fn dot_dot_segments_cannot_walk_into_a_forbidden_root() {
        let data = TestDir::new("agent-forbidden-dotdot");
        let outside = TestDir::new("agent-forbidden-dotdot-out");
        file_under(data.path(), &[], "notes.txt", b"secret");
        let walked = outside
            .path()
            .join("..")
            .join(data.path().file_name().unwrap())
            .join("notes.txt");

        let e = refusal(&path_item(&walked), &[data.path().to_path_buf()]);

        assert!(e.contains("Demeteo's own data directory"), "{e}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_from_outside_into_a_forbidden_root_is_refused() {
        let data = TestDir::new("agent-forbidden-link-data");
        let outside = TestDir::new("agent-forbidden-link-out");
        let target = file_under(data.path(), &[], "notes.txt", b"secret");
        let link = outside.path().join("innocent.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let e = refusal(&path_item(&link), &[data.path().to_path_buf()]);

        assert!(e.contains("Demeteo's own data directory"), "{e}");
    }

    #[test]
    fn a_workspace_inside_the_data_dir_keeps_its_mockups_but_not_the_stores() {
        let data = TestDir::new("agent-forbidden-default");
        let forbidden = agent_attachment_forbidden_roots(data.path(), data.path());
        let mockup = file_under(data.path(), &["repos", "app"], "mock.png", PNG_BYTES);
        let staged = file_under(data.path(), &["attachments", "f-1"], "notes.txt", b"x");
        let artifact = file_under(data.path(), &["artifacts", "f-1"], "report.json", b"{}");
        let db = file_under(data.path(), &[], "demeteo.db-journal", b"x");

        assert!(resolve_agent_attachment(0, &path_item(&mockup), &forbidden).is_ok());
        for denied in [staged, artifact] {
            assert!(
                refusal(&path_item(&denied), &forbidden).contains("data directory"),
                "{}",
                denied.display()
            );
        }
        let db_as_text = AgentAttachment {
            filename: Some("a.txt".to_string()),
            ..path_item(&db)
        };
        assert!(refusal(&db_as_text, &forbidden).starts_with("attachments[0]: "));
    }

    #[test]
    fn a_workspace_elsewhere_forbids_the_whole_data_dir() {
        let data = TestDir::new("agent-forbidden-split-data");
        let workspace = TestDir::new("agent-forbidden-split-ws");
        let forbidden = agent_attachment_forbidden_roots(data.path(), workspace.path());
        let in_data = file_under(data.path(), &["anything"], "shot.png", PNG_BYTES);
        let in_workspace = file_under(workspace.path(), &["app"], "mock.png", PNG_BYTES);

        assert!(refusal(&path_item(&in_data), &forbidden).contains("data directory"));
        assert!(resolve_agent_attachment(0, &path_item(&in_workspace), &forbidden).is_ok());
    }

    #[test]
    fn a_workspace_nested_in_the_data_dir_is_not_mistaken_for_one_elsewhere() {
        let data = TestDir::new("agent-forbidden-nested");
        let workspace = data.path().join("workspaces");
        std::fs::create_dir_all(&workspace).unwrap();
        let forbidden = agent_attachment_forbidden_roots(data.path(), &workspace);
        let mockup = file_under(&workspace, &["app"], "mock.png", PNG_BYTES);

        assert!(resolve_agent_attachment(0, &path_item(&mockup), &forbidden).is_ok());
    }
}

#[cfg(unix)]
mod agent_symlink_type {
    use super::agent_item::path_item;
    use super::*;
    use crate::support::test_dir::TestDir;

    fn link(dir: &Path, name: &str, target: &Path) -> PathBuf {
        let link = dir.join(name);
        std::os::unix::fs::symlink(target, &link).unwrap();
        link
    }

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let file = dir.join(name);
        std::fs::write(&file, bytes).unwrap();
        file
    }

    #[test]
    fn a_txt_symlink_to_an_extensionless_file_is_refused_without_naming_the_target() {
        let dir = TestDir::new("agent-link-extless");
        let target = write(dir.path(), "credentials", b"aws_secret_access_key = x\n");
        let l = link(dir.path(), "notes.txt", &target);

        let e = resolve_agent_attachment(0, &path_item(&l), &[]).unwrap_err();

        assert_eq!(e, format!("attachments[0]: {UNREADABLE_PATH}"));
        assert!(!e.contains("credentials"), "{e}");
    }

    #[test]
    fn a_txt_symlink_to_a_png_named_png_is_refused() {
        let dir = TestDir::new("agent-link-png");
        let target = write(dir.path(), "real.png", PNG_BYTES);
        let l = link(dir.path(), "notes.txt", &target);

        let e = resolve_agent_attachment(0, &path_item(&l), &[]).unwrap_err();

        assert_eq!(e, format!("attachments[0]: {UNREADABLE_PATH}"));
    }

    #[test]
    fn a_txt_symlink_to_a_markdown_file_is_refused() {
        let dir = TestDir::new("agent-link-md");
        let target = write(dir.path(), "readme.md", b"# hi\n");
        let l = link(dir.path(), "notes.txt", &target);

        let e = resolve_agent_attachment(0, &path_item(&l), &[]).unwrap_err();

        assert_eq!(e, format!("attachments[0]: {UNREADABLE_PATH}"));
    }

    #[test]
    fn a_png_symlink_to_a_png_in_another_directory_attaches_the_targets_bytes() {
        let here = TestDir::new("agent-link-same-here");
        let there = TestDir::new("agent-link-same-there");
        let target = write(there.path(), "original.png", PNG_BYTES);
        let l = link(here.path(), "shot.png", &target);

        let staged = resolve_agent_attachment(0, &path_item(&l), &[]).unwrap();

        assert_eq!(staged.mime.as_deref(), Some("image/png"));
        assert_eq!(staged.bytes.as_deref(), Some(PNG_BYTES));
    }
}

mod agent_tiff_stored_name {
    use super::agent_item::{inline_item, path_item};
    use super::*;
    use crate::support::test_dir::TestDir;

    #[tokio::test]
    async fn a_tiff_by_path_is_stored_under_its_own_extension_whatever_the_filename_says() {
        let ctx = fixture("tiff-path");
        let dir = TestDir::new("agent-tiff-path");
        let src = dir.path().join("a.tiff");
        std::fs::write(&src, b"II*\0-not-a-real-tiff").unwrap();
        let item = AgentAttachment {
            filename: Some("anything.zzz".to_string()),
            ..path_item(&src)
        };

        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();
        feature_in(&ctx, "f-tiff");
        let file = commit_attachment_inner(
            &ctx.features,
            &ctx.attachment_json,
            &ctx.attachments,
            "f-tiff",
            &staged.source_path,
            staged.mime.as_deref(),
            staged.source_filename.as_deref(),
            staged.bytes,
        )
        .unwrap();

        assert_eq!(file.mime, "image/tiff");
        assert_eq!(stored_ext(&ctx, "f-tiff", &file), "tiff");
    }

    async fn stored_inline(tag: &str, item: AgentAttachment) -> String {
        let ctx = fixture(tag);
        let staged = resolve_agent_attachment(0, &item, &[]).unwrap();
        feature_in(&ctx, "f-tiff");
        let file = commit_attachment_inner(
            &ctx.features,
            &ctx.attachment_json,
            &ctx.attachments,
            "f-tiff",
            &staged.source_path,
            staged.mime.as_deref(),
            staged.source_filename.as_deref(),
            staged.bytes,
        )
        .unwrap();
        assert_eq!(file.mime, "image/tiff");
        stored_ext(&ctx, "f-tiff", &file)
    }

    #[tokio::test]
    async fn an_inline_tiff_is_stored_under_the_extension_its_filename_carries() {
        let tiff = b"II*\0-not-a-real-tiff";
        let named = inline_item(tiff, "a.tiff");
        let mislabelled = AgentAttachment {
            mime: Some("image/tiff".to_string()),
            ..inline_item(tiff, "x.zzz")
        };
        let bare = AgentAttachment {
            mime: Some("image/tiff".to_string()),
            ..inline_item(tiff, "scan")
        };

        assert_eq!(stored_inline("tiff-inline-named", named).await, "tiff");
        assert_eq!(stored_inline("tiff-inline-zzz", mislabelled).await, "zzz");
        assert_eq!(stored_inline("tiff-inline-bare", bare).await, "bin");
    }
}

#[cfg(unix)]
mod agent_non_regular_file {
    use super::agent_item::path_item;
    use super::*;
    use crate::support::test_dir::TestDir;

    /// `read_agent_path` stats before it opens because opening a FIFO with no
    /// writer blocks forever. Run on its own thread so a regression fails here
    /// instead of hanging the suite.
    #[test]
    fn a_fifo_named_like_an_accepted_type_is_refused_without_blocking() {
        let dir = TestDir::new("agent-fifo");
        let fifo = dir.path().join("notes.txt");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo should run");
        assert!(made.success());
        let item = path_item(&fifo);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(resolve_agent_attachment(0, &item, &[]));
        });
        let outcome = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("resolving a FIFO must not block on open");

        assert_eq!(
            outcome.unwrap_err(),
            format!("attachments[0]: {UNREADABLE_PATH}")
        );
    }
}
