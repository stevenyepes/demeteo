// The decoder and the copy, asserted against the shapes Rust actually emits.
//
// The strings in `WIRE` below are copied from
// `crates/demeteo-core/tests/domain/mr_list_error.rs`, whose
// `serialized_shape_is_the_wire_contract` asserts `MrListError` serializes to
// exactly them. That is the whole of the cross-language pin: rename a variant or
// a field on either side and one of the two suites goes red. Hand-written JSON
// in a shape nothing produces would validate the mock instead.

import { describe, expect, it } from 'vitest';

import {
  asPullRequestListFailure,
  describeListFailure,
  providerName,
  truncateDetail,
} from './pullRequests';

const WIRE = {
  noProvider: '{"kind":"no-provider"}',
  noCredential:
    '{"kind":"no-credential","provider":"github","host":"api.github.com","detail":"no entry found"}',
  unauthorized:
    '{"kind":"unauthorized","provider":"github","host":"api.github.com","status":401}',
  rateLimited: '{"kind":"rate-limited","host":"gitlab.com","retry_after":30}',
  http: '{"kind":"http","host":"api.github.com","status":500,"body":"boom"}',
  internal: '{"kind":"http","host":"","status":null,"body":"database query failed"}',
};

describe('asPullRequestListFailure', () => {
  it('decodes every variant the Rust enum serializes', () => {
    expect(asPullRequestListFailure(WIRE.noProvider)).toEqual({ kind: 'no-provider' });
    expect(asPullRequestListFailure(WIRE.noCredential)).toEqual({
      kind: 'no-credential',
      provider: 'github',
      host: 'api.github.com',
      detail: 'no entry found',
    });
    expect(asPullRequestListFailure(WIRE.unauthorized)).toEqual({
      kind: 'unauthorized',
      provider: 'github',
      host: 'api.github.com',
      status: 401,
    });
    expect(asPullRequestListFailure(WIRE.rateLimited)).toEqual({
      kind: 'rate-limited',
      host: 'gitlab.com',
      retry_after: 30,
    });
    expect(asPullRequestListFailure(WIRE.http)).toEqual({
      kind: 'http',
      host: 'api.github.com',
      status: 500,
      body: 'boom',
    });
  });

  it('keeps a status-less internal failure out of the provider-blaming copy', () => {
    const failure = asPullRequestListFailure(WIRE.internal);

    expect(failure).toEqual({ kind: 'http', host: 'The provider', status: null, body: 'database query failed' });
    expect(describeListFailure(failure).detail).toBe('database query failed');
  });

  it('accepts an already-decoded object as well as the JSON string', () => {
    expect(asPullRequestListFailure({ kind: 'no-provider' })).toEqual({ kind: 'no-provider' });
  });

  it('carries an unrecognisable rejection through verbatim rather than losing it', () => {
    // Tauri refusing the call, or any error that is not this envelope: still a
    // real failure, and the text is the only evidence the user has.
    const failure = asPullRequestListFailure(
      new Error('Command list_open_pull_requests not found'),
    );

    expect(failure.kind).toBe('http');
    if (failure.kind === 'http') {
      expect(failure.body).toContain('list_open_pull_requests');
      expect(failure.status).toBeNull();
    }
  });
});

describe('describeListFailure', () => {
  it('sends a missing connection to the provider manager, not to a retry', () => {
    const copy = describeListFailure({ kind: 'no-provider' });

    expect(copy.title).toBe('No provider connected');
    expect(copy.actions.map((a) => a.intent)).toEqual(['connect']);
    expect(copy.actions[0].label).toBe('Connect a provider');
  });

  it('says nothing was sent when the token never left the keyring', () => {
    // The whole reason this failure is not `unauthorized`: told a host answered
    // 401, the user goes and audits the scopes of a token that is fine.
    const copy = describeListFailure({
      kind: 'no-credential',
      provider: 'github',
      host: 'api.github.com',
      detail: 'No matching entry found in secure storage',
    });

    expect(copy.title).toBe('No GitHub token is stored');
    expect(copy.body).toContain('Nothing was sent to api.github.com');
    expect(copy.body).not.toContain('answered');
    expect(copy.detail).toBe('No matching entry found in secure storage');
    expect(copy.actions.map((a) => a.label)).toEqual(['Reconnect GitHub', 'Retry']);
  });

  it('renders no empty evidence block when the keyring said nothing', () => {
    const copy = describeListFailure({
      kind: 'no-credential',
      provider: 'gitlab',
      host: 'gitlab.com',
      detail: '',
    });

    expect(copy.detail).toBeUndefined();
    expect(copy.body).not.toContain('answer:');
  });

  it('names the provider it wants reconnected', () => {
    const copy = describeListFailure({
      kind: 'unauthorized',
      provider: 'gitlab',
      host: 'gitlab.example.com',
      status: 403,
    });

    expect(copy.title).toBe('Your GitLab token was rejected');
    expect(copy.body).toContain('gitlab.example.com answered 403');
    expect(copy.actions.map((a) => a.label)).toEqual(['Reconnect GitLab', 'Retry']);
  });

  it('spends the retry-after the provider gave, and stays vague without one', () => {
    const withHint = describeListFailure({
      kind: 'rate-limited',
      host: 'api.github.com',
      retry_after: 90,
    });
    const without = describeListFailure({
      kind: 'rate-limited',
      host: 'api.github.com',
      retry_after: null,
    });

    expect(withHint.body).toContain('in about 2 min');
    expect(describeListFailure({ kind: 'rate-limited', host: 'h', retry_after: 45 }).body).toContain(
      'in about 45s',
    );
    expect(without.body).toContain('shortly');
    expect(without.body).not.toContain('about');
  });

  it('quotes the provider response rather than summarising it', () => {
    const copy = describeListFailure({
      kind: 'http',
      host: 'api.github.com',
      status: 503,
      body: '  {"message":"upstream unavailable"}  ',
    });

    expect(copy.detail).toBe('{"message":"upstream unavailable"}');
    expect(copy.actions.map((a) => a.intent)).toEqual(['retry']);
  });
});

describe('truncateDetail', () => {
  it('leaves a body at the limit whole and cuts one past it', () => {
    expect(truncateDetail('x'.repeat(600))).toHaveLength(600);

    const cut = truncateDetail('x'.repeat(601));
    expect(cut).toHaveLength(601);
    expect(cut.endsWith('…')).toBe(true);
  });

  it('caps a provider HTML page long before it reaches the DOM', () => {
    expect(truncateDetail('<html>'.repeat(5_000)).length).toBeLessThan(700);
  });
});

describe('providerName', () => {
  it('spells each provider the way the provider spells itself', () => {
    expect(providerName('github')).toBe('GitHub');
    expect(providerName('gitlab')).toBe('GitLab');
    expect(providerName('GITHUB')).toBe('GitHub');
  });

  it('passes an unknown kind through rather than inventing a name for it', () => {
    expect(providerName('gitea')).toBe('gitea');
  });
});
