import { buttonClasses } from '../../../src/components/ui/Button';
import { Chip } from '../../../src/components/ui/Chip';
import { TONE_BORDER_L } from '../../../src/lib/runStatus';
import { routeHref } from '../lib/router';

/**
 * Keyed by the codes `POST /git-token` answers (`TokenRefusal` in
 * `crates/demeteo-hub/src/git_token/verdict.rs`). Each is retried by the
 * runner, so fixing the cause lets the run carry on without a restart.
 */
const PROBLEMS: Record<string, string> = {
  github_app_missing:
    'No GitHub App is connected for this repository’s host, so the runner cannot push. Connect one on Git hosts.',
  github_app_not_installed:
    'The connected GitHub App is not installed on this repository. Add the repository to the App on GitHub.',
  github_app_ambiguous:
    'More than one connected GitHub App is installed on this repository, and the Hub will not guess which to use. Remove the repository from all but one.',
  git_token_unavailable:
    'The Hub could not mint a git token: its key service or GitHub did not answer. The runner keeps retrying.',
};

interface RunGitTokenProblemProps {
  problem: string;
}

/**
 * The desktop's "Re-inject credentials" is the wrong fix for a run the Hub
 * started: its token comes from a GitHub App, so the way out is on Git hosts
 * or on GitHub, which is where this card sends the user.
 */
export function RunGitTokenProblem({ problem }: RunGitTokenProblemProps) {
  return (
    <section
      aria-label="Git token"
      className={`panel flex flex-col gap-3 border-l-2 p-5 text-sm ${TONE_BORDER_L.amber}`}
    >
      <Chip tone="amber" dot={false} size="md" className="self-start">
        Needs you
      </Chip>
      <p className="text-slate-200">
        {PROBLEMS[problem] ?? 'The runner could not get a git token for this run, so it cannot push yet.'}
      </p>
      <a href={routeHref({ name: 'git-hosts' })} className={`${buttonClasses('secondary', 'sm')} self-start`}>
        Open Git hosts
      </a>
    </section>
  );
}
