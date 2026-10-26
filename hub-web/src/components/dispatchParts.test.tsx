import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, type Mock, vi } from 'vitest';

import type { FleetInstance } from '../lib/fleet';
import { AttachmentPicker } from './AttachmentPicker';
import { DispatchCapsFields } from './DispatchCapsFields';
import { DispatchTargetList } from './DispatchTargetList';
import { RequestStatusCard } from './RequestStatusCard';
import { StepOverridePanel } from './StepOverridePanel';

function renderCaps(props: Partial<Parameters<typeof DispatchCapsFields>[0]> = {}) {
  const handlers = { onMaxCostUsdChange: vi.fn(), onMaxWallClockMinsChange: vi.fn() };
  render(<DispatchCapsFields maxCostUsd="" maxWallClockMins="" {...handlers} {...props} />);
  return handlers;
}

describe('DispatchCapsFields', () => {
  it('marks max cost required and says why the instance refuses without one', () => {
    renderCaps();
    const cost = screen.getByLabelText('Max cost (USD)');

    expect(cost).toHaveAttribute('aria-required', 'true');
    expect(cost).toHaveAccessibleDescription(/refuses to start a run with no cap/);
    expect(cost).toHaveAccessibleDescription(/above its own local ceiling/);
    expect(screen.getByLabelText('Max wall-clock (min)')).not.toHaveAttribute('aria-required');
  });

  it('shows a field error passed in', () => {
    renderCaps({ costError: 'A max cost is required.', wallClockError: 'Enter whole minutes.' });

    expect(screen.getByText('A max cost is required.')).toBeVisible();
    expect(screen.getByText('Enter whole minutes.')).toBeVisible();
    expect(screen.getByLabelText('Max cost (USD)')).toHaveAttribute('aria-invalid', 'true');
    expect(screen.getByLabelText('Max wall-clock (min)')).toHaveAccessibleDescription(/Enter whole minutes\./);
  });

  it('reports typing to its handlers', async () => {
    const handlers = renderCaps();

    await userEvent.type(screen.getByLabelText('Max cost (USD)'), '5');
    await userEvent.type(screen.getByLabelText('Max wall-clock (min)'), '9');

    expect(handlers.onMaxCostUsdChange).toHaveBeenCalledWith('5');
    expect(handlers.onMaxWallClockMinsChange).toHaveBeenCalledWith('9');
  });
});

const agentNode = { id: 'implement', type: 'agent', title: 'Implement' };

function renderPanel(props: Partial<Parameters<typeof StepOverridePanel>[0]> = {}) {
  const handlers = { onChange: vi.fn(), onReset: vi.fn() };
  render(<StepOverridePanel node={agentNode} override={{}} defaultAgentKind="claude-code" {...handlers} {...props} />);
  return handlers;
}

describe('StepOverridePanel', () => {
  it('reports the agent, model and effort set on a step', async () => {
    const handlers = renderPanel();

    await userEvent.selectOptions(screen.getByLabelText('Agent'), 'codex');
    expect(handlers.onChange).toHaveBeenLastCalledWith({ agentKind: 'codex' });

    await userEvent.type(screen.getByLabelText('Model'), 'x');
    expect(handlers.onChange).toHaveBeenLastCalledWith({ model: 'x' });

    await userEvent.selectOptions(screen.getByLabelText('Effort'), 'low');
    expect(handlers.onChange).toHaveBeenLastCalledWith({ effort: 'low' });
  });

  it('clears the model and clamps the effort when the agent changes', async () => {
    const handlers = renderPanel({ override: { agentKind: 'claude-code', model: 'opus', effort: 'max' } });

    await userEvent.selectOptions(screen.getByLabelText('Agent'), 'codex');

    expect(handlers.onChange).toHaveBeenLastCalledWith({ agentKind: 'codex', effort: 'xhigh' });
  });

  it('inherits every field on Inherit all', async () => {
    const handlers = renderPanel({ override: { agentKind: 'codex', model: 'gpt', effort: 'low' } });

    await userEvent.click(screen.getByRole('button', { name: 'Inherit all' }));

    expect(handlers.onReset).toHaveBeenCalledOnce();
    expect(handlers.onChange).not.toHaveBeenCalled();
  });

  it.each(['gate', 'finalize'])('offers a %s node no override controls', (type) => {
    renderPanel({ node: { id: 'n', type, title: 'Fixed' } });

    expect(screen.queryByLabelText('Agent')).toBeNull();
    expect(screen.queryByLabelText('Model')).toBeNull();
    expect(screen.queryByLabelText('Effort')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Inherit all' })).toBeNull();
    expect(screen.getByText(/runs no agent/)).toBeVisible();
  });

  it('says effort is unsupported when the step runs on hermes, inherited or pinned', () => {
    const { unmount } = render(
      <StepOverridePanel node={agentNode} override={{}} defaultAgentKind="hermes" onChange={vi.fn()} onReset={vi.fn()} />,
    );
    expect(screen.getByText(/hermes has no per-run effort setting/)).toBeVisible();
    expect(screen.getByLabelText('Effort')).toBeDisabled();
    unmount();

    renderPanel({ override: { agentKind: 'hermes' } });
    expect(screen.getByText(/hermes has no per-run effort setting/)).toBeVisible();
  });

  it('does not show the hermes note for an agent with effort levels', () => {
    renderPanel();

    expect(screen.queryByText(/no per-run effort setting/)).toBeNull();
    expect(screen.getByLabelText('Effort')).toBeEnabled();
  });

  it('says the instance refuses a harness it has not registered', () => {
    renderPanel();

    expect(screen.getByText(/harness_not_registered/)).toBeVisible();
  });
});

const fileNamed = (name: string, bytes = 3) => new File(['x'.repeat(bytes)], name, { type: 'text/plain' });

/** By name: `toEqual` finds any two `File`s alike, since a `File` has no own enumerable fields. */
function namesLastPassed(onChange: Mock<(files: File[]) => void>): string[] | undefined {
  return onChange.mock.lastCall?.[0].map((file) => file.name);
}

describe('AttachmentPicker', () => {
  it('reports two picked files to onChange', async () => {
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={[]} onChange={onChange} />);
    const picked = [fileNamed('a.txt'), fileNamed('b.txt')];

    await userEvent.upload(screen.getByLabelText('Add files'), picked);

    expect(namesLastPassed(onChange)).toEqual(['a.txt', 'b.txt']);
  });

  it('adds dropped and pasted files after the ones already held', () => {
    const held = fileNamed('held.txt');
    const dropped = fileNamed('dropped.txt');
    const pasted = fileNamed('pasted.png');
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={[held]} onChange={onChange} />);
    const zone = screen.getByRole('group', { name: 'Attachments' });

    fireEvent.drop(zone, { dataTransfer: { files: [dropped] } });
    expect(namesLastPassed(onChange)).toEqual(['held.txt', 'dropped.txt']);

    fireEvent.paste(zone, { clipboardData: { files: [pasted] } });
    expect(namesLastPassed(onChange)).toEqual(['held.txt', 'pasted.png']);
  });

  it('shows each file as a chip and removes the one asked', async () => {
    const a = fileNamed('a.txt', 2048);
    const b = fileNamed('b.txt');
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={[a, b]} onChange={onChange} />);

    expect(screen.getByText('a.txt')).toBeVisible();
    expect(screen.getByText('2.0 KiB')).toBeVisible();
    await userEvent.click(screen.getByRole('button', { name: 'Remove a.txt' }));

    expect(namesLastPassed(onChange)).toEqual(['b.txt']);
  });

  it('refuses an 11th file with a visible message', async () => {
    const held = Array.from({ length: 10 }, (_, i) => fileNamed(`f${i}.txt`));
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={held} onChange={onChange} />);

    await userEvent.upload(screen.getByLabelText('Add files'), fileNamed('eleventh.txt'));

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.getByRole('alert')).toHaveTextContent(/at most 10 files/);
    expect(screen.getByRole('alert')).toHaveTextContent(/eleventh\.txt/);
  });

  it('keeps the files that fit under the cap and refuses the rest', async () => {
    const held = Array.from({ length: 9 }, (_, i) => fileNamed(`f${i}.txt`));
    const fits = fileNamed('fits.txt');
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={held} onChange={onChange} />);

    await userEvent.upload(screen.getByLabelText('Add files'), [fits, fileNamed('over.txt')]);

    expect(namesLastPassed(onChange)).toEqual([...held.map((file) => file.name), 'fits.txt']);
    expect(screen.getByRole('alert')).toHaveTextContent(/over\.txt/);
  });

  it('says files are uploaded encrypted, fetched once, then deleted', () => {
    render(<AttachmentPicker files={[]} onChange={vi.fn()} />);

    expect(screen.getByText(/uploaded encrypted, fetched once by the instance, then deleted/)).toBeVisible();
  });

  it('takes no files and removes none while disabled', () => {
    const held = fileNamed('held.txt');
    const onChange = vi.fn<(files: File[]) => void>();
    render(<AttachmentPicker files={[held]} onChange={onChange} disabled />);

    expect(screen.getByLabelText('Add files')).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Remove held.txt' })).toBeDisabled();
    fireEvent.drop(screen.getByRole('group', { name: 'Attachments' }), { dataTransfer: { files: [fileNamed('x')] } });
    expect(onChange).not.toHaveBeenCalled();
  });
});

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const REQUEST_ID = 'q8Hn3_kZ-1';
const STATUS_REQUEST = `GET /v1/instances/${INSTANCE_ID}/requests/${REQUEST_ID}`;

describe('RequestStatusCard', () => {
  /** Every request the double saw; the status path answers from `answers` in order, anything else throws. */
  let asked: string[];
  let answers: Record<string, unknown>[];
  let expiresAt: number;

  beforeEach(() => {
    vi.useFakeTimers();
    asked = [];
    answers = [];
    expiresAt = Math.floor(Date.now() / 1000) + 600;
    vi.stubGlobal(
      'fetch',
      vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
        const request = `${init?.method ?? 'GET'} ${String(input)}`;
        asked.push(request);
        const answer = answers.shift();
        if (request !== STATUS_REQUEST || answer === undefined) {
          throw new Error(`fetch double was not told about ${request}`);
        }
        const { httpStatus = 200, ...body } = answer;
        return new Response(JSON.stringify(body), {
          status: Number(httpStatus),
          headers: { 'Content-Type': 'application/json' },
        });
      }),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  const status = (fields: Record<string, unknown>) =>
    answers.push({
      request_id: REQUEST_ID,
      kind: 'start_feature',
      deliveries: 1,
      expires_at: expiresAt,
      answered_at: null,
      ...fields,
    });

  const renderCard = (present = true) =>
    render(<RequestStatusCard instanceId={INSTANCE_ID} enqueued={{ requestId: REQUEST_ID, expiresAt }} present={present} />);

  const elapse = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

  it('shows a refused over_run_ceiling request in ruby, with the per-run-ceiling sentence and the raw code', async () => {
    status({ status: 'refused', reason: 'over_run_ceiling' });
    renderCard();

    await elapse(2_000);

    const card = screen.getByRole('status');
    expect(card).toHaveClass('text-ruby-300', 'border-ruby-500/30', 'bg-ruby-500/10');
    expect(card).toHaveTextContent(/Refused/);
    expect(screen.getByText("The max cost is above this instance's per-run ceiling.")).toBeVisible();
    expect(screen.getByText('over_run_ceiling')).toHaveClass('font-mono');
  });

  it('says spend is off for a scope_off refusal', async () => {
    status({ status: 'refused', reason: 'scope_off' });
    renderCard();

    await elapse(2_000);

    expect(screen.getByText(/Spend is off on this instance/)).toBeVisible();
    expect(screen.getByText('scope_off')).toBeVisible();
  });

  it('shows the raw code alone for a reason it has no sentence for', async () => {
    status({ status: 'refused', reason: 'brand_new_reason' });
    renderCard();

    await elapse(2_000);

    expect(screen.getByRole('status')).toHaveTextContent(/^Refused by the instancebrand_new_reason$/);
  });

  it('says an offline request is queued until the instance reconnects, with the answered expiry', async () => {
    status({ status: 'queued', deliveries: 0 });
    renderCard(false);

    await elapse(2_000);

    const card = screen.getByRole('status');
    expect(card).toHaveTextContent(/Queued until the instance reconnects/);
    expect(card).toHaveTextContent(new Date(expiresAt * 1000).toLocaleTimeString());
    expect(card).not.toHaveTextContent(/24 h/);
    expect(asked).toEqual([STATUS_REQUEST]);
  });

  it('backs off while waiting and stops polling after a terminal status', async () => {
    status({ status: 'queued' });
    status({ status: 'delivered' });
    status({ status: 'accepted', answered_at: expiresAt - 590 });
    renderCard();

    await elapse(2_000);
    await elapse(3_999);
    expect(asked).toHaveLength(1);
    await elapse(1);
    expect(screen.getByRole('status')).toHaveTextContent(/Delivered/);
    await elapse(8_000);
    expect(screen.getByRole('status')).toHaveClass('text-emerald-300');

    await elapse(60_000);
    expect(asked).toHaveLength(3);
    expect(vi.getTimerCount()).toBe(0);
  });

  it('stops polling once past expiry', async () => {
    expiresAt = Math.floor(Date.now() / 1000) + 3;
    status({ status: 'queued' });
    status({ status: 'queued' });
    renderCard();

    await elapse(60_000);

    expect(asked).toHaveLength(2);
    expect(vi.getTimerCount()).toBe(0);
    expect(screen.getByRole('status')).toHaveTextContent(/past its expiry/);
  });

  it('shows a poll error by its code and stops polling', async () => {
    status({ httpStatus: 404, error: 'not_found' });
    renderCard();

    await elapse(60_000);

    expect(screen.getByRole('alert')).toHaveTextContent('not_found');
    expect(asked).toHaveLength(1);
  });

  it('clears its timer on unmount', async () => {
    status({ status: 'queued' });
    const { unmount } = renderCard();
    expect(vi.getTimerCount()).toBe(1);

    unmount();

    expect(vi.getTimerCount()).toBe(0);
    await elapse(60_000);
    expect(asked).toEqual([]);
  });
});

function fleetInstance(overrides: Partial<FleetInstance> = {}): FleetInstance {
  return {
    id: 'inst-1',
    label: 'laptop',
    present: true,
    os: 'linux',
    arch: 'x86_64',
    version: '1.0.0',
    scopes: ['read', 'spend'],
    readGranted: true,
    lastSeen: 1_700_000_000,
    liveRuns: 0,
    needsYou: [],
    ...overrides,
  };
}

describe('DispatchTargetList', () => {
  it('disables a row without spend, says why, and does not select it', async () => {
    const onSelect = vi.fn();
    render(
      <DispatchTargetList
        instances={[fleetInstance({ id: 'inst-2', label: 'desk', scopes: ['read'] })]}
        selectedId={null}
        onSelect={onSelect}
      />,
    );
    const row = screen.getByRole('button', { name: 'desk' });

    expect(row).toBeDisabled();
    expect(screen.getByText('Spend has not been granted on this instance.')).toBeVisible();
    expect(row).toHaveAccessibleDescription(/Spend has not been granted on this instance\./);
    await userEvent.click(row);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('lets an offline row with spend be selected and says requests queue', async () => {
    const onSelect = vi.fn();
    render(
      <DispatchTargetList
        instances={[fleetInstance({ id: 'inst-3', label: null, present: false })]}
        selectedId={null}
        onSelect={onSelect}
      />,
    );
    const row = screen.getByRole('button', { name: 'inst-3' });

    expect(row).toBeEnabled();
    expect(row).toHaveAccessibleDescription(/offline · requests queue until it reconnects/);
    expect(screen.getByText('offline · requests queue until it reconnects')).toBeVisible();
    await userEvent.click(row);
    expect(onSelect).toHaveBeenCalledWith('inst-3');
  });

  it('shows presence and scopes, and marks the selected row', () => {
    render(
      <DispatchTargetList
        instances={[fleetInstance(), fleetInstance({ id: 'inst-4', label: 'other' })]}
        selectedId="inst-1"
        onSelect={vi.fn()}
      />,
    );
    const row = screen.getByRole('button', { name: 'laptop' });

    expect(row).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByRole('button', { name: 'other' })).toHaveAttribute('aria-pressed', 'false');
    expect(row).toHaveAccessibleDescription(/online/);
    expect(row).toHaveAccessibleDescription(/read, spend/);
  });

  it('renders no unattended text or control for any row', () => {
    render(
      <DispatchTargetList
        instances={[fleetInstance(), fleetInstance({ id: 'inst-5', present: false, scopes: [] })]}
        selectedId="inst-1"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.queryByText(/unattended/i)).toBeNull();
    expect(screen.queryByRole('checkbox')).toBeNull();
    expect(screen.queryByRole('switch')).toBeNull();
    expect(screen.queryAllByRole('button')).toHaveLength(2);
  });
});
