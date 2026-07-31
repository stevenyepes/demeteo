import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { invoke } from '@tauri-apps/api/core';

import StartFeatureModal from './StartFeatureModal';

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

function renderModal(onLaunch = vi.fn()) {
  render(
    <StartFeatureModal
      isOpen
      projectId="project-1"
      repositories={[]}
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
