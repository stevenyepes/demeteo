import { useNavigation } from '../../context';
import { forceStartTicket, startTicket } from '../../lib/discovery';
import { useErrorBus } from '../../lib/errorBus';
import { formatError } from '../../lib/errors';
import { reportDegradedLaunch } from '../../lib/launch';
import { invalidateRunnerCompatibility, isRunnerIncompatibleError } from '../../lib/runnerCompatibility';
import { draftPlacement } from '../../lib/ticketPlacement';
import type { LaunchedRun, TicketView } from '../../types';

interface TicketLaunchDeps {
  /** The view's own action runner, which owns `busy`. */
  runAction: (action: () => Promise<unknown>, reportFailure: (cause: unknown) => void) => Promise<void>;
  setActionError: (message: string) => void;
}

/**
 * Start or force-start `view`'s ticket — on `machineId` for this launch only,
 * when one is given (`ticket_start`'s override; blank is none). `busy`
 * disables both controls until the call settles, and a detached start awaits
 * the runner's answer, so `runAction` is also what stops a slow start from
 * being submitted twice.
 *
 * A runner refusal goes to the toast rather than the banner because its
 * remedy, the machine's runner, lives in settings — the same refusal
 * `useLaunchRun` reports for a Feature. A run the runner accepted but left
 * degraded is a started ticket, so it gets that hook's notice, not the banner.
 */
export function useTicketLaunch({ runAction, setActionError }: TicketLaunchDeps): {
  start: (view: TicketView, machineId?: string) => void;
  forceStart: (view: TicketView, reason: string, machineId?: string) => void;
} {
  const { reportError } = useErrorBus();
  const { navigate } = useNavigation();

  function launch(
    view: TicketView,
    machineId: string | undefined,
    start: (ticketId: string) => Promise<LaunchedRun>,
  ) {
    void runAction(
      async () =>
        reportDegradedLaunch(await start(view.ticket.id), reportError, () =>
          navigate({ kind: 'remote-inbox' }),
        ),
      (cause) => {
        if (!isRunnerIncompatibleError(cause)) {
          setActionError(formatError(cause));
          return;
        }
        // The runner that refused is the one this launch went to, which an
        // override moves off the ticket's own placement.
        const placement = draftPlacement(machineId?.trim() ?? '', view.placement.placement);
        if (placement.kind === 'detached') invalidateRunnerCompatibility(placement.machine_id);
        reportError(cause, {
          action: { label: 'Open machine settings', onClick: () => navigate({ kind: 'settings' }) },
        });
      },
    );
  }

  return {
    start: (view, machineId) => launch(view, machineId, (id) => startTicket(id, machineId)),
    forceStart: (view, reason, machineId) =>
      launch(view, machineId, (id) => forceStartTicket(id, reason, machineId)),
  };
}
