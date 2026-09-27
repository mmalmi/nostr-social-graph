import { describe, expect, it } from "vitest";
import { finalizeEvent, getPublicKey, verifyEvent } from "nostr-tools";
import { SocialGraph } from "../src/SocialGraph";
import {
  chooseTrustedAuthors,
  countDistinctTrustedAuthors,
} from "../src/trustedAuthors";

const key = (n: number) => {
  const bytes = new Uint8Array(32);
  new DataView(bytes.buffer).setUint32(28, n);
  return bytes;
};
const pub = (n: number) => getPublicKey(key(n));
const root = pub(1);
const honest = pub(2);
const compromised = pub(3);
const stranger = pub(4);

function follows(n: number, contacts: string[], createdAt = 100) {
  const event = finalizeEvent(
    {
      kind: 3,
      content: "",
      created_at: createdAt,
      tags: contacts.map((p) => ["p", p]),
    },
    key(n),
  );
  // Match the verified-event boundary used by production graph consumers.
  const received = JSON.parse(JSON.stringify(event));
  expect(verifyEvent(received)).toBe(true);
  return received;
}

function select(graph: SocialGraph, eligible: Iterable<string>) {
  return chooseTrustedAuthors({
    rootPubkey: graph.getRoot(),
    eligibleAuthors: eligible,
    directFollows: graph.getFollowedByUser(graph.getRoot()),
    mutedAuthors: graph.getMutedByUser(graph.getRoot()),
  });
}

describe("nondelegated trusted-author authority", () => {
  it("requires both application eligibility and direct trust, and respects mutes", () => {
    expect(
      chooseTrustedAuthors({
        rootPubkey: root,
        eligibleAuthors: [root, honest, compromised, stranger],
        directFollows: new Set([honest, compromised]),
        mutedAuthors: new Set([compromised]),
      }),
    ).toEqual(new Set([root, honest]));
    expect(
      chooseTrustedAuthors({
        rootPubkey: root,
        eligibleAuthors: [stranger],
        directFollows: new Set([honest]),
      }).size,
    ).toBe(0);
    expect(() =>
      chooseTrustedAuthors({
        rootPubkey: "invalid",
        eligibleAuthors: [],
        directFollows: new Set(),
      }),
    ).toThrow("Invalid root");
  });

  it("returns deterministic independent snapshots and ignores malformed authors", () => {
    const eligible = new Set([root, honest, "invalid"]);
    const direct = new Set([honest, "invalid"]);
    const snapshot = chooseTrustedAuthors({
      rootPubkey: root,
      eligibleAuthors: eligible,
      directFollows: direct,
    });
    expect([...snapshot]).toEqual([...snapshot].sort());
    eligible.clear();
    direct.clear();
    expect(snapshot).toEqual(new Set([root, honest]));
    expect(
      countDistinctTrustedAuthors(
        [root, root, honest, stranger, honest],
        snapshot,
      ),
    ).toBe(2);
  });

  for (const size of [1000, 10000]) {
    it(`keeps ${size} Sybils and repeated signals within the compromised author's one vote`, () => {
      const graph = new SocialGraph(root);
      graph.handleEvent(follows(1, [honest, compromised]), true);
      // Distinct canonical identifiers are sufficient for the authority bound;
      // the signed attacker follow event is verified through the real parser.
      const sybils = Array.from({ length: size }, (_, i) =>
        (i + 100).toString(16).padStart(64, "0"),
      );
      graph.handleEvent(follows(3, sybils), true);
      const admitted = [root, honest, compromised, ...sybils];
      const authority = select(graph, admitted);
      expect(authority).toEqual(new Set([root, honest, compromised]));
      expect(
        countDistinctTrustedAuthors(
          [compromised, ...sybils, ...sybils, compromised],
          authority,
        ),
      ).toBe(1);
      expect(select(graph, [...admitted].reverse())).toEqual(authority);

      // One compromised member follows more accounts; the viewer's explicit
      // trust boundary stays unchanged even if every new identity is admitted.
      const anotherBatch = sybils.map((_, i) =>
        (i + size + 100).toString(16).padStart(64, "0"),
      );
      graph.handleEvent(follows(3, [...sybils, ...anotherBatch], 110), true);
      expect(select(graph, [...admitted, ...anotherBatch])).toEqual(authority);

      // Revocation affects a fresh view, while an already signed poll can keep
      // the earlier explicit snapshot. Replayed old follows cannot restore it.
      graph.handleEvent(follows(1, [honest], 120), true);
      graph.handleEvent(follows(1, [honest, compromised]), true);
      expect(select(graph, admitted)).toEqual(new Set([root, honest]));
      expect(authority.has(compromised)).toBe(true);
    });
  }

  it("makes compromised root or explicitly trusted collusion a visible assumption boundary", () => {
    const graph = new SocialGraph(root);
    graph.handleEvent(follows(1, [honest]), true);
    const admitted = [root, honest, compromised, stranger];
    const frozen = select(graph, admitted);
    // Expected counterexample, not a passing claim of resistance: the root can
    // explicitly grant authority to new keys in a new view or future poll.
    graph.handleEvent(follows(1, [honest, compromised, stranger], 110), true);
    const replaced = select(graph, admitted);
    expect(replaced.size).toBe(4);
    expect(countDistinctTrustedAuthors([compromised, stranger], replaced)).toBe(
      2,
    );
    expect(countDistinctTrustedAuthors([compromised, stranger], frozen)).toBe(
      0,
    );
  });
});
