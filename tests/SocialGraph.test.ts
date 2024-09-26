import { describe, it, expect } from 'vitest';
import { SocialGraph } from '../src/SocialGraph';
import { NostrEvent } from '../src/utils';
import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const pubKeys = {
    adam: "020f2d21ae09bf35fcdfb65decf1478b846f5f728ab30c5eaabcd6d081a81c3e",
    fiatjaf: "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d",
    snowden: "84dee6e676e5bb67b4ad4e042cf70cbd8681155db535942fcc6a0533858a7240",
    sirius: "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0",
}

const SOCIAL_GRAPH_FILE = path.join(__dirname, '../data/socialGraph.json');

describe('SocialGraph', () => {
  it('should initialize with root user', () => {
    const graph = new SocialGraph(pubKeys.adam);
    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0);
  });

  it('should handle follow events', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000) / 1000,
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
      created_at: Math.floor(Date.now() / 1000),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
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
      created_at: Math.floor(Date.now() / 1000),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
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
      created_at: Math.floor(Date.now() / 1000),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
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

  it('should load social graph from crawled JSON file', () => {
    const jsonFilePath = path.join(__dirname, '../data/socialGraph.json');
    const jsonData = fs.readFileSync(jsonFilePath, 'utf-8');
    const graph = new SocialGraph(pubKeys.adam, JSON.parse(jsonData));

    expect(graph.getFollowDistance(pubKeys.adam)).toBe(0);
  });

  it('should validate the structure of the crawled social graph', () => {
    if (!fs.existsSync(SOCIAL_GRAPH_FILE)) {
      throw new Error('Social graph file does not exist');
    }

    const jsonData = fs.readFileSync(SOCIAL_GRAPH_FILE, 'utf-8');
    const parsedData = JSON.parse(jsonData);

    // Check followLists structure
    expect(Array.isArray(parsedData.followLists)).toBe(true);
    parsedData.followLists.forEach((followList: any) => {
      expect(Array.isArray(followList)).toBe(true);
      expect(followList.length).toBe(3); // pubkey, followed users, list timestamp
      expect(typeof followList[0]).toBe('number');
      const followedUsers = followList[1]
      expect(Array.isArray(followedUsers)).toBe(true);
      followedUsers.forEach((id: any) => {
        expect(typeof id).toBe('number');
      });
      const listTimestamp = followList[2]
      expect(typeof listTimestamp).toBe('number');
    });

    // Check uniqueIds structure
    expect(Array.isArray(parsedData.uniqueIds)).toBe(true);
    parsedData.uniqueIds.forEach((uniqueId: any) => {
      expect(Array.isArray(uniqueId)).toBe(true);
      expect(uniqueId.length).toBe(2);
      expect(typeof uniqueId[0]).toBe('string');
      expect(typeof uniqueId[1]).toBe('number');
    });

    // Attempt to load the graph to ensure it's valid
    const graph = new SocialGraph('rootPubKey', parsedData);
    expect(graph).toBeInstanceOf(SocialGraph);
  });

  it('should utilize existing follow lists for new users', () => {
    const graph = new SocialGraph(pubKeys.adam);
    const event1: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
      content: '',
      tags: [['p', pubKeys.fiatjaf]],
      kind: 3,
      pubkey: pubKeys.adam,
      id: 'event1',
      sig: 'signature',
    };
    const event2: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
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
    const newGraph = new SocialGraph(pubKeys.sirius, serialized);

    expect(newGraph.getFollowDistance(pubKeys.sirius)).toBe(0)
    expect(newGraph.getFollowDistance(pubKeys.adam)).toBe(1000)
    expect(newGraph.getFollowDistance(pubKeys.fiatjaf)).toBe(1000);
    expect(newGraph.getFollowDistance(pubKeys.snowden)).toBe(1000);
    expect(newGraph.isFollowing(pubKeys.adam, pubKeys.fiatjaf)).toBe(true);
    expect(newGraph.isFollowing(pubKeys.fiatjaf, pubKeys.snowden)).toBe(true);

    const event3: NostrEvent = {
      created_at: Math.floor(Date.now() / 1000),
      content: '',
      tags: [['p', pubKeys.adam]],
      kind: 3,
      pubkey: pubKeys.sirius,
      id: 'event3',
      sig: 'signature',
    };
    graph.handleEvent(event3);
    expect(newGraph.getFollowDistance(pubKeys.sirius)).toBe(0)
    expect(newGraph.getFollowDistance(pubKeys.adam)).toBe(1)
    expect(newGraph.getFollowDistance(pubKeys.fiatjaf)).toBe(2);
    expect(newGraph.getFollowDistance(pubKeys.snowden)).toBe(3);
  });
});