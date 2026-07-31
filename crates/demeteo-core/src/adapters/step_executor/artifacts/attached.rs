use crate::domain::attachment::AttachedFile;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::attachment_store::AttachmentStore;

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
                let ext = crate::domain::attachment::resolved_ext(att);
                let stored = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
                let display_path = crate::domain::attachment::worktree_display_path(
                    att,
                    &ext,
                    worktree_artifacts_dir,
                    &stored,
                );
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
            let ext = crate::domain::attachment::resolved_ext(att);
            let stored = attachment_store.lookup_path(feature_id, &att.sha256, &ext);
            let display_path = crate::domain::attachment::worktree_display_path(
                att,
                &ext,
                worktree_artifacts_dir,
                &stored,
            );
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

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/attached.rs"]
mod tests;
