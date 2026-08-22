// The one place in the suite that renders the *real* react-markdown. Every
// other test stubs it because it is heavy (`FeatureDetail.test.tsx` and
// friends), and a stub cannot tell a working renderer from a broken one — it
// hands the raw string straight back either way.

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { AgentMarkdown } from './AgentMarkdown';

afterEach(cleanup);

describe('the interviewer’s prose', () => {
  it('renders inline code as a code element rather than backticks', () => {
    render(<AgentMarkdown text="Fan-out lives in `path.rs:77`, not in the driver." />);

    const code = screen.getByText('path.rs:77');
    expect(code.tagName).toBe('CODE');
    expect(screen.getByTestId('agent-markdown').textContent).not.toContain('`');
  });

  it('renders a fenced block as code that scrolls inside its own container', () => {
    const { container } = render(
      <AgentMarkdown text={'Try:\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n'} />,
    );

    const code = container.querySelector('pre code, div code');
    expect(code?.textContent).toBe('let a = 1;\nlet b = 2;');
    expect(code?.parentElement?.className).toContain('overflow-x-auto');
  });

  it('renders bullets and numbers as list items', () => {
    const { container } = render(
      <AgentMarkdown text={'- first\n- second\n\n1. one\n2. two\n'} />,
    );

    expect(container.querySelectorAll('ul > li')).toHaveLength(2);
    expect(container.querySelectorAll('ol > li')).toHaveLength(2);
    expect(screen.getByText('second').tagName).toBe('LI');
  });

  it('renders headings, emphasis and blockquotes', () => {
    const { container } = render(
      <AgentMarkdown text={'## Topology\n\n**already** and *there*\n\n> a quote\n'} />,
    );

    expect(container.querySelector('h2')?.textContent).toBe('Topology');
    expect(container.querySelector('strong')?.textContent).toBe('already');
    expect(container.querySelector('em')?.textContent).toBe('there');
    expect(container.querySelector('blockquote')?.textContent).toContain('a quote');
  });

  it('renders a GFM table, in a container of its own', () => {
    const { container } = render(
      <AgentMarkdown text={'| join | means |\n|---|---|\n| all_success | every edge |\n'} />,
    );

    expect(container.querySelectorAll('th')).toHaveLength(2);
    expect(screen.getByText('all_success').tagName).toBe('TD');
    expect(container.querySelector('table')?.parentElement?.className).toContain('overflow-x-auto');
  });

  it('sends a link somewhere other than this webview', () => {
    render(<AgentMarkdown text="See [the spec](https://example.com/spec)." />);

    const link = screen.getByRole('link', { name: 'the spec' });
    expect(link).toHaveAttribute('href', 'https://example.com/spec');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
  });

  it('refuses a javascript: url', () => {
    const { container } = render(<AgentMarkdown text="[click](javascript:alert(1))" />);

    const link = container.querySelector('a');
    expect(link?.textContent).toBe('click');
    expect(link?.getAttribute('href') ?? '').not.toContain('javascript:');
  });

  // Agent output. Turning it into markup is the one thing this renderer must
  // never do, and react-markdown's default of emitting HTML as text is what
  // stops it — `rehype-raw` would undo exactly this assertion.
  it('does not render embedded HTML', () => {
    const { container } = render(
      <AgentMarkdown text={'<img src="x" onerror="boom"> and <b>bold</b>'} />,
    );

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('b')).toBeNull();
    expect(container.textContent).toContain('<b>bold</b>');
  });

  it('is memoized, so a settled message parses once', () => {
    expect(AgentMarkdown).toHaveProperty('$$typeof', Symbol.for('react.memo'));
  });
});
