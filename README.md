# Nostr Social Graph

**Decentralized content filtering and moderation for Nostr, in TypeScript and Rust.**

Nostr identities are free to create, so spammers can generate endless accounts.
Nostr Social Graph lets you limit whose events you care about: people you follow,
people they follow, or a wider circle you choose. Use follow distances and mute
lists to filter feeds, replies, notifications, and search results. Each user
chooses their own starting point and moderation rules.

- Build and update the graph from Nostr follow and mute lists.
- Query follows, followers, distances, and mutes relative to any user.
- Save and load compact binary snapshots, with a pre-crawled bootstrap available.
- Use TypeScript in web apps or Rust with in-memory and persistent LMDB backends.

[Try the demo](https://graph.iris.to) · [TypeScript API](https://mmalmi.github.io/nostr-social-graph/docs/)

## Quickstart

```sh
npm install nostr-social-graph
```

```ts
import { SocialGraph, type NostrEvent } from "nostr-social-graph";

const root = "<your hex pubkey>";
const graph = new SocialGraph(root);

// Supply verified kind 3 (follow) and 10000 (mute) events from your app.
// Allow unknown authors while loading the graph.
graph.handleEvent(followAndMuteEvents, true);
await graph.recalculateFollowDistances();

// Show content from yourself, your follows, and their follows, excluding mutes.
function shouldShow(event: NostrEvent): boolean {
  return graph.getFollowDistance(event.pubkey) <= 2 &&
    !graph.getMutedByUser(root).has(event.pubkey);
}
```

Use `shouldShow` when filtering incoming content. Distance `0` is you, `1` is a
direct follow, and `2` is a follow of a follow. The app fetches and verifies events
and chooses how far to extend trust. See the [TypeScript guide](./ts/README.md)
for persistence and root changes, or the [Rust guide](./rust/README.md) to get started in Rust.

## Used by

- [Iris](https://iris.to) — social feeds and moderation.
- [Iris Chat](https://chat.iris.to) — social graph and private contact helpers.
- [Hashtree](https://github.com/mmalmi/hashtree/tree/master/rust/crates/hashtree-cli#social-graph) — storage access based on follow distance.
- [Haps](https://github.com/mmalmi/haps#social-discovery-and-release-attestations) — package discovery and publisher filtering.

## Documentation

- [TypeScript](./ts/README.md) and [Rust](./rust/README.md): setup, APIs, and development commands.
- [Demo and data](./ts/examples/README.md), [server](./ts/server/README.md), and [bootstrap snapshot](./ts/README.md#published-bootstrap-snapshot).
- [Private contact memory and sync](./guides/private-contact-memory.md): names, favorites, notes, mutes, and migration.
- [Fact events](./nips/fact-events.md): draft protocol for identity and entity data.

Main development is on [decentralized git](https://git.iris.to/#/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/nostr-social-graph).
[Ask DeepWiki](https://deepwiki.com/mmalmi/nostr-social-graph).
