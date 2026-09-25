import { describe, expect, it } from 'vitest';

import { GATE_BODY_MAX_CHARS, GATE_BODY_MAX_LINES, sanitizedGateBody } from './gateRedaction';

const GH_TOKEN = `ghp_${'a1B2c3D4e5'.repeat(3)}${'x9Y8z7'}`;

describe('sanitizedGateBody', () => {
  it.each([
    ['/home/alice/proj/a.ts:1', '~/proj/a.ts:1'],
    ['at file:///home/alice/proj/a.ts', 'at file://~/proj/a.ts'],
    ['/Users/alice/proj', '~/proj'],
    ['cd /root/app && npm test', 'cd ~/app && npm test'],
    ['C:\\Users\\Alice\\proj', '~\\proj'],
    ['c:/users/alice/proj', '~/proj'],
    ['"C:\\\\Users\\\\Alice\\\\proj"', '"~\\\\proj"'],
  ])('replaces the home directory in %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  it('leaves a repo path that only contains a home-like segment alone', () => {
    for (const path of ['app/home/page.tsx:4', 'src/Users/list.ts', 'rootfs/x']) {
      expect(sanitizedGateBody(path)).toBe(path);
    }
  });

  it.each([
    ['API_KEY=abc123', 'API_KEY=[redacted]'],
    ['  export AWS_SECRET=xyz', '  export AWS_SECRET=[redacted]'],
    ['set TOKEN=xyz', 'set TOKEN=[redacted]'],
    ['declare -x NPM_TOKEN="xyz"', 'declare -x NPM_TOKEN=[redacted]'],
    ['+ SECRET=xyz ./run.sh', '+ SECRET=[redacted]'],
  ])('redacts the value of the assignment %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  it('leaves assertions that only look like assignments to the tool that printed them', () => {
    const body = 'expected=5 received=6\nAssertionError: want FOO=1\nif (token === undefined)';
    expect(sanitizedGateBody(body)).toBe(body);
  });

  it.each([
    ["  API_KEY: 'abc123secret',", "  API_KEY: '[redacted]',"],
    ['  STRIPE: "rk_live_x"', '  STRIPE: "[redacted]"'],
    ['DATABASE_URL: postgres://u:p@db/app', 'DATABASE_URL: [redacted]'],
    ["{ NODE_ENV: 'test', HOME: '/x' }", "{ NODE_ENV: '[redacted]', HOME: '[redacted]' }"],
  ])('redacts the value of the upper-case colon assignment %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  it('redacts the value of a quoted upper-case key in a JSON env dump', () => {
    expect(sanitizedGateBody('  "DB_PASSWORD": "hunter2",')).toBe('  "DB_PASSWORD": "[redacted]",');
  });

  it.each([
    ['Running with STRIPE_KEY=rk_live_x npm test', 'Running with STRIPE_KEY=[redacted] npm test'],
    ['sh -c "NPM_TOKEN=abc npm ci"', 'sh -c "NPM_TOKEN=[redacted] npm ci"'],
    ["run 'DB_URL=a b' now", "run 'DB_URL=[redacted] b' now"],
    ['env A_B="x y" ./t', 'env A_B="[redacted]" ./t'],
  ])('redacts the value of the mid-line assignment %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  it.each([
    ['aws_secret_access_key = wJalrXUtnFEMI', 'aws_secret_access_key = [redacted]'],
    ['  "db_password": "hunter2"', '  "db_password": "[redacted]"'],
    ["config { apiKey: 'k1', api-key=k2 }", "config { apiKey: '[redacted]', api-key=[redacted] }"],
    ['passwd:x token=y', 'passwd:[redacted] token=[redacted]'],
    ['Private_Key : "abc"', 'Private_Key : "[redacted]"'],
    ['spring.datasource.password: hunter2', 'spring.datasource.password: [redacted]'],
  ])('redacts the value of the credential-named key %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  it.each([
    ['config/secrets.env:3:DB_PASSWORD=hunter2', 'config/secrets.env:[redacted]'],
    ['src/token.ts:12:password=hunter2', 'src/token.ts:[redacted]'],
    ['token.ts:DB_PASSWORD=hunter2', 'token.ts:[redacted]'],
    ['auth.token.refresh: password=hunter2', 'auth.token.refresh: [redacted]'],
    ['token.ts: `DB_PASSWORD=hunter2`', 'token.ts: [redacted]'],
  ])('redacts an assignment consumed after a file-shaped name in %j', (input, expected) => {
    expect(sanitizedGateBody(input)).toBe(expected);
  });

  // The gate evidence a fix run needs: none of these may lose a word.
  it('leaves error, assertion and JSON status lines whole', () => {
    const body = [
      "ERROR: Cannot find module 'x'",
      "TS2345: Argument of type 'string'",
      'FAIL src/a.test.ts',
      'Expected: "foo"',
      'Received: "bar"',
      '  "status": "ok",',
    ].join('\n');
    expect(sanitizedGateBody(body)).toBe(body);
  });

  // A file whose name holds a credential keyword is still a file, and a
  // compiler quoting the token it choked on is not printing a secret.
  it('leaves source locations, quoted tokens and error codes whole', () => {
    const body = [
      'at parseToken (src/lexer/token.ts:12:5)',
      'src/token.rs:12: expected token, found',
      'error: expected token: `;`',
      'src/auth/token.service.ts:40:3',
      'api-key.guard.ts:7:1',
      'src/secrets.rs:3:9',
      "ERR_MODULE_NOT_FOUND: Cannot find package 'x'",
    ].join('\n');
    expect(sanitizedGateBody(body)).toBe(body);
  });

  it('replaces a private key block with one line', () => {
    const body = [
      'before',
      '-----BEGIN RSA PRIVATE KEY-----',
      'MIIEpAIBAAKCAQEA0Z3VS5JJcds3xfn',
      'kVsWjbHgYzT2w0ZMQ==',
      '-----END RSA PRIVATE KEY-----',
      'after',
    ].join('\n');
    expect(sanitizedGateBody(body)).toBe('before\n[redacted private key]\nafter');
  });

  it.each(['PRIVATE KEY', 'OPENSSH PRIVATE KEY', 'EC PRIVATE KEY', 'PGP PRIVATE KEY BLOCK'])(
    'recognises a %s block, including one printed on a single line',
    (label) => {
      const body = `key=-----BEGIN ${label}-----\\nMIIsecret\\n-----END ${label}----- ok`;
      expect(sanitizedGateBody(`x ${body}`)).toBe('x key=[redacted private key] ok');
    },
  );

  it('redacts an unterminated private key block to the end of the body', () => {
    const body = 'FAIL x\n-----BEGIN PRIVATE KEY-----\nMIIsecret\nmore';
    expect(sanitizedGateBody(body)).toBe('FAIL x\n[redacted private key]');
  });

  // Cut first, the header would be gone and the key lines would pass for output.
  it('redacts a private key block before it cuts', () => {
    const keyLines = Array.from({ length: GATE_BODY_MAX_LINES + 50 }, (_, i) => `MIIkey${i}`);
    const body = [
      'FAIL x',
      '-----BEGIN RSA PRIVATE KEY-----',
      ...keyLines,
      '-----END RSA PRIVATE KEY-----',
      'done',
    ].join('\n');
    expect(sanitizedGateBody(body)).toBe('FAIL x\n[redacted private key]\ndone');
  });

  it.each([
    GH_TOKEN,
    `gho_${'a'.repeat(36)}`,
    `github_pat_${'A1_'.repeat(20)}`,
    `glpat-${'x1-'.repeat(8)}`,
    `sk-ant-api03-${'Zz9'.repeat(10)}`,
    `sk-${'q'.repeat(40)}`,
    'xoxb-1234567890-abcdef',
    'AKIAABCDEFGHIJKLMNOP',
  ])('redacts the credential %s', (token) => {
    expect(sanitizedGateBody(`token: ${token} rejected`)).toBe('token: [redacted] rejected');
  });

  it('keeps the scheme and host of a URL whose credentials it removes', () => {
    expect(sanitizedGateBody('fetch https://bot:s3cret@git.example.com/a.git')).toBe(
      'fetch https://[redacted]@git.example.com/a.git',
    );
    expect(sanitizedGateBody('https://u:p@h/x')).toBe('https://[redacted]@h/x');
    expect(sanitizedGateBody('git+ssh://me:pw@host')).toBe('git+ssh://[redacted]@host');
    expect(sanitizedGateBody('(https://u:p@h)')).toBe('(https://[redacted]@h)');
    expect(sanitizedGateBody('Authorization: Bearer abc.def-ghi_jkl')).toBe(
      'Authorization: Bearer [redacted]',
    );
  });

  it.each(['a-', 'a.'])('stays linear on a long run of %j', (unit) => {
    const body = unit.repeat(100_000);
    const started = performance.now();
    expect(sanitizedGateBody(body).endsWith(unit)).toBe(true);
    expect(performance.now() - started).toBeLessThan(500);
  });

  it('stays linear on repeated file-shaped credential names', () => {
    const body = 'token.ts:'.repeat(30_000);
    const started = performance.now();
    expect(sanitizedGateBody(body)).not.toContain('token.ts:token.ts:');
    expect(performance.now() - started).toBeLessThan(500);
  });

  it.each(['10.0.0.5', '172.16.4.2', '172.31.255.1', '192.168.1.20'])(
    'redacts the private address %s',
    (ip) => {
      expect(sanitizedGateBody(`connect ${ip}:5432 refused`)).toBe(
        'connect [redacted-ip]:5432 refused',
      );
    },
  );

  it('leaves public addresses and longer dotted numbers whole', () => {
    const body = 'connect 8.8.8.8 and 172.32.0.1; v10.0.0.5; 1.10.0.0.5';
    expect(sanitizedGateBody(body)).toBe(body);
  });

  it('carries a short body with every line break unchanged', () => {
    const body = 'FAIL x\r\n  at y\rprogress\nok';
    expect(sanitizedGateBody(body)).toBe(body);
  });

  it('caps a body of short lines at the line limit and counts what it cut', () => {
    const body = Array.from({ length: GATE_BODY_MAX_LINES + 1 }, (_, i) => `l${i}`).join('\n');
    const [marker, ...kept] = sanitizedGateBody(body).split('\n');

    expect(marker).toBe('[gate output truncated: 1 earlier line omitted]');
    expect(kept).toHaveLength(GATE_BODY_MAX_LINES);
    expect(kept[0]).toBe('l1');
  });

  it('caps a body of long lines at the character limit, on a line boundary', () => {
    const line = 'x'.repeat(999);
    const body = Array.from({ length: 30 }, () => line).join('\n');
    const [marker, ...kept] = sanitizedGateBody(body).split('\n');

    expect(kept).toHaveLength(12);
    expect(kept.join('\n').length).toBeLessThanOrEqual(GATE_BODY_MAX_CHARS);
    expect(marker).toBe('[gate output truncated: 18 earlier lines omitted]');
  });

  it('keeps the end of a single line longer than the character limit', () => {
    const body = `head\n${'a'.repeat(GATE_BODY_MAX_CHARS)}END`;
    const [marker, kept] = sanitizedGateBody(body).split('\n');

    expect(marker).toBe(
      '[gate output truncated: 1 earlier line and the start of the line below omitted]',
    );
    expect(kept).toHaveLength(GATE_BODY_MAX_CHARS);
    expect(kept.endsWith('END')).toBe(true);
  });

  // Redacting after the cut could leave a token's suffix no pattern knows.
  it('redacts before it cuts', () => {
    const body = `${GH_TOKEN} ${'y'.repeat(GATE_BODY_MAX_CHARS - GH_TOKEN.length + 19)}`;

    expect(sanitizedGateBody(body)).not.toContain(GH_TOKEN.slice(-20));
    expect(sanitizedGateBody(body).startsWith('[redacted] y')).toBe(true);
  });
});
