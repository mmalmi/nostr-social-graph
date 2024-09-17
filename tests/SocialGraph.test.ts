import { describe, it, expect } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { NostrEvent } from '../src/utils';

const pubKeys = {
    adam: "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e",
    fiatjaf: "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d",
    snowden: "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240",
}

describe('SocialGraph', () => {
  it('should initialize with root user', () => {
    const graph = new SocialGraph(pubKeys.adam);
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0);
  });

  it('should handle follow events', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    graph.handleEvent(event);
    expect(graph.isFollowing(pubKeys.adam, pubKeys.fiatjaf)).toBe(true);
  });

  it('should update follow distances correctly', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event1: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.snowden]],
      kind: 3,
      pubkey: pubKeys.fiatjaf,
      id: 'event2',
      sig: 'signature',
    };
    graph.handleEvent(event1);
    graph.handleEvent(event2);
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(2);
  });

  it('should serialize and deserialize correctly', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event1: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.snowden]],
      kind: 3,
      pubkey: pubKeys.fiatjaf,
      id: 'event2',
      sig: 'signature',
    };
    graph.handleEvent(event1);
    graph.handleEvent(event2);

    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0)
    expect(graph.getFollowDistance(pubKeys.fiatjaf)).toBe(1);
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(2);
    expect(graph.isFollowing(pubKeys.adam, pubKeys.fiatjaf)).toBe(true);
    expect(graph.isFollowing(pubKeys.fiatjaf, pubKeys.snowden)).toBe(true);

    const serialized = graph.serialize();
    const newGraph = new SocialGraph(pubKeys.adam, serialized);

    expect(newGraph.getFollowDistance(pubKeys.adam)).toBe(0)
    expect(newGraph.getFollowDistance(pubKeys.fiatjaf)).toBe(1);
    expect(newGraph.getFollowDistance(pubKeys.snowden)).toBe(2);
    expect(newGraph.isFollowing(pubKeys.adam, pubKeys.fiatjaf)).toBe(true);
    expect(newGraph.isFollowing(pubKeys.fiatjaf, pubKeys.snowden)).toBe(true);
  });

  it('should update follow distances when root is changed', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event1: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Date.now(),
      content: '',
      tags: [['p', pubKeys.snowden]],
      kind: 3,
      pubkey: pubKeys.fiatjaf,
      id: 'event2',
      sig: 'signature',
    };
    graph.handleEvent(event1);
    graph.handleEvent(event2);

    // Initial follow distances
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0);
    expect(graph.getFollowDistance(pubKeys.fiatjaf)).toBe(1);
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(2);

    graph.setRoot(pubKeys.snowden);

    // Snowden doesn't follow anyone.
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(0);
    expect(graph.getFollowDistance(pubKeys.fiatjaf)).toBe(1000);
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(1000);

    graph.setRoot(pubKeys.fiatjaf);

    // Fiatjaf follows Snowden.
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(1);
    expect(graph.getFollowDistance(pubKeys.fiatjaf)).toBe(0);
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(1000);

    graph.setRoot(pubKeys.adam)
    // Initial follow distances
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0);
    expect(graph.getFollowDistance(pubKeys.fiatjaf)).toBe(1);
    expect(graph.getFollowDistance(pubKeys.snowden)).toBe(2);
  });
});