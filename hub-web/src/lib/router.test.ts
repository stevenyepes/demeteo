import { describe, expect, it } from 'vitest';

import { parseRoute, routeHref, type Route } from './router';

type Linkable = Exclude<Route, { name: 'not-found' }>;

const SIX_ROUTES: Array<[hash: string, route: Linkable]> = [
  ['#/fleet', { name: 'fleet' }],
  ['#/instances/i-1', { name: 'instance', instanceId: 'i-1' }],
  ['#/runs', { name: 'runs' }],
  ['#/instances/i-1/runs/f-9', { name: 'run', instanceId: 'i-1', featureId: 'f-9' }],
  ['#/dispatch', { name: 'dispatch' }],
  ['#/add-instance', { name: 'add-instance' }],
];

describe('routeHref', () => {
  it.each(SIX_ROUTES)('spells %s', (hash, route) => {
    expect(routeHref(route)).toBe(hash);
  });

  it('encodes each id as one path segment', () => {
    expect(routeHref({ name: 'run', instanceId: 'lab/box 1', featureId: 'f/9' })).toBe(
      '#/instances/lab%2Fbox%201/runs/f%2F9',
    );
  });
});

describe('parseRoute', () => {
  it.each(SIX_ROUTES)('round-trips %s', (_hash, route) => {
    expect(parseRoute(routeHref(route))).toEqual(route);
  });

  it.each(['', '#', '#/'])('reads the empty hash %j as fleet', (hash) => {
    expect(parseRoute(hash)).toEqual({ name: 'fleet' });
  });

  it('reports an unknown path as not-found, carrying the path', () => {
    expect(parseRoute('#/nowhere')).toEqual({ name: 'not-found', path: '/nowhere' });
  });

  it.each(['lab/box', 'lab box', '50%', 'add-instance'])('round-trips the id %j', (id) => {
    const instance: Linkable = { name: 'instance', instanceId: id };
    const run: Linkable = { name: 'run', instanceId: id, featureId: id };

    expect(parseRoute(routeHref(instance))).toEqual(instance);
    expect(parseRoute(routeHref(run))).toEqual(run);
  });

  it('tolerates one trailing slash', () => {
    expect(parseRoute('#/runs/')).toEqual({ name: 'runs' });
    expect(parseRoute('#/instances/i-1/')).toEqual({ name: 'instance', instanceId: 'i-1' });
  });

  it('reports an over-long path as not-found', () => {
    expect(parseRoute('#/runs/x/y')).toEqual({ name: 'not-found', path: '/runs/x/y' });
    expect(parseRoute('#/instances/i-1/runs/f-9/extra').name).toBe('not-found');
  });

  it('reports a malformed percent-escape as not-found instead of throwing', () => {
    expect(parseRoute('#/instances/%E0%A4%A')).toEqual({
      name: 'not-found',
      path: '/instances/%E0%A4%A',
    });
  });

  it('refuses an empty id rather than routing to an instance with no name', () => {
    expect(parseRoute('#/instances/').name).toBe('not-found');
    expect(parseRoute('#/instances//runs/f-9').name).toBe('not-found');
    expect(parseRoute('#/instances/i-1/runs/').name).toBe('not-found');
  });

  it('does not let an id shadow the add-instance route', () => {
    expect(parseRoute('#/instances/add')).toEqual({ name: 'instance', instanceId: 'add' });
    expect(parseRoute('#/add-instance')).toEqual({ name: 'add-instance' });
  });
});
