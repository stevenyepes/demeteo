import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import StartFeatureModal from './StartFeatureModal';
import { STANDARD_STARTER_WORKFLOW_ID } from '../lib/workflowDefault';

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: vi.fn().mockResolvedValue(() => {}),
  }),
}));

vi.mock('../lib/agentCatalog', () => ({
  useAgentCatalog: () => ({ agents: [] }),
  effortLevelsFor: () => [],
}));

interface ClipboardItemFixture {
  kind: string;
  type: string;
  getAsFile: () => File | null;
}

function clipboardData(items: ClipboardItemFixture[]): DataTransfer {
  return { items } as unknown as DataTransfer;
}

function paste(node: Element, items: ClipboardItemFixture[]) {
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', { value: clipboardData(items) });
  const preventDefault = vi.spyOn(event, 'preventDefault');
  fireEvent(node, event);
  return preventDefault;
}

function imageItem(file: File): ClipboardItemFixture {
  return { kind: 'file', type: file.type, getAsFile: () => file };
}

function textItem(): ClipboardItemFixture {
  return { kind: 'string', type: 'text/plain', getAsFile: () => null };
}

function unavailableImageItem(): ClipboardItemFixture {
  return { kind: 'file', type: 'image/png', getAsFile: () => null };
}

function renderModal(onLaunch = vi.fn(), defaultWorkflowId?: string | null) {
  render(
    <StartFeatureModal
      isOpen
      projectId="project-1"
      repositories={[]}
      defaultWorkflowId={defaultWorkflowId}
      onClose={vi.fn()}
      onLaunch={onLaunch}
    />,
  );
  return onLaunch;
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((command: string) => {
    switch (command) {
      case 'workflow_list':
        return Promise.resolve([{ id: 'workflow-1', name: 'Default', version: 1 }]);
      case 'workflow_get':
        return Promise.resolve({ steps: [], version_id: 'version-1' });
      case 'workflow_version_graph':
        return Promise.resolve(null);
      case 'get_machines':
      case 'get_agent_configs':
      case 'fetch_active_features':
        return Promise.resolve([]);
      default:
        return Promise.resolve(undefined);
    }
  });
});

describe('StartFeatureModal clipboard paste', () => {
  it('stages an image pasted into the initially focused title once and launches it', async () => {
    const onLaunch = renderModal();
    const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);
    const image = new File(['image bytes'], 'pasted.png', { type: 'image/png' });

    await waitFor(() => expect(title).toHaveFocus());

    const preventDefault = paste(title, [imageItem(image)]);

    expect(preventDefault).toHaveBeenCalledTimes(1);
    await screen.findByText('pasted.png');
    expect(screen.getAllByRole('button', { name: /^remove /i })).toHaveLength(1);

    fireEvent.change(title, { target: { value: 'Pasted image feature' } });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'Describe the pasted image feature' },
    });
    await waitFor(() => expect(screen.getByRole('button', { name: /launch feature/i })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: /launch feature/i }));

    expect(onLaunch).toHaveBeenCalledWith(expect.objectContaining({
      attachments: [expect.objectContaining({
        name: 'pasted.png',
        file: image,
        sourcePath: null,
      })],
    }));
  });

  it('recovers an image from the async clipboard after WebKitGTK supplies empty items', async () => {
    const clipboardRead = vi.fn().mockResolvedValue([{
      types: ['image/png'],
      getType: vi.fn().mockResolvedValue(new Blob(['png bytes'], { type: 'image/png' })),
    }]);
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { read: clipboardRead } });
    try {
      renderModal();
      const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

      paste(title, []);

      await waitFor(() => expect(clipboardRead).toHaveBeenCalledTimes(1));
      expect(await screen.findByText('pasted-image.png')).toBeInTheDocument();
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it('shows a soft error when the async clipboard read is denied', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { read: vi.fn().mockRejectedValue(new DOMException('denied', 'NotAllowedError')) },
    });
    try {
      renderModal();
      const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

      paste(title, []);

      expect(await screen.findByRole('alert')).toHaveTextContent(/could not read image bytes/i);
    } finally {
      if (previousClipboard) Object.defineProperty(navigator, 'clipboard', previousClipboard);
      else Reflect.deleteProperty(navigator, 'clipboard');
    }
  });

  it.each([
    ['title', /add oauth2 login flow/i],
    ['description', /what does this feature do/i],
  ])('leaves text-only paste native in the %s', async (_field, placeholder) => {
    renderModal();
    const input = await screen.findByPlaceholderText(placeholder);

    const preventDefault = paste(input, [textItem()]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(screen.queryByRole('button', { name: /^remove /i })).not.toBeInTheDocument();
  });

  it('shows the attachment soft error for an unavailable image without consuming text paste', async () => {
    renderModal();
    const title = await screen.findByPlaceholderText(/add oauth2 login flow/i);

    const preventDefault = paste(title, [textItem(), unavailableImageItem()]);

    expect(preventDefault).not.toHaveBeenCalled();
    expect(await screen.findByRole('alert')).toHaveTextContent(
      /clipboard offered an image, but this webview could not access its file/i,
    );
    expect(screen.queryByRole('button', { name: /^remove /i })).not.toBeInTheDocument();
  });
});

/**
 * The launch path the audit's F10 finding is really about: the header claimed a
 * default the schema did not have, but the modal *behaved* as if one existed by
 * taking whatever `workflow_list` returned first.
 *
 * Every list below is ordered so that the removed `workflows[0]` fall-through
 * would answer differently from the rule — otherwise these pass against the bug
 * they exist to pin.
 */
describe('StartFeatureModal workflow default', () => {
  const listed = [
    { id: 'wf-alpha', name: 'Alpha', version: 1 },
    { id: 'wf-beta', name: 'Beta', version: 2 },
    { id: STANDARD_STARTER_WORKFLOW_ID, name: 'Standard Feature Pipeline', version: 3 },
  ];

  function mockBackend(options: {
    workflows?: Array<{ id: string; name: string; version: number }>;
    storedDefault?: string | null;
    settingsFail?: boolean;
    /** Held-open reads. Both loads resolve in the same tick under jsdom, which
     *  hides the orderings the app actually sees — a SQLite settings read and a
     *  workflow list have no fixed order between them, and the seed is wrong if
     *  it commits before either one answers. */
    settingsPending?: Promise<unknown>;
    workflowsPending?: Promise<unknown>;
  }) {
    vi.mocked(invoke).mockImplementation((command: string) => {
      switch (command) {
        case 'workflow_list':
          return options.workflowsPending ?? Promise.resolve(options.workflows ?? listed);
        case 'workflow_get':
          return Promise.resolve({ steps: [], version_id: 'version-1' });
        case 'workflow_version_graph':
          return Promise.resolve(null);
        case 'get_proposed_strategy':
          if (options.settingsFail) return Promise.reject(new Error('settings unavailable'));
          return options.settingsPending
            ?? Promise.resolve({ default_workflow_id: options.storedDefault ?? null });
        case 'get_machines':
        case 'get_agent_configs':
        case 'fetch_active_features':
          return Promise.resolve([]);
        default:
          return Promise.resolve(undefined);
      }
    });
  }

  function picker(): HTMLSelectElement {
    return screen.getByLabelText('Workflow') as HTMLSelectElement;
  }

  async function fillAndLaunch() {
    fireEvent.change(screen.getByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'A feature' },
    });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'What the feature does' },
    });
    const button = screen.getByRole('button', { name: /launch feature/i });
    await waitFor(() => expect(button).toBeEnabled());
    fireEvent.click(button);
  }

  it('launches on the workflow the project stored, not the first one listed', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    const onLaunch = renderModal();

    await waitFor(() => expect(picker().value).toBe('wf-beta'));
    await fillAndLaunch();

    expect(onLaunch).toHaveBeenCalledWith(expect.objectContaining({ workflowId: 'wf-beta' }));
  });

  it('prefers the workflow the caller pointed at over the stored default', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    renderModal(vi.fn(), 'wf-alpha');

    await waitFor(() => expect(picker().value).toBe('wf-alpha'));
  });

  it('falls to the standard starter when the stored default no longer exists', async () => {
    mockBackend({ storedDefault: 'wf-deleted' });
    renderModal();

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
  });

  it('treats an unreadable project setting as unset instead of blocking the launch', async () => {
    mockBackend({ settingsFail: true });
    const onLaunch = renderModal();

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
    await fillAndLaunch();

    expect(onLaunch).toHaveBeenCalledWith(
      expect.objectContaining({ workflowId: STANDARD_STARTER_WORKFLOW_ID }),
    );
  });

  it('asks rather than guesses when no rule names a workflow', async () => {
    mockBackend({ workflows: listed.slice(0, 2) });
    renderModal();

    await screen.findByText('Choose a workflow…');
    expect(picker().value).toBe('');
    fireEvent.change(screen.getByPlaceholderText(/add oauth2 login flow/i), {
      target: { value: 'A feature' },
    });
    fireEvent.change(screen.getByPlaceholderText(/what does this feature do/i), {
      target: { value: 'What the feature does' },
    });
    expect(screen.getByRole('button', { name: /launch feature/i })).toBeDisabled();
  });

  it('waits for the stored setting instead of seeding a fallback ahead of it', async () => {
    let landed: (settings: unknown) => void = () => {};
    mockBackend({
      settingsPending: new Promise((resolve) => {
        landed = resolve;
      }),
    });
    renderModal();

    await screen.findByText('Beta (v2)');
    expect(picker().value).toBe('');

    landed({ default_workflow_id: 'wf-beta' });
    await waitFor(() => expect(picker().value).toBe('wf-beta'));
  });

  it('waits for the workflow list instead of latching on an empty one', async () => {
    let landed: (workflows: unknown) => void = () => {};
    mockBackend({
      storedDefault: 'wf-beta',
      workflowsPending: new Promise((resolve) => {
        landed = resolve;
      }),
    });
    renderModal();

    await act(async () => {});
    expect(picker().value).toBe('');

    landed(listed);
    await waitFor(() => expect(picker().value).toBe('wf-beta'));
  });

  it('never re-seeds over a pick the user made after the modal opened', async () => {
    mockBackend({ storedDefault: 'wf-beta' });
    const modal = (seedTitle?: string) => (
      <StartFeatureModal
        isOpen
        projectId="project-1"
        repositories={[]}
        defaultWorkflowId={STANDARD_STARTER_WORKFLOW_ID}
        seedTitle={seedTitle}
        onClose={vi.fn()}
        onLaunch={vi.fn()}
      />
    );
    const { rerender } = render(modal());

    await waitFor(() => expect(picker().value).toBe(STANDARD_STARTER_WORKFLOW_ID));
    fireEvent.change(picker(), { target: { value: 'wf-alpha' } });
    rerender(modal(''));

    await waitFor(() => expect(picker().value).toBe('wf-alpha'));
  });
});
