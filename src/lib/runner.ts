import { invoke } from "@tauri-apps/api/core";

/**
 * What this laptop can push right now, with no network and no SSH — the
 * first step of provisioning (docs/REMOTE_EXECUTION.md M7.1). Mirrors the
 * Rust `LocalRunnerCheck`, whose `status` is the serde tag.
 */
export type LocalRunnerCheck =
  | {
      status: 'ready';
      path: string;
      version: string | null;
      expected: string;
      stale_warning: string | null;
    }
  | { status: 'missing'; expected: string };

/** State of `demeteo-runner` on a machine. Mirrors the Rust
 *  `RunnerInstallStatus`; every probe is independently nullable because a
 *  failed probe is not the same answer as a negative one. */
export interface RunnerInstallStatus {
  installed: boolean;
  version: string | null;
  service_active: boolean | null;
  lingering: boolean | null;
}

/** Result of pushing + installing the binary. Mirrors the Rust
 *  `EnableRemoteRunsOutcome`. */
export interface EnableRemoteRunsOutcome {
  version: string | null;
  linger_enabled: boolean;
  warning: string | null;
}

/** Where the release the laptop just fetched landed. Mirrors the Rust
 *  `DownloadedRunner`. */
export interface DownloadedRunner {
  path: string;
  version: string;
}

export async function checkLocalRunner(): Promise<LocalRunnerCheck> {
  return invoke<LocalRunnerCheck>('remote_runner_local_check');
}

export async function getRunnerStatus(machineId: string): Promise<RunnerInstallStatus> {
  return invoke<RunnerInstallStatus>('remote_runner_status', { machineId });
}

/**
 * Fetch the release asset **onto the laptop**, never onto the machine: a
 * remote box is not assumed to have internet access, so the laptop is always
 * the one that downloads. Progress arrives on the `runner-download-progress`
 * Tauri event.
 */
export async function downloadRunner(): Promise<DownloadedRunner> {
  return invoke<DownloadedRunner>('remote_runner_download');
}

/** Cancel whatever {@link downloadRunner} call is in flight; a no-op when
 *  nothing is downloading. The download itself rejects with its own
 *  "cancelled" error. */
export async function cancelRunnerDownload(): Promise<void> {
  return invoke<void>('remote_runner_download_cancel');
}

/** SFTP `localBinPath` to the machine and install it as a systemd `--user`
 *  service. Idempotent — the same sequence installs and upgrades. */
export async function enableRemoteRuns(
  machineId: string,
  localBinPath: string,
): Promise<EnableRemoteRunsOutcome> {
  return invoke<EnableRemoteRunsOutcome>('remote_enable_runs', { machineId, localBinPath });
}
