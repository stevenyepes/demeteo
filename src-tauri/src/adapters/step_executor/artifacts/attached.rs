use crate::domain::artifact::{ArtifactCapture, ArtifactDecl};
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::FeatureId;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::permission::{PermissionProfile, StepCapability};
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::attachment_store::AttachmentStore;
use crate::ports::db::GateRepository;

/// Returns `(decision, feedback)` from the most recently *decided* gate step
/// for a feature.  Used to inject `{{gate_decision}}` and `{{gate_feedback}}`
/// into the next agent step's rendered prompt.
///
/// Best-effort: returns `("", "")` when no gate has been decided yet (the
/// common case for the first agent step in any workflow).
pub(crate) fn get_latest_gate_decision(
    gates: &dyn GateRepository,
    feature_id: &str,
) -> (String, String) {
    let f_id = FeatureId::from(feature_id.to_string());
    match gates.latest_decided_for_feature(&f_id) {
        Ok(Some(decided)) => (
            decided.decision.unwrap_or_default(),
            decided.feedback.unwrap_or_default(),
        ),
        _ => (String::new(), String::new()),
    }
}

/// How a referenced artifact step's body should be injected into the
/// next step's prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AttachmentMode {
    /// Emit a path manifest pointing at the on-disk file. The agent
    /// uses its `Read` tool on demand. Cheaper for vendor prompt
    /// caching; default for new workflow artifacts.
    PathManifest,
    /// Inline the file body verbatim (legacy behavior, opt-in per
    /// [`ArtifactDecl::inline`]).
    InlineBody,
}

fn mode_for_step(step_id: &str, step_confs: &[StepConfig]) -> AttachmentMode {
    if let Some(conf) = step_confs.iter().find(|c| c.id.0 == step_id) {
        if let Some(decls) = conf.artifacts.as_ref() {
            // If *any* declaration opts in to inline, inline everything
            // from that step — partial mixing within one step's
            // attachments would surprise workflow authors. Authors who
            // want fine-grained control can split into separate steps.
            if decls.iter().any(|d| d.inline) {
                return AttachmentMode::InlineBody;
            }
        }
    }
    AttachmentMode::PathManifest
}

fn render_path_manifest(step_id: &str, paths: &[String]) -> String {
    let mut lines = vec![
        format!(
            "The following artifacts from step `{}` are on disk:",
            step_id
        ),
        String::new(),
    ];
    for p in paths {
        lines.push(format!("- `{}`", p));
    }
    lines.push(String::new());
    lines.push(
        "Use your Read tool to load them on demand — the bodies are not inlined here so the \
         vendor prompt-cache prefix stays stable across steps."
            .to_string(),
    );
    lines.join("\n")
}

/// Resolve `[attached — <step_id>]` and `[attached — previous step artifact]`
/// placeholders inside a prompt template. For each referenced step the
/// function looks up the step's [`StepConfig::artifacts`]: if any
/// declaration has `inline: true`, the bodies are inlined verbatim; if
/// all declarations leave `inline: false` (the default), a path
/// manifest is emitted instead so the agent `Read`s on demand. This is
/// the cost-optimized default — see [`ArtifactDecl::inline`] for the
/// tradeoff.
pub(crate) fn resolve_attached_artifacts(
    prompt: &str,
    step_execs: &[StepExecution],
    step_index: usize,
    store: &dyn ArtifactStore,
    step_confs: &[StepConfig],
) -> String {
    let mut resolved_prompt = prompt.to_string();
    let mut search_start = 0;
    let mut attachments = Vec::new();

    while let Some(start_idx) = resolved_prompt[search_start..].find("[attached") {
        let absolute_start = search_start + start_idx;
        if let Some(end_offset) = resolved_prompt[absolute_start..].find(']') {
            let absolute_end = absolute_start + end_offset;
            let full_placeholder = resolved_prompt[absolute_start..=absolute_end].to_string();

            let inside = &full_placeholder[1..full_placeholder.len() - 1];

            let parts: Vec<&str> = if inside.contains('\u{2014}') {
                inside.split('\u{2014}').collect()
            } else if inside.contains('\u{2013}') {
                inside.split('\u{2013}').collect()
            } else {
                inside.split('-').collect()
            };

            if parts.len() >= 2 {
                let content = parts[1].trim();
                let mut replacement = String::new();

                if content == "previous step artifact" {
                    if step_index > 0 {
                        if let Some(prev_step) = step_execs.get(step_index - 1) {
                            let paths: Vec<String> = if !prev_step.artifact_paths.is_empty() {
                                prev_step.artifact_paths.clone()
                            } else {
                                prev_step
                                    .artifact_path
                                    .as_ref()
                                    .map(|p| vec![p.clone()])
                                    .unwrap_or_default()
                            };
                            let mode = mode_for_step(&prev_step.step_id.0, step_confs);
                            let body = render_attachment_body(
                                &prev_step.step_id.0,
                                &paths,
                                mode.clone(),
                                store,
                            );
                            attachments.push((
                                prev_step.step_index as usize,
                                prev_step.step_id.0.clone(),
                                mode,
                                body,
                            ));
                            replacement = format!(
                                "[See attached {} at the beginning of the prompt]",
                                prev_step.step_id.0
                            );
                        }
                    } else {
                        replacement = "(No previous step exists)".to_string();
                    }
                } else {
                    let mut matched: Option<(usize, String, Vec<String>)> = None;

                    for s in step_execs {
                        let sid = s.step_id.0.to_lowercase();
                        let content_lower = content.to_lowercase();

                        if content_lower.contains(&sid) || sid.contains(&content_lower) {
                            let paths: Vec<String> = if !s.artifact_paths.is_empty() {
                                s.artifact_paths.clone()
                            } else {
                                s.artifact_path
                                    .as_ref()
                                    .map(|p| vec![p.clone()])
                                    .unwrap_or_default()
                            };
                            if !paths.is_empty() {
                                matched = Some((s.step_index as usize, s.step_id.0.clone(), paths));
                                break;
                            }
                        }
                    }

                    if let Some((step_idx, step_id, paths)) = matched {
                        let mode = mode_for_step(&step_id, step_confs);
                        let body = render_attachment_body(&step_id, &paths, mode.clone(), store);
                        attachments.push((step_idx, step_id.clone(), mode, body));
                        replacement =
                            format!("[See attached {} at the beginning of the prompt]", step_id);
                    } else {
                        replacement =
                            format!("(Artifact '{}' not found or not yet generated)", content);
                    }
                }

                resolved_prompt = resolved_prompt.replace(&full_placeholder, &replacement);
                search_start = 0;
                continue;
            }
        }
        search_start += start_idx + 1;
    }

    if !attachments.is_empty() {
        attachments.sort_by_key(|a| a.0);

        let mut prepended = String::new();
        for (_, step_id, mode, content) in attachments {
            prepended.push_str(&format!(
                "=== ATTACHED CONTEXT: {} ({}) ===\n{}\n================================\n\n",
                step_id,
                match mode {
                    AttachmentMode::PathManifest => "path manifest",
                    AttachmentMode::InlineBody => "inlined body",
                },
                content
            ));
        }
        resolved_prompt = format!("{}{}", prepended, resolved_prompt);
    }

    resolved_prompt
}

/// Resolve `[attachment — <name>]` placeholders in a prompt template
/// against a feature's per-run attachment manifest. Each match is
/// prepended to the prompt as a path-manifest block pointing at the
/// on-disk file under `<attachments_root>/<feature_id>/<sha256>.<ext>`.
/// The companion `spawn` step copies each matched file into the
/// per-step worktree's `artifacts/_context/attachments/` directory
/// so the agent's `external_directory: deny` fence accepts the file
/// when it calls `Read`.
///
/// This is split out from [`resolve_attached_artifacts`] so the
/// existing step-artifact substitution (and its existing tests)
/// remain stable — `[attached — <step_id>]` and `[attachment — <name>]`
/// placeholders are matched by *different* opening tokens (`[attached`
/// vs `[attachment`) so they live in independent scans. Unmatched
/// attachment names get the same "(Artifact '…' not found or not
/// yet generated)" message that step-artifact misses do.
///
/// **Fallback notice.** When a feature has one or more attachments but
/// the template does not reference any of them by name (a common case
/// for workflows whose plan/implement templates don't include
/// `[attachment — <name>]` placeholders), the agent has no way to know
/// the files exist. Append a short "user attached files" footer at
/// the end of the rendered prompt so the agent at least sees the
/// attachment manifest and can decide whether to `Read` the file on
/// demand. This is a non-blocking safety net — the placeholder path is
/// still preferred for templates that want to inline the file body.
pub(crate) fn resolve_attached_user_attachments(
    prompt: &str,
    feature_id: &str,
    attachments: &[AttachedFile],
    attachment_store: &dyn AttachmentStore,
    worktree_artifacts_dir: Option<&str>,
) -> String {
    if attachments.is_empty() {
        return prompt.to_string();
    }
    let mut resolved = prompt.to_string();
    let mut search = 0usize;
    let mut rendered: Vec<(usize, String, String)> = Vec::new(); // (sort_key, step_id, body)
    let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();

    while let Some(start_idx) = resolved[search..].find("[attachment") {
        let absolute_start = search + start_idx;
        // The closing `]` belongs to the placeholder; anything that
        // looks like `[attachment - X]` or `[attachment — X]` counts.
        let end_offset = match resolved[absolute_start..].find(']') {
            Some(o) => o,
            None => break,
        };
        let absolute_end = absolute_start + end_offset;
        let full = resolved[absolute_start..=absolute_end].to_string();
        let inside = &full[1..full.len() - 1];

        let parts: Vec<&str> = if inside.contains('\u{2014}') {
            inside.split('\u{2014}').collect()
        } else if inside.contains('\u{2013}') {
            inside.split('\u{2013}').collect()
        } else {
            inside.split('-').collect()
        };
        let matched = parts.len() >= 2;
        let content = if matched { parts[1].trim() } else { "" };

        let lc = content.to_lowercase();
        let found = attachments.iter().find(|a| {
            let a_name = a.name.to_lowercase();
            let a_id = a.id.to_lowercase();
            let a_src = a.source_filename.to_lowercase();
            lc == a_name || lc == a_id || lc == a_src
        });

        match found {
            Some(att) => {
                let ext = crate::domain::attachment::ext_for_mime(&att.mime)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        std::path::Path::new(&att.source_filename)
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_ascii_lowercase())
                            .unwrap_or_else(|| "bin".to_string())
                    });
                let stored = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
                let stored_str = stored.to_string_lossy().to_string();
                // If a worktree context dir is provided, the
                // `materialize_user_attachments_to_worktree` step has
                // already copied the file into
                // `{wt}/artifacts/_context/attachments/<sha>.<ext>`;
                // prefer that relative destination so the
                // `external_directory: deny` fence accepts the read.
                let display_path = if let Some(wt_dir) = worktree_artifacts_dir {
                    let rel = std::path::Path::new(wt_dir)
                        .join("attachments")
                        .join(format!("{}.{}", att.sha256, ext));
                    rel.to_string_lossy().to_string()
                } else {
                    stored_str.clone()
                };
                let body = format!(
                    "The following attachment `{name}` ({mime}, {size} bytes) is on disk:\n\n- `{path}`\n\nUse your Read tool to load it on demand.",
                    name = att.name,
                    mime = att.mime,
                    size = att.size,
                    path = display_path,
                );
                referenced.insert(att.sha256.clone());
                rendered.push((
                    // Use a high sort key so user attachments always
                    // trail real step artifacts.
                    usize::MAX - 1,
                    format!("attachment:{}", att.name),
                    body.clone(),
                ));
                let replacement =
                    format!("[See attached {} at the beginning of the prompt]", att.name);
                resolved = resolved.replace(&full, &replacement);
                search = 0;
            }
            None if matched => {
                let replacement = format!(
                    "(Artifact 'attachment {}' not found or not yet generated)",
                    content
                );
                resolved = resolved.replace(&full, &replacement);
                search = 0;
            }
            _ => {
                // `[attachment` substring that's not the placeholder
                // shape — leave it untouched and advance.
                search = absolute_start + 1;
            }
        }
    }

    if !rendered.is_empty() {
        rendered.sort_by_key(|r| r.0);
        let mut prepended = String::new();
        for (_, step_id, body) in rendered {
            prepended.push_str(&format!(
                "=== ATTACHED CONTEXT: {} (path manifest) ===\n{}\n================================\n\n",
                step_id, body
            ));
        }
        resolved = format!("{}{}", prepended, resolved);
    }

    // Fallback: surface any attachments that the template didn't
    // reference via a `[attachment — <name>]` placeholder. Without
    // this, a workflow whose plan/implement prompt doesn't mention
    // attachments leaves the agent blind to the user's files — the
    // file is on disk but the agent has no signal it exists. We
    // append a short footer naming every un-referenced attachment
    // and pointing at its on-disk path; the agent can then `Read`
    // the file on demand if the task appears to call for it.
    let unreferenced: Vec<&AttachedFile> = attachments
        .iter()
        .filter(|a| !referenced.contains(&a.sha256))
        .collect();
    if !unreferenced.is_empty() {
        let mut footer = String::from(
            "\n\n---\n\n## User Attached Files (not referenced by template)\n\n\
             The user attached the following file(s) to this feature but the workflow \
             template did not reference them by name. They are available on disk at the \
             paths below — use your Read tool to inspect them if the task appears to \
             call for it (e.g. a screenshot referenced in the description, a spec \
             document, etc.):\n",
        );
        for att in &unreferenced {
            let ext = crate::domain::attachment::ext_for_mime(&att.mime)
                .map(str::to_string)
                .unwrap_or_else(|| {
                    std::path::Path::new(&att.source_filename)
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_else(|| "bin".to_string())
                });
            let display_path = if let Some(wt_dir) = worktree_artifacts_dir {
                let rel = std::path::Path::new(wt_dir)
                    .join("attachments")
                    .join(format!("{}.{}", att.sha256, ext));
                rel.to_string_lossy().to_string()
            } else {
                let stored = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
                stored.to_string_lossy().to_string()
            };
            footer.push_str(&format!(
                "\n- `{name}` ({mime}, {size} bytes) — `{path}`",
                name = att.source_filename,
                mime = att.mime,
                size = att.size,
                path = display_path,
            ));
        }
        resolved.push_str(&footer);
    }

    resolved
}

/// Copy each user attachment into `{wt_path}/artifacts/_context/attachments/`
/// so the agent's `external_directory: deny` accepts the file when its
/// `Read` tool is called on it. Idempotent: re-running with the same
/// `(sha256, ext)` is a no-op when the destination already exists.
/// Logs a warning when the on-disk size differs from the recorded
/// `size` (sha256 hash mismatch is the most likely cause).
pub(crate) fn materialize_user_attachments_to_worktree(
    feature_id: &str,
    attachments: &[AttachedFile],
    attachment_store: &dyn AttachmentStore,
    wt_path: &str,
) -> Vec<String> {
    if attachments.is_empty() {
        return Vec::new();
    }
    let dest_root = std::path::Path::new(wt_path)
        .join("artifacts")
        .join("_context")
        .join("attachments");
    if std::fs::create_dir_all(&dest_root).is_err() {
        return Vec::new();
    }

    let mut copied = Vec::with_capacity(attachments.len());
    for att in attachments {
        let ext = crate::domain::attachment::ext_for_mime(&att.mime)
            .map(str::to_string)
            .unwrap_or_else(|| {
                std::path::Path::new(&att.source_filename)
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_else(|| "bin".to_string())
            });
        let src_path = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
        if !src_path.exists() {
            tracing::warn!(
                feature_id = feature_id,
                sha256 = %att.sha256,
                "user attachment source file is missing on disk; skipping pre-spawn copy"
            );
            continue;
        }
        let dest = dest_root.join(format!("{}.{}", att.sha256, ext));
        if dest.exists() {
            // Idempotent re-run: if the destination already exists,
            // sanity-check the size. A mismatch is the sha256-collision
            // (in practice impossible) or a stale-file bug — log
            // loudly and keep the on-disk bytes (the user's content
            // is safe).
            let src_meta = std::fs::metadata(&src_path).ok();
            let dst_meta = std::fs::metadata(&dest).ok();
            match (src_meta, dst_meta) {
                (Some(s), Some(d)) if s.len() != d.len() => {
                    tracing::warn!(
                        feature_id = feature_id,
                        src = %src_path.display(),
                        dst = %dest.display(),
                        src_bytes = s.len(),
                        dst_bytes = d.len(),
                        sha256 = %att.sha256,
                        "user-attach re-copy found existing worktree file with different size; \
                         possible stale copy or sha256 collision"
                    );
                }
                _ => {}
            }
            copied.push(dest.to_string_lossy().to_string());
            continue;
        }
        match std::fs::copy(&src_path, &dest) {
            Ok(_) => {
                copied.push(dest.to_string_lossy().to_string());
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    feature_id = feature_id,
                    src = %src_path.display(),
                    "failed to copy user attachment into worktree _context/"
                );
            }
        }
    }
    copied
}

fn render_attachment_body(
    step_id: &str,
    paths: &[String],
    mode: AttachmentMode,
    store: &dyn ArtifactStore,
) -> String {
    match mode {
        AttachmentMode::PathManifest => render_path_manifest(step_id, paths),
        AttachmentMode::InlineBody => {
            let mut parts_content = Vec::new();
            for p in paths {
                match store.get(p) {
                    Ok(c) => parts_content.push(c),
                    Err(_) => parts_content.push(format!("(Error reading artifact at {})", p)),
                }
            }
            if parts_content.is_empty() {
                "(No artifacts produced by this step yet)".to_string()
            } else if parts_content.len() == 1 {
                parts_content.into_iter().next().unwrap_or_default()
            } else {
                parts_content.join("\n\n---\n\n")
            }
        }
    }
}

/// Append a synthetic `## Expected Artifacts (orchestrator contract)` block
/// to `prompt` when `declarations` is non-empty. The agent sees exactly
/// which named artifacts the orchestrator expects and where to write
/// them, without the prompt author having to repeat the contract in
/// natural-language prose.
///
/// Returns the original `prompt` unchanged when `declarations` is
/// `None` or empty (legacy backstop).
pub(crate) fn inject_artifact_contract(
    prompt: &str,
    declarations: Option<&[ArtifactDecl]>,
) -> String {
    let decls = match declarations {
        Some(d) if !d.is_empty() => d,
        _ => return prompt.to_string(),
    };

    let mut lines = vec![
        String::new(),
        "## Expected Artifacts (orchestrator contract)".to_string(),
        String::new(),
        "Capture your work in the following files so downstream".to_string(),
        "steps and the reviewer can see what you produced:".to_string(),
        String::new(),
    ];

    for d in decls {
        let hint = match &d.capture {
            ArtifactCapture::ByName { name } => {
                format!("- Produce an artifact named `{}`", name)
            }
            ArtifactCapture::LastWriteTo { path } => {
                format!("- Write `{}` → artifact `{}`", path, d.name)
            }
            ArtifactCapture::AllWrites => {
                "- Every file you write will be captured automatically via git".to_string()
            }
            ArtifactCapture::ChangedFiles { path_filter, .. } => {
                if let Some(filter) = path_filter {
                    format!(
                        "- All files matching `{}` will be captured automatically via git",
                        filter
                    )
                } else {
                    "- All changed files will be captured automatically via git".to_string()
                }
            }
            ArtifactCapture::Diff { .. } => {
                "- A diff will be computed at the end of the step".to_string()
            }
            ArtifactCapture::Worktree { path: Some(p) } => {
                format!("- Worktree pointer for `{}`", p)
            }
            ArtifactCapture::Worktree { path: None } => "- Worktree root pointer".to_string(),
        };
        lines.push(hint);
    }

    lines.push(String::new());
    lines.push(
        "Your file changes are automatically detected via git — no special naming required."
            .to_string(),
    );

    let mut result = prompt.to_string();
    result.push_str(&lines.join("\n"));
    result
}

/// Prepend a prohibitive **Operating Boundary** block describing what the
/// step's capability forbids — the prompt-level counterpart to the OS-level
/// fence and the agent's tool policy. Where [`inject_artifact_contract`]
/// tells the agent *what to produce*, this tells it *what it must not do*,
/// in imperative MUST/MUST NOT language that survives a redirected step
/// trying to "just fix it".
///
/// The block is keyed on the [`StepCapability`] (role) and refined by the
/// resolved [`PermissionProfile`] so the shell/network lines match any
/// per-step `allow_shell` / `allow_network` widening. `Implement` steps get
/// no block (full access — nothing to forbid).
///
/// Returned at the *front* of the prompt: a boundary the model reads first
/// outranks instructions buried in a long template that might tempt it to
/// implement.
pub(crate) fn inject_operating_boundary(
    prompt: &str,
    capability: StepCapability,
    profile: &PermissionProfile,
) -> String {
    let mut lines: Vec<String> = Vec::new();

    let (mode, rules): (&str, Vec<String>) = match capability {
        StepCapability::Implement => return prompt.to_string(),
        StepCapability::ReadOnly => (
            "REVIEW-ONLY",
            vec![
                "You MUST NOT create, edit, move, or delete any file.".to_string(),
                "You MUST NOT modify source code, configuration, or artifacts.".to_string(),
                "Your job is to inspect and report — produce your assessment as \
                 text in your response."
                    .to_string(),
            ],
        ),
        StepCapability::Artifacts => (
            "ANALYSIS",
            vec![
                "You may ONLY write files under the `artifacts/` directory.".to_string(),
                "You MUST NOT modify source code, tests, configuration, or any \
                 file outside `artifacts/`."
                    .to_string(),
                "If the task appears to call for code changes, do NOT make them — \
                 that is a later implementation step's job. Capture your findings, \
                 spec, or plan in your artifact instead."
                    .to_string(),
            ],
        ),
        StepCapability::Verify => (
            "VALIDATION",
            vec![
                "You may run build/test/lint/audit commands and read any file.".to_string(),
                "You may ONLY write files under the `artifacts/` directory (your report)."
                    .to_string(),
                "You MUST NOT fix or modify source code. If you find problems, \
                 document them precisely in your artifact so an implementation \
                 step can address them."
                    .to_string(),
            ],
        ),
    };

    lines.push(format!("## Operating Boundary — {} mode", mode));
    lines.push(String::new());
    lines.push(
        "These constraints are enforced by the orchestrator (the filesystem is \
         fenced and out-of-scope writes are reverted and fail the step). Staying \
         inside them is part of completing the task:"
            .to_string(),
    );
    lines.push(String::new());
    for r in rules {
        lines.push(format!("- {}", r));
    }

    // Shell / network lines reflect the *resolved* profile so per-step
    // widenings (allow_shell / allow_network) don't contradict the block.
    if !profile.execute.is_allow() {
        lines.push("- You MUST NOT run shell commands.".to_string());
    }
    if profile.network.is_allow() {
        lines.push(
            "- You MAY use web search/fetch to consult up-to-date documentation.".to_string(),
        );
    } else {
        lines.push("- You MUST NOT access the network.".to_string());
    }

    lines.push(String::new());
    lines.push("---".to_string());
    lines.push(String::new());

    format!("{}{}", lines.join("\n"), prompt)
}

/// Copy any external artifact paths referenced in a path-manifest prompt
/// into `{wt_path}/artifacts/_context/` so the agent can read them
/// without needing `external_directory: allow`.
///
/// Opencode's `external_directory: deny` restricts all tool access to
/// the worktree `--dir`. Artifact paths in path manifests are absolute
/// paths under the app data directory (e.g. `~/Library/Application
/// Support/…/artifacts/…`) — outside the worktree. This function
/// copies those files into the worktree before the agent runs so the
/// Read tool succeeds.
///
/// Path manifests use the format `- \`/absolute/path\`` (one path per
/// bullet). Any absolute path NOT already under `wt_path` is copied to
/// `{wt_path}/artifacts/_context/<filename>` and the path is rewritten
/// in the returned prompt.
pub(crate) fn materialize_external_artifact_paths(prompt: &str, wt_path: &str) -> String {
    let wt = std::path::Path::new(wt_path);
    let mut result = prompt.to_string();
    let mut rewrites: Vec<(String, String)> = Vec::new();

    // Scan for backtick-quoted absolute paths: `- `/some/path`
    let mut search = prompt;
    while let Some(tick_pos) = search.find("- `") {
        let after_tick = &search[tick_pos + 3..];
        if !after_tick.starts_with('/') {
            search = &search[tick_pos + 1..];
            continue;
        }
        let close = match after_tick.find('`') {
            Some(p) => p,
            None => break,
        };
        let abs_path = &after_tick[..close];
        let path = std::path::Path::new(abs_path);

        if !path.starts_with(wt)
            && !rewrites.iter().any(|(old, _)| old == abs_path)
            && path.is_file()
        {
            if let Some(file_name) = path.file_name() {
                let dest_dir = wt.join("artifacts").join("_context");
                if std::fs::create_dir_all(&dest_dir).is_ok() {
                    let dest = dest_dir.join(file_name);
                    if std::fs::copy(path, &dest).is_ok() {
                        rewrites.push((abs_path.to_string(), dest.to_string_lossy().to_string()));
                    }
                }
            }
        }
        search = &search[tick_pos + 1..];
    }

    for (old, new) in &rewrites {
        result = result.replace(old.as_str(), new.as_str());
    }
    result
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/attached.rs"]
mod tests;
