use crate::application::attachments::StagedAttachmentInput;
use crate::domain::ids::FeatureId;
use crate::domain::run_spec::RunSpecAttachment;
use crate::ports::db::FeaturePatch;
use crate::state::AppContext;

const MAX_DETACHED_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;

fn attachment_spool_dir(home: &str, run_id: &str) -> String {
    format!("{home}/.local/share/demeteo-runner/attachment-spool/{run_id}")
}

pub(super) async fn cleanup_attachment_spool(ctx: &AppContext, machine_id: &str, run_id: &str) {
    let Ok(home) = ctx.exec.resolve_home(machine_id).await else {
        return;
    };
    let spool_dir = attachment_spool_dir(&home, run_id);
    let _ = ctx
        .exec
        .run_command(
            machine_id,
            &format!("rm -rf {}", crate::paths::shell_escape_posix(&spool_dir)),
        )
        .await;
}

pub(super) async fn spool_attachments(
    ctx: &AppContext,
    machine_id: &str,
    run_id: &str,
    staged: Vec<StagedAttachmentInput>,
) -> Result<Vec<RunSpecAttachment>, String> {
    if staged.is_empty() {
        return Ok(Vec::new());
    }
    let home = ctx.exec.resolve_home(machine_id).await?;
    let spool_dir = attachment_spool_dir(&home, run_id);
    ctx.exec
        .run_command(
            machine_id,
            &format!("mkdir -p {}", crate::paths::shell_escape_posix(&spool_dir)),
        )
        .await?;

    let mut out = Vec::new();
    for (i, attachment) in staged.into_iter().enumerate() {
        let display_name = attachment
            .source_filename
            .clone()
            .unwrap_or_else(|| attachment.source_path.clone());
        let too_big = |actual_bytes: u64| {
            format!(
                "attachment {display_name} is {} MB — detached runs cap attachments at {} MB",
                actual_bytes / (1024 * 1024),
                MAX_DETACHED_ATTACHMENT_BYTES / (1024 * 1024),
            )
        };
        let bytes = match attachment.bytes {
            Some(bytes) => bytes,
            None => {
                let meta = tokio::fs::metadata(&attachment.source_path)
                    .await
                    .map_err(|e| {
                        format!("failed to read attachment {}: {e}", attachment.source_path)
                    })?;
                if meta.len() > MAX_DETACHED_ATTACHMENT_BYTES as u64 {
                    return Err(too_big(meta.len()));
                }
                tokio::fs::read(&attachment.source_path)
                    .await
                    .map_err(|e| {
                        format!("failed to read attachment {}: {e}", attachment.source_path)
                    })?
            }
        };
        if bytes.len() > MAX_DETACHED_ATTACHMENT_BYTES {
            return Err(too_big(bytes.len() as u64));
        }
        let name = attachment
            .source_filename
            .clone()
            .or_else(|| {
                std::path::Path::new(&attachment.source_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("attachment-{i}"));
        let safe_name = std::path::Path::new(&name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("attachment")
            .to_string();
        let staged_path = format!("{spool_dir}/{i}-{safe_name}");
        ctx.exec
            .write_file_bytes(machine_id, &staged_path, &bytes)
            .await
            .map_err(|e| format!("failed to spool attachment {safe_name} to the runner: {e}"))?;
        out.push(RunSpecAttachment {
            staged_path,
            mime: attachment.mime,
            source_filename: attachment.source_filename,
        });
    }
    Ok(out)
}

pub(super) fn mark_placeholder_failed(ctx: &AppContext, feature_id: &str) {
    let _ = ctx.features.update(
        &FeatureId::from(feature_id.to_string()),
        &FeaturePatch {
            status: Some("failed".to_string()),
            ..Default::default()
        },
    );
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/attachments.rs"]
mod tests;
