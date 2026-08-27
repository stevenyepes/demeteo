//! What the resolver is told, and what the words are drawn from.
//!
//! Pure but for two best-effort reads of the worktree — which branch the merge
//! is pulling in, and what that side moved — so every decision here is
//! reachable from a test with no double at all.

use crate::adapters::worktree::git_ops::sync_verify::{run_gate_prepare, GatePrepare};
use crate::paths;
use crate::ports::execution::{ask_within, Answer, ExecutionPort, ShellOptions};
use crate::ports::worktree_ops::MergeGate;

/// What the incoming side of the open merge added, moved or deleted.
///
/// The files git merged *without asking* — the other half of the pair
/// [`build_resolver_prompt`] has to be correct over.
///
/// Best-effort by construction: an unreadable answer leaves the prompt without
/// the section rather than failing a turn over a hint. The gate in
/// [`run_resolver_turn`](super::run_resolver_turn) is what refuses to publish
/// when the aim was off.
///
/// `within` bounds it because this read is issued with the resolver already
/// spawned: unbounded, a transport that goes quiet holds a live agent process
/// open for as long as it stays quiet.
pub(super) async fn base_side_moves(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
    within: std::time::Duration,
) -> Vec<String> {
    match ask_within(
        exec,
        machine_str,
        &format!(
            "git -C {} diff --name-status -M --diff-filter=ADR HEAD...MERGE_HEAD",
            paths::shell_escape_posix(worktree)
        ),
        within,
    )
    .await
    {
        Answer::Said(out) => out
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        Answer::Refused | Answer::Unreadable(_) => Vec::new(),
    }
}

/// Which branch the open merge is pulling in.
///
/// A sync merges `origin/<base>`; a reconcile of a diverged branch merges
/// `origin/<feature>` — the same branch, as origin holds it. Named the wrong
/// one, the resolver reads the incoming commits as upstream's, and the side it
/// defers to is another person's work on the user's own branch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum IncomingSide<'a> {
    Base(&'a str),
    OwnBranch(&'a str),
    /// MERGE_HEAD sits at neither tracking tip, or nothing could be read. The
    /// prompt then names no branch at all rather than the likely one: a name
    /// that is wrong is the failure this type exists to prevent, and it costs
    /// the resolver a hint rather than misdirecting it.
    Unknown,
}

impl IncomingSide<'_> {
    /// How the prompt names this side mid-sentence.
    fn name(&self) -> String {
        match self {
            Self::Base(branch) | Self::OwnBranch(branch) => format!("origin/{}", branch),
            Self::Unknown => "the other side of this merge".to_string(),
        }
    }
}

/// Which branch MERGE_HEAD is the tip of, asked of the worktree rather than of
/// the sync row — the working tree is the authority
/// ([`crate::application::sync_session`]), and the row's `base_branch` is not
/// the incoming side of a reconcile.
///
/// Best-effort on the same terms as [`base_side_moves`], and `within` for the
/// same reason: it is issued with the resolver already spawned.
pub(super) async fn incoming_side<'a>(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    worktree: &str,
    base_branch: &'a str,
    feature_branch: &'a str,
    within: std::time::Duration,
) -> IncomingSide<'a> {
    match ask_within(
        exec,
        machine_str,
        &format!(
            "git -C {} branch --remotes --points-at MERGE_HEAD",
            paths::shell_escape_posix(worktree)
        ),
        within,
    )
    .await
    {
        Answer::Said(out) => tracking_tip_at_merge_head(&out, base_branch, feature_branch),
        Answer::Refused | Answer::Unreadable(_) => IncomingSide::Unknown,
    }
}

/// Read `git branch --remotes --points-at MERGE_HEAD` against the two tips it
/// could be.
///
/// The feature's own tip is tested first: `origin/<feature>` is merged only by
/// a reconcile, and where both tips name one commit the base merge this would
/// otherwise be had nothing to conflict over. Any other answer — a tip that
/// moved under the merge, `origin/HEAD`, an empty list — is
/// [`IncomingSide::Unknown`], because a tracking ref this cannot account for
/// is not evidence for either branch.
pub(super) fn tracking_tip_at_merge_head<'a>(
    points_at: &str,
    base_branch: &'a str,
    feature_branch: &'a str,
) -> IncomingSide<'a> {
    let named = |branch: &str| {
        let want = format!("origin/{}", branch);
        points_at.lines().any(|line| line.trim() == want)
    };
    if named(feature_branch) {
        IncomingSide::OwnBranch(feature_branch)
    } else if named(base_branch) {
        IncomingSide::Base(base_branch)
    } else {
        IncomingSide::Unknown
    }
}

/// How many moves may share one basename with a conflicted file before that
/// basename stops being an aim.
///
/// Four candidates for one conflicted name is a directory reshuffle, not a
/// lead, and promoting all four spends [`RESOLVER_BASE_MOVE_CAP`] on noise.
/// Counting beats a list of generic names (`mod.rs`, `index.ts`,
/// `__init__.py`): in a tree full of `mod.rs`, `mod.rs` demotes itself, with
/// no list to keep up to date.
const AIM_BASENAME_MAX_HITS: usize = 3;

/// The base side's moves, the ones most likely to matter first.
///
/// Git answers in path order, and path order is not relevance order: the file
/// that broke the build in [`build_resolver_prompt`]'s incident was the 69th
/// of 252 entries, so a path-ordered cap would have dropped exactly what the
/// section exists to surface. Naming a conflicted path, and then sharing a
/// filename with one, are the two relationships a path alone can carry, and
/// git's own order survives inside each tier.
pub(super) fn aimed_first<'a>(conflict_files: &[String], base_moves: &'a [String]) -> Vec<&'a str> {
    fn file_name(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
    }
    // Tab-separated (`R100\told\tnew`) rather than whitespace-separated: a
    // path containing a space is one path, and splitting it makes fragments
    // that match nothing.
    fn moved_paths(line: &str) -> impl Iterator<Item = &str> {
        line.split('\t').skip(1).map(str::trim)
    }
    let conflicted: std::collections::HashSet<&str> =
        conflict_files.iter().map(String::as_str).collect();
    let conflicted_names: std::collections::HashSet<&str> =
        conflict_files.iter().map(|f| file_name(f)).collect();

    let mut hits: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for line in base_moves {
        let named: std::collections::HashSet<&str> = moved_paths(line).map(file_name).collect();
        for name in named {
            *hits.entry(name).or_default() += 1;
        }
    }

    let tier = |line: &str| {
        if moved_paths(line).any(|p| conflicted.contains(p)) {
            0
        } else if moved_paths(line).any(|p| {
            let name = file_name(p);
            conflicted_names.contains(name)
                && hits.get(name).is_some_and(|n| *n <= AIM_BASENAME_MAX_HITS)
        }) {
            1
        } else {
            2
        }
    };
    let mut ranked: Vec<(u8, &str)> = base_moves
        .iter()
        .map(|line| (tier(line), line.as_str()))
        .collect();
    ranked.sort_by_key(|(tier, _)| *tier);
    ranked.into_iter().map(|(_, line)| line).collect()
}

/// How many base-side moves the prompt is willing to spend context on before it
/// hands the resolver the command instead. A hint that drowns the conflict it
/// was meant to aim at is worse than no hint, and the tail is reachable in one
/// call by an agent that has a reason to want it.
const RESOLVER_BASE_MOVE_CAP: usize = 40;

/// What the prompt may say about the project's own checks — and, because
/// [`Gated`](Self::Gated) is the only variant carrying a command as far as
/// [`run_gate_harness`](crate::adapters::worktree::git_ops::sync_verify::run_gate_harness),
/// which trees those checks are ever run against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Verification<'a> {
    Ungated,
    /// `command` runs against whatever the turn leaves, and a red answer
    /// refuses the resolution.
    Gated {
        command: &'a str,
    },
    /// `prepare` failed here, so `command` would answer about the worktree
    /// rather than the merge and nothing will run it.
    Unprepared {
        prepare: &'a str,
        command: &'a str,
    },
}

/// What this worktree earned, from what `prepare` did in it — not from what
/// the project declared.
fn verification_for<'a>(prepared: &GatePrepare, gate: MergeGate<'a>) -> Verification<'a> {
    match (prepared, gate.harness, gate.prepare) {
        (_, None, _) => Verification::Ungated,
        (GatePrepare::Failed(_), Some(command), Some(prepare)) => {
            Verification::Unprepared { prepare, command }
        }
        (_, Some(command), _) => Verification::Gated { command },
    }
}

/// Bring the worktree to a state the harness can answer about, then decide
/// what the prompt may claim. `None` is a stop that arrived during prepare.
///
/// The two belong together: told to run the harness in a tree nothing had
/// installed into, and told in the same breath not to go looking for another
/// command, an agent stops on a red that is about the tree it was handed.
pub(super) async fn prepared_verification<'a>(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    gate: MergeGate<'a>,
    opts: ShellOptions,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> Option<Verification<'a>> {
    let prepared = run_gate_prepare(exec, machine_str, gate, opts, cancel).await;
    if matches!(prepared, GatePrepare::Stopped) {
        return None;
    }
    Some(verification_for(&prepared, gate))
}

/// Build the prompt for the conflict-resolution agent.
///
/// Git's conflicted list is what it could not reconcile *textually*, and not
/// the set a resolution has to be correct over: a file only one side touched
/// merges silently and can still call a signature the other side changed. A
/// resolver told to touch nothing else did exactly as asked, and the merge
/// commit it produced turned every check on the pull request red. So the scope
/// clause is bounded by the merge's own damage — the bound the `git add -A` in
/// [`run_resolver_turn`](super::run_resolver_turn) already stages to, with
/// [`base_side_moves`] naming
/// the silent half outright — and it stays a bound rather than an opening
/// because an agent invited to fix whatever it finds returns a refactor nobody
/// merged.
///
/// The verification line is the one that has to be exact. "Run the project's
/// build / test suite" reads as a complete instruction and is not one: the
/// agent has to *find* the command first, and a search is a turn each against
/// a cap of [`RESOLVER_MAX_TURNS`](super::RESOLVER_MAX_TURNS). Naming the
/// project's own command turns
/// that search into a single call, and a project with no command configured
/// gets no verification line at all rather than a vague one — an unanswerable
/// instruction is more expensive than a missing one.
pub(super) fn build_resolver_prompt(
    feature_branch: &str,
    incoming: IncomingSide<'_>,
    conflict_files: &[String],
    verification: Verification<'_>,
    base_moves: &[String],
) -> String {
    let files_list = conflict_files
        .iter()
        .map(|f| format!("- {}", f))
        .collect::<Vec<_>>()
        .join("\n");
    let merged_silently = if base_moves.is_empty() {
        String::new()
    } else {
        let mut lines = aimed_first(conflict_files, base_moves)
            .into_iter()
            .take(RESOLVER_BASE_MOVE_CAP)
            .map(|m| format!("- {}", m))
            .collect::<Vec<_>>();
        let rest = base_moves.len().saturating_sub(RESOLVER_BASE_MOVE_CAP);
        if rest > 0 {
            lines.push(format!(
                "- …and {} more. Run `git diff --name-status -M --diff-filter=ADR \
                 HEAD...MERGE_HEAD` for the full list.",
                rest
            ));
        }
        let opener = match incoming {
            IncomingSide::Base(_) => format!(
                "{} also added, moved or deleted these files since this branch left it.",
                incoming.name()
            ),
            IncomingSide::OwnBranch(_) => format!(
                "{} added, moved or deleted these files too.",
                incoming.name()
            ),
            IncomingSide::Unknown => {
                "The other side of this merge also added, moved or deleted these files.".to_string()
            }
        };
        format!(
            "{opener} Git merged them without asking, so they carry no markers — and \
             they are where a resolution that only fixes the listed files goes wrong:\n\
             {moves}\n\
             Files {side} only *modified* are not in that list and merged just as \
             silently — `git diff --name-status -M HEAD...MERGE_HEAD` has those.\n\n",
            opener = opener,
            side = incoming.name(),
            moves = lines.join("\n"),
        )
    };
    let checks = match verification {
        Verification::Ungated => String::new(),
        Verification::Gated { command } if command.trim().is_empty() => String::new(),
        Verification::Gated { command } => format!(
            "- When done, verify with this project's own command, exactly as written: `{cmd}`.\n\
             - Do NOT go looking for another command if that one does not work here — \
             say so in your summary and stop.\n\
             - A tree that does not build is not a resolved conflict: Demeteo runs `{cmd}` \
             against your resolution and will not commit it if that comes back red, so fix \
             what it reports here rather than leaving it.\n",
            cmd = command.trim()
        ),
        Verification::Unprepared { prepare, command } => format!(
            "- This worktree could not be prepared: `{prepare}` failed here, so `{cmd}` \
             cannot give a meaningful answer and Demeteo will not run it either. Resolve \
             the conflict by reading the code, and do not chase errors that command \
             reports.\n",
            prepare = prepare.trim(),
            cmd = command.trim()
        ),
    };
    let (opening, own_branch_note) = match incoming {
        IncomingSide::Base(_) => (incoming.name(), String::new()),
        IncomingSide::OwnBranch(branch) => (
            incoming.name(),
            format!(
                "origin/{branch} is this same branch as origin holds it: those commits are \
                 someone else's work on it, not a change from upstream.\n",
                branch = branch
            ),
        ),
        IncomingSide::Unknown => ("another branch".to_string(), String::new()),
    };
    format!(
        "We just merged {opening} into {feature}. A merge conflict was detected.\n\
         {note}\
         Please resolve the conflicts in the following files:\n\
         {files}\n\n\
         {merged_silently}\
         For each file:\n\
         - Read the conflict markers (<<<<<<<, =======, >>>>>>>).\n\
         - Integrate the changes from both sides correctly.\n\
         - Remove all conflict markers.\n\
         - Fix a file outside this list only where the merge itself broke it — a \
         caller of a signature one side changed, a test the other side moved. Do \
         not refactor, reformat, or fix anything the merge did not break.\n\
         {checks}\
         - Do NOT stage or commit — Demeteo validates, stages, and commits the resolution.\n\
         - Report back with a one-line summary when you're done.",
        opening = opening,
        note = own_branch_note,
        feature = feature_branch,
        files = files_list,
        merged_silently = merged_silently,
        checks = checks,
    )
}

/// Ask the resolver to fix the tree it just left, in the harness's own words.
///
/// Not framed as a conflict, because by the time this is asked there is none:
/// the markers are gone, and an opening that tells an agent to read them is an
/// instruction over a file that no longer contains anything to read. The
/// output is already bounded by
/// [`gate_output_excerpt`](crate::domain::sync_session::gate_output_excerpt);
/// nothing here truncates a second time.
pub(super) fn build_repair_prompt(feature_branch: &str, command: &str, excerpt: &str) -> String {
    format!(
        "Your conflict resolution in {feature} does not build.\n\n\
         Demeteo ran the project's own command against the tree you just left, and it \
         came back red:\n\n\
         $ {cmd}\n\
         {excerpt}\n\n\
         The conflict markers are gone and the merge is still open — nothing has been \
         staged or committed. Fix what that output reports, in this same worktree.\n\n\
         - Fix only what the merge broke. The base branch moved under this one, so the \
         usual cause is a caller, an import, a test or a signature the two sides changed \
         apart. Do not refactor, reformat, or fix anything unrelated.\n\
         - Re-run `{cmd}` yourself when you think it is fixed.\n\
         - Do NOT stage or commit — Demeteo validates, stages, and commits the resolution.\n\
         - If you cannot make it pass, say so in one line and stop. Demeteo runs `{cmd}` \
         again either way, and will not commit a tree that is still red.",
        feature = feature_branch,
        cmd = command.trim(),
        excerpt = excerpt,
    )
}
