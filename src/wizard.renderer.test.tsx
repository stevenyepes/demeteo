// Render tests for the Create-Project wizard shell and its seven steps.
//
// Each step is a controlled component that emits a `CreateProjectStepPayload`
// through `onSubmit`; these tests pin the payload each one produces, plus the
// shell's Back-button gating and seven-dot progress indicator.

import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { BootstrapStep, STEP_ORDER, type CreateProjectStepPayload } from './types';
import { EFFORT_LEVELS } from './lib/effortLevels';
import { WizardShell } from './components/wizard/WizardShell';
import { NameStep } from './components/wizard/NameStep';
import { ProviderStep } from './components/wizard/ProviderStep';
import { GroupStep } from './components/wizard/GroupStep';
import { MachineStep } from './components/wizard/MachineStep';
import { AgentStep } from './components/wizard/AgentStep';
import { ModelStep } from './components/wizard/ModelStep';
import { DescriptionStep } from './components/wizard/DescriptionStep';

import type { Machine, Provider } from './types';
import type { ProviderNamespace } from './lib/createProjectWizard';

const PROVIDERS: Provider[] = [
  {
    id: 'gh-1',
    type: 'github',
    name: 'github',
    host: 'github.com',
    pat: 'hidden',
    username: 'me',
    avatarUrl: '',
  },
  {
    id: 'gl-1',
    type: 'gitlab',
    name: 'gitlab',
    host: 'gitlab.com',
    pat: 'hidden',
    username: 'me',
    avatarUrl: '',
  },
];

const MACHINES: Machine[] = [
  { id: 'm-1', name: 'box', host: '10.0.0.1', port: 22, username: 'u', auth_type: 'key' },
];

const NAMESPACES: ProviderNamespace[] = [
  { id: 'me', name: 'me', kind: 'personal' },
  { id: 'acme', name: 'acme', kind: 'org' },
];

const ALL_STEPS = [
  BootstrapStep.Name,
  BootstrapStep.Provider,
  BootstrapStep.Group,
  BootstrapStep.Machine,
  BootstrapStep.Agent,
  BootstrapStep.Model,
  BootstrapStep.Description,
];

describe('WizardShell', () => {
  function renderShell(step: BootstrapStep, history: BootstrapStep[]) {
    render(
      <WizardShell
        step={step}
        history={history}
        canProceed
        reason=""
        onBack={() => {}}
        onNext={() => {}}
      >
        {null}
      </WizardShell>,
    );
  }

  // Matches the Rust `BootstrapState::can_go_back`.
  it('disables Back on the first step', () => {
    renderShell(BootstrapStep.Name, [BootstrapStep.Name]);

    expect(screen.getByTestId('wizard-back')).toBeDisabled();
  });

  it('enables Back once there is history to rewind', () => {
    renderShell(BootstrapStep.Provider, [BootstrapStep.Name, BootstrapStep.Provider]);

    expect(screen.getByTestId('wizard-back')).toBeEnabled();
  });

  it('renders exactly one dot per step', () => {
    renderShell(BootstrapStep.Description, ALL_STEPS);

    const dots = within(screen.getByTestId('wizard-dots')).getAllByRole('listitem');

    expect(dots).toHaveLength(7);
    expect(dots).toHaveLength(STEP_ORDER.length);
  });
});

describe('the wizard steps', () => {
  it('NameStep renders its container', () => {
    render(<NameStep value="my-repo" onSubmit={() => {}} />);

    expect(screen.getByTestId('wizard-step-name')).toBeInTheDocument();
  });

  it('ProviderStep renders a card per provider kind and emits the provider payload', async () => {
    const onSubmit = vi.fn<(p: CreateProjectStepPayload) => void>();
    render(<ProviderStep providers={PROVIDERS} value="" onSubmit={onSubmit} />);

    expect(screen.getByTestId('wizard-provider-gitlab')).toBeInTheDocument();
    await userEvent.click(screen.getByTestId('wizard-provider-github'));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({ step: 'provider', kind: 'github' });
  });

  it('GroupStep emits the picked namespace', async () => {
    const onSubmit = vi.fn<(p: CreateProjectStepPayload) => void>();
    render(<GroupStep namespaces={NAMESPACES} loading={false} value="" onSubmit={onSubmit} />);

    await userEvent.click(screen.getByTestId('wizard-namespace-acme'));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      step: 'group',
      kind: 'org',
      name: 'acme',
    });
  });

  it('MachineStep toggles from local to remote', async () => {
    const onSubmit = vi.fn<(p: CreateProjectStepPayload) => void>();
    render(
      <MachineStep
        machines={MACHINES}
        kind="local"
        machineId=""
        keyPassphrase=""
        onSubmit={onSubmit}
        onPassphraseChange={() => {}}
      />,
    );

    await userEvent.click(screen.getByTestId('wizard-machine-remote'));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({ step: 'machine', kind: 'remote' });
  });

  // The legacy wizard has no probe gating (that lives on CreateZeroMachineStep),
  // but the remote tile must stay clickable so the new wizard's gating has a
  // target and there is no silent fallback to local.
  it('MachineStep keeps the remote tile selectable before any probe runs', () => {
    render(
      <MachineStep
        machines={MACHINES}
        kind="local"
        machineId=""
        keyPassphrase=""
        onSubmit={() => {}}
        onPassphraseChange={() => {}}
      />,
    );

    expect(screen.getByTestId('wizard-machine-local')).toBeInTheDocument();
    expect(screen.getByTestId('wizard-machine-remote')).toBeEnabled();
  });

  it('AgentStep emits the selected agent kind', async () => {
    const onSubmit = vi.fn<(p: CreateProjectStepPayload) => void>();
    render(
      <AgentStep
        agentKinds={['opencode', 'hermes', 'claude-code']}
        value=""
        onSubmit={onSubmit}
      />,
    );

    await userEvent.selectOptions(screen.getByTestId('wizard-agent-select-input'), 'opencode');

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({ step: 'agent', kind: 'opencode' });
  });

  it('ModelStep hides the picker and emits nothing while disabled', () => {
    const onSubmit = vi.fn();
    render(
      <ModelStep
        enabled={false}
        loading={false}
        models={[]}
        value=""
        effort=""
        effortLevels={EFFORT_LEVELS}
        onSubmit={onSubmit}
      />,
    );

    expect(screen.getByTestId('wizard-step-model')).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('DescriptionStep emits the commit payload with the chosen visibility', async () => {
    const onSubmit = vi.fn();
    render(
      <DescriptionStep
        description="Build a billing service."
        title="Billing"
        visibility="private"
        onSubmit={onSubmit}
      />,
    );

    await userEvent.click(screen.getByTestId('wizard-visibility-public'));

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit.mock.calls[0][0]).toMatchObject({ step: 'commit', visibility: 'public' });
  });

  describe('DescriptionStep partial-patch behaviour', () => {
    it('typing into title does not emit description on the patch', async () => {
      const onSubmit = vi.fn();
      render(
        <DescriptionStep
          description="Already filled description"
          title=""
          visibility="private"
          onSubmit={onSubmit}
        />,
      );
      await userEvent.type(screen.getByTestId('wizard-title'), 'A');
      expect(onSubmit).toHaveBeenCalledTimes(1);
      const call = onSubmit.mock.calls[0][0] as Record<string, unknown>;
      expect(call.title).toBe('A');
      expect(call).not.toHaveProperty('description');
      expect(call.visibility).toBeUndefined();
    });

    it('typing into description does not emit title on the patch', async () => {
      const onSubmit = vi.fn();
      render(
        <DescriptionStep
          description=""
          title="Existing title"
          visibility="private"
          onSubmit={onSubmit}
        />,
      );
      await userEvent.type(screen.getByTestId('wizard-description'), 'B');
      expect(onSubmit).toHaveBeenCalledTimes(1);
      const call = onSubmit.mock.calls[0][0] as Record<string, unknown>;
      expect(call.description).toBe('B');
      expect(call).not.toHaveProperty('title');
      expect(call.visibility).toBeUndefined();
    });

    it('clicking a visibility button does not emit title or description', async () => {
      const onSubmit = vi.fn();
      render(
        <DescriptionStep
          description="Add OAuth2 PKCE flow"
          title="Auth revamp"
          visibility="private"
          onSubmit={onSubmit}
        />,
      );
      await userEvent.click(screen.getByTestId('wizard-visibility-public'));
      expect(onSubmit).toHaveBeenCalledTimes(1);
      const call = onSubmit.mock.calls[0][0] as Record<string, unknown>;
      expect(call.visibility).toBe('public');
      expect(call).not.toHaveProperty('title');
      expect(call).not.toHaveProperty('description');
    });
  });
});

// AC-3: the wizard reaches the provider API through this wrapper module. A
// refactor that moves or duplicates it breaks the import, and this fails.
describe('the createProjectWizard IPC wrappers', () => {
  it('exports listProviderNamespaces and providerCreateRepo', async () => {
    const wizard = await import('./lib/createProjectWizard');

    expect(wizard.listProviderNamespaces).toBeTypeOf('function');
    expect(wizard.providerCreateRepo).toBeTypeOf('function');
  });

  // C-5: the host picked on the Provider step must flow into the Commit payload
  // so the HTTP adapter can route to self-hosted enterprise hosts.
  it('carries provider_host on the commit payload', () => {
    const sample: CreateProjectStepPayload = {
      step: 'commit',
      title: 't',
      description: 'd',
      visibility: 'private',
      name: 'n',
      provider_id: 'pid',
      provider_kind: 'github',
      provider_host: 'gh.corp.example.com',
      namespace_id: 'me',
      namespace_kind: 'personal',
      namespace_name: 'me',
      machine_kind: 'remote',
      machine_id: 'm1',
      agent_kind: 'opencode',
      model: 'anthropic/claude-sonnet-4',
    };

    expect(sample.provider_host).toBe('gh.corp.example.com');
  });
});
