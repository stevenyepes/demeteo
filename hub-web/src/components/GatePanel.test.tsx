import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { GatePanel } from './GatePanel';

const INSTANCE_ID = '7f3e2a10-9c4b-4d2e-8f1a-0b6c5d4e3f21';
const FEATURE_ID = 'f-42';
const STEP_EXECUTION_ID = 'se-approve';
const REQUEST_ID = 'q7Zr-1_x';
const CHALLENGE_PATH = `/v1/instances/${INSTANCE_ID}/gates/challenge`;
const DECISION_PATH = `/v1/instances/${INSTANCE_ID}/gates/decision`;

/** Unix seconds; the fake clock starts here. */
const NOW = 1_700_000_000;
const EXPIRES_AT = NOW + 60;

type Sent = { method: string; path: string; body: unknown };
type Reply = { status: number; body: unknown } | Promise<Response>;

/** Every request a double was not told about, held to `[]` after each test. */
let unscripted: string[] = [];

/** Both doubles write here, so a test reads the order across fetch and the browser. */
let order: string[] = [];

/** Answers each `"METHOD path"` with its replies in order; anything else, or one call too many, throws. */
function stubHub(script: Record<string, Reply[]>) {
  const sent: Sent[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: unknown, init?: RequestInit): Promise<Response> => {
      const method = init?.method ?? 'GET';
      const request = `${method} ${String(input)}`;
      const reply = script[request]?.shift();
      if (reply === undefined) {
        unscripted.push(request);
        throw new Error(`fetch double was not told about ${request} here`);
      }
      order.push(request);
      sent.push({
        method,
        path: String(input),
        body: typeof init?.body === 'string' ? JSON.parse(init.body) : init?.body,
      });
      if (reply instanceof Promise) return reply;
      return new Response(JSON.stringify(reply.body), {
        status: reply.status,
        headers: { 'Content-Type': 'application/json' },
      });
    }),
  );
  return { sent };
}

/** Answers `get` with `answer` and refuses `create`, which no gate decision reaches. */
function stubWebAuthn(answer: () => Promise<unknown> = async () => ASSERTION) {
  const credentials = {
    create: vi.fn(async () => {
      throw new Error('navigator.credentials.create double was not told to answer');
    }),
    get: vi.fn((_options?: CredentialRequestOptions) => {
      order.push('credentials.get');
      return answer();
    }),
  };
  vi.stubGlobal('navigator', { credentials });
  vi.stubGlobal('PublicKeyCredential', class {});
  return credentials;
}

function buffer(...octets: number[]): ArrayBuffer {
  return new Uint8Array(octets).buffer;
}

/** The Hub asks for less than the gate needs; hub-web must not take it at its word. */
const challenge = {
  status: 200,
  body: {
    ceremony_id: 'cer-9',
    request_id: REQUEST_ID,
    expires_at: EXPIRES_AT,
    gate_challenge: 'AAECAw',
    public_key: {
      challenge: '-_-_',
      rpId: 'hub.example',
      allowCredentials: [{ type: 'public-key', id: 'BAU' }],
      userVerification: 'discouraged',
    },
  },
};

const ASSERTION = {
  rawId: buffer(0xfb, 0xff),
  type: 'public-key',
  response: {
    clientDataJSON: buffer(1, 2, 3),
    authenticatorData: buffer(4, 5, 6, 7),
    signature: buffer(8),
    userHandle: null,
  },
};

/** `ASSERTION` as `getGateAssertion` encodes it. */
const ASSERTION_FIELDS = {
  credential_id: '-_8',
  authenticator_data: 'BAUGBw',
  client_data_json: 'AQID',
  signature: 'CA',
};

const ticket = { status: 202, body: { request_id: REQUEST_ID, expires_at: EXPIRES_AT } };

function renderPanel(overrides: Partial<ComponentProps<typeof GatePanel>> = {}) {
  const onDecided = vi.fn();
  const rendered = render(
    <GatePanel
      instanceId={INSTANCE_ID}
      featureId={FEATURE_ID}
      stepExecutionId={STEP_EXECUTION_ID}
      scopes={['read', 'gates']}
      pending={false}
      onDecided={onDecided}
      {...overrides}
    />,
  );
  return { ...rendered, onDecided };
}

const DECIDED = { requestId: REQUEST_ID, expiresAt: EXPIRES_AT };

async function advance(ms: number) {
  await act(() => vi.advanceTimersByTimeAsync(ms));
}

function button(name: string): HTMLElement {
  return screen.getByRole('button', { name });
}

const LABELS = { approve: 'Approve', cancel: 'Abort feature' } as const;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
  vi.setSystemTime(NOW * 1000);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
  order = [];
  const strays = unscripted;
  unscripted = [];
  // Soft: a throw here would skip the setup file's own `afterEach`.
  expect.soft(strays).toEqual([]);
});

describe('GatePanel', () => {
  it.each(['approve', 'cancel'] as const)(
    '%s: challenge, then one required-UV passkey prompt, then the decision',
    async (decision) => {
      const { sent } = stubHub({
        [`POST ${CHALLENGE_PATH}`]: [challenge],
        [`POST ${DECISION_PATH}`]: [ticket],
      });
      const credentials = stubWebAuthn();
      const { onDecided } = renderPanel();

      fireEvent.click(button(LABELS[decision]));
      await advance(0);

      expect(credentials.get).toHaveBeenCalledTimes(1);
      expect(credentials.get.mock.calls[0]?.[0]?.publicKey?.userVerification).toBe('required');
      expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get', `POST ${DECISION_PATH}`]);
      expect(sent[0]?.body).toStrictEqual({
        feature_id: FEATURE_ID,
        step_execution_id: STEP_EXECUTION_ID,
        decision,
      });
      expect(sent[1]?.body).toStrictEqual({ ceremony_id: 'cer-9', request_id: REQUEST_ID, assertion: ASSERTION_FIELDS });
      expect(onDecided.mock.calls).toEqual([[DECIDED]]);
    },
  );

  it('reads no request status itself: the gate leaves the mirror before the instance answers', async () => {
    stubHub({
      [`POST ${CHALLENGE_PATH}`]: [challenge],
      [`POST ${DECISION_PATH}`]: [ticket],
    });
    stubWebAuthn();
    renderPanel();

    fireEvent.click(button('Approve'));
    await advance(60_000);

    expect(screen.queryByRole('status')).toBeNull();
    expect(button('Approve')).toBeEnabled();
  });

  it('reports a decision the Hub queued after the panel was closed', async () => {
    let answer: (credential: unknown) => void = () => {};
    stubHub({
      [`POST ${CHALLENGE_PATH}`]: [challenge],
      [`POST ${DECISION_PATH}`]: [ticket],
    });
    stubWebAuthn(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const { onDecided, unmount } = renderPanel();
    fireEvent.click(button('Approve'));
    await advance(0);

    unmount();
    answer(ASSERTION);
    await advance(0);

    expect(onDecided.mock.calls).toEqual([[DECIDED]]);
  });

  it('shows Redirect disabled, saying the Hub does not offer it yet', () => {
    stubHub({});
    renderPanel();

    const redirect = button('Redirect');
    expect(redirect).toBeDisabled();
    expect(redirect).toHaveAccessibleDescription(/not available from the Hub yet/i);
  });

  it('says every decision asks for the passkey, and offers no feedback the instance would discard', () => {
    stubHub({});
    renderPanel();

    expect(screen.getByText(/every gate decision asks for your passkey/i)).toBeInTheDocument();
    expect(screen.queryByLabelText(/feedback/i)).toBeNull();
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByText(/feedback can only be given on the desktop/i)).toBeInTheDocument();
  });

  it('disables every button while the ceremony is in flight, so a double click spends one challenge', async () => {
    let answer: (credential: unknown) => void = () => {};
    stubHub({
      [`POST ${CHALLENGE_PATH}`]: [challenge],
      [`POST ${DECISION_PATH}`]: [ticket],
    });
    const credentials = stubWebAuthn(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const { onDecided } = renderPanel();

    fireEvent.click(button('Approve'));
    fireEvent.click(button('Approve'));
    await advance(0);

    expect(button('Approve')).toBeDisabled();
    expect(button('Abort feature')).toBeDisabled();
    expect(button('Redirect')).toBeDisabled();
    fireEvent.click(button('Abort feature'));
    await advance(0);
    expect(credentials.get).toHaveBeenCalledTimes(1);

    answer(ASSERTION);
    await advance(0);

    expect(order.filter((each) => each === `POST ${CHALLENGE_PATH}`)).toHaveLength(1);
    expect(onDecided).toHaveBeenCalledTimes(1);
  });

  it('is disabled, naming the gates scope, when the instance has not granted it', () => {
    stubHub({});
    renderPanel({ scopes: ['read', 'spend'] });

    expect(button('Approve')).toBeDisabled();
    expect(button('Abort feature')).toBeDisabled();
    expect(screen.getByRole('group')).toHaveAccessibleDescription(/needs the gates scope/i);
  });

  it('holds both decisions, saying why, while a sent decision waits for the instance', async () => {
    stubHub({});
    const credentials = stubWebAuthn();
    const { rerender, onDecided } = renderPanel({ pending: true });

    expect(button('Approve')).toBeDisabled();
    expect(button('Abort feature')).toBeDisabled();
    expect(screen.getByRole('group')).toHaveAccessibleDescription(/waiting for the instance/i);
    fireEvent.click(button('Approve'));
    await advance(0);
    expect(credentials.get).not.toHaveBeenCalled();

    rerender(
      <GatePanel
        instanceId={INSTANCE_ID}
        featureId={FEATURE_ID}
        stepExecutionId={STEP_EXECUTION_ID}
        scopes={['read', 'gates']}
        pending={false}
        onDecided={onDecided}
      />,
    );
    expect(button('Approve')).toBeEnabled();
    expect(screen.queryByText(/waiting for the instance/i)).toBeNull();
  });

  it('leaves the decision to the Hub while the scopes are unknown', () => {
    stubHub({});
    renderPanel({ scopes: null });

    expect(button('Approve')).toBeEnabled();
  });

  it.each([
    'ceremony_expired',
    'challenge_mismatch',
  ])('shows %s as a refusal to start again from, without resubmitting', async (code) => {
    stubHub({
      [`POST ${CHALLENGE_PATH}`]: [challenge],
      [`POST ${DECISION_PATH}`]: [{ status: 400, body: { error: code } }],
    });
    const credentials = stubWebAuthn();
    renderPanel();

    fireEvent.click(button('Approve'));
    await advance(0);

    expect(screen.getByRole('alert')).toHaveTextContent(code);
    expect(screen.getByText(/start again/i)).toBeInTheDocument();
    expect(credentials.get).toHaveBeenCalledTimes(1);
    expect(button('Approve')).toBeEnabled();
  });

  it('sends no decision when the passkey prompt is cancelled', async () => {
    stubHub({ [`POST ${CHALLENGE_PATH}`]: [challenge] });
    stubWebAuthn(async () => {
      throw new DOMException('dismissed', 'NotAllowedError');
    });
    renderPanel();

    fireEvent.click(button('Abort feature'));
    await advance(0);

    expect(screen.getByRole('alert')).toHaveTextContent(/passkey prompt was cancelled/i);
    expect(order).toEqual([`POST ${CHALLENGE_PATH}`, 'credentials.get']);
  });
});
