import { UniqueIds, canonicalNodeId } from './UniqueIds.js';
import { isValidPubKey, NostrEvent } from './utils.js';
import * as Binary from './SocialGraphBinary.js';
import { SocialGraphUtils } from './SocialGraphUtils.js';

export interface ListRevision { id: string; created_at: number; complete: boolean }
export interface GraphListRevision extends ListRevision { pubkey: string; kind: 3 | 10000 }

/** Reverse-index slot storing a single follower/muter inline without allocating a Set. */
export type CompactEdgeEntry = number | Set<number>;

const hexEventIdPattern = /^[0-9a-f]{64}$/;
const MAX_DENSE_DISTANCES = 1_000_000;

/**
 * Dense `Int32Array` distance table that transitions cleanly to a `Map` if a
 * sparse legacy ID arrives, avoiding any key shadowing between dense & sparse.
 */
export class DistanceTable {
  private dense: Int32Array | null;
  private sparse: Map<number, number> | null;
  private count = 0;

  constructor(denseCapacity = 0) {
    if (denseCapacity > 0 && denseCapacity <= MAX_DENSE_DISTANCES) {
      this.dense = new Int32Array(denseCapacity).fill(-1);
      this.sparse = null;
    } else {
      this.dense = null;
      this.sparse = new Map();
    }
  }

  private switchToSparse(): Map<number, number> {
    if (this.sparse) return this.sparse;
    const map = new Map<number, number>();
    const d = this.dense!;
    for (let i = 0; i < d.length; i++) {
      if (d[i] !== -1) map.set(i, d[i]);
    }
    this.dense = null;
    this.sparse = map;
    return map;
  }

  get size(): number {
    return this.dense !== null ? this.count : this.sparse!.size;
  }

  get(id: number): number | undefined {
    const d = this.dense;
    if (d !== null) {
      if (id >= 0 && id < d.length) {
        const val = d[id];
        return val === -1 ? undefined : val;
      }
      return undefined;
    }
    return this.sparse!.get(id);
  }

  has(id: number): boolean {
    const d = this.dense;
    if (d !== null) return id >= 0 && id < d.length && d[id] !== -1;
    return this.sparse!.has(id);
  }

  set(id: number, distance: number): void {
    const d = this.dense;
    if (d !== null) {
      if (id >= 0 && id < d.length) {
        if (d[id] === -1) this.count++;
        d[id] = distance;
        return;
      }
      if (id >= 0 && id < MAX_DENSE_DISTANCES && id < d.length * 2 + 64) {
        const nextLen = Math.min(MAX_DENSE_DISTANCES, Math.max(id + 1, d.length * 2));
        const grown = new Int32Array(nextLen).fill(-1);
        grown.set(d);
        this.dense = grown;
        grown[id] = distance;
        this.count++;
        return;
      }
      this.switchToSparse().set(id, distance);
      return;
    }
    this.sparse!.set(id, distance);
  }

  delete(id: number): boolean {
    const d = this.dense;
    if (d !== null) {
      if (id >= 0 && id < d.length && d[id] !== -1) {
        d[id] = -1;
        this.count--;
        return true;
      }
      return false;
    }
    return this.sparse!.delete(id);
  }

  clear(): void {
    if (this.dense) {
      this.dense.fill(-1);
      this.count = 0;
    } else {
      this.sparse!.clear();
    }
  }
}

export function addCompactEdge(
  map: Map<number, CompactEdgeEntry>,
  target: number,
  owner: number,
  materialized: boolean,
): void {
  const cur = map.get(target);
  if (cur === undefined) {
    map.set(target, materialized ? new Set([owner]) : owner);
  } else if (typeof cur === 'number') {
    if (cur !== owner) map.set(target, new Set([cur, owner]));
  } else {
    cur.add(owner);
  }
}

export function removeCompactEdge(
  map: Map<number, CompactEdgeEntry>,
  target: number,
  owner: number,
): void {
  const cur = map.get(target);
  if (cur === undefined) return;
  if (typeof cur === 'number') {
    if (cur === owner) map.delete(target);
  } else {
    cur.delete(owner);
    if (cur.size === 0) map.delete(target);
  }
}

export class SocialGraph {
  private listRevisions = new Map<string, ListRevision>();

  /** Metadata is not a signature proof. Callers must verify events before ingestion. */
  getListRevision(pubkey: string, kind: number): ListRevision | undefined {
    const revision = this.listRevisions.get(`${kind}:${canonicalNodeId(pubkey)}`);
    return revision && {...revision};
  }

  getListRevisions(): GraphListRevision[] {
    const result: GraphListRevision[] = [];
    for (const [key, revision] of this.listRevisions) {
      const split = key.indexOf(':');
      result.push({pubkey: key.slice(split + 1), kind: Number(key.slice(0, split)) as 3 | 10000, ...revision});
    }
    return result;
  }

  private acceptRevision(event: NostrEvent, previousAt: number | undefined): boolean {
    const key = `${event.kind}:${canonicalNodeId(event.pubkey)}`;
    const previous = this.listRevisions.get(key);
    const validId = hexEventIdPattern.test(event.id);
    if (previousAt !== undefined && (event.created_at < previousAt ||
      (event.created_at === previousAt && (!validId || (previous &&
        (event.id > previous.id || (event.id === previous.id && previous.complete))))))) return false;
    if (validId) this.listRevisions.set(key, {id: event.id, created_at: event.created_at, complete: true});
    else this.listRevisions.delete(key);
    return true;
  }

  markListIncomplete(pubkey: string, kind: number) {
    const revision = this.listRevisions.get(`${kind}:${canonicalNodeId(pubkey)}`);
    if (revision) revision.complete = false;
  }

  private root: number;
  private recalculatingPromise = null as Promise<void> | null;
  private followDistanceByUser = new DistanceTable(16);
  private usersByFollowDistance = new Map<number, Set<number>>();
  private followedByUser = new Map<number, Set<number>>();
  private followersByUser = new Map<number, CompactEdgeEntry>();
  private followersMaterialized = false;
  private followListCreatedAt = new Map<number, number>();
  private mutedByUser = new Map<number, Set<number>>();
  private userMutedBy = new Map<number, CompactEdgeEntry>();
  private userMutedByMaterialized = false;
  private muteListCreatedAt = new Map<number, number>();
  private ids = new UniqueIds();
  private recalculationRequested = false;

  constructor(root: string) {
    this.root = this.id(root);
    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));
  }

  private id(str: string): number {
    return this.ids.id(str);
  }

  /** Lookups must not grow the ID table: feeds query many keys the graph never stores. */
  private known(str: string): number | undefined {
    return this.ids.get(str);
  }

  private str(id: number): string {
    return this.ids.str(id);
  }

  getRoot() {
    return this.str(this.root);
  }

  private materializeFollowers(): Map<number, Set<number>> {
    if (!this.followersMaterialized) {
      for (const [k, v] of this.followersByUser) {
        if (typeof v === 'number') this.followersByUser.set(k, new Set([v]));
      }
      this.followersMaterialized = true;
    }
    return this.followersByUser as Map<number, Set<number>>;
  }

  private materializeUserMutedBy(): Map<number, Set<number>> {
    if (!this.userMutedByMaterialized) {
      for (const [k, v] of this.userMutedBy) {
        if (typeof v === 'number') this.userMutedBy.set(k, new Set([v]));
      }
      this.userMutedByMaterialized = true;
    }
    return this.userMutedBy as Map<number, Set<number>>;
  }

  getInternalData() {
    const self = this;
    return {
      listRevisions: this.listRevisions,
      followedByUser: this.followedByUser,
      get followersByUser(): Map<number, Set<number>> {
        return self.materializeFollowers();
      },
      mutedByUser: this.mutedByUser,
      get userMutedBy(): Map<number, Set<number>> {
        return self.materializeUserMutedBy();
      },
      followListCreatedAt: this.followListCreatedAt,
      muteListCreatedAt: this.muteListCreatedAt,
      ids: this.ids,
      str: (id: number) => this.str(id),
    };
  }

  private newDistanceTable(): DistanceTable {
    return new DistanceTable(this.ids.isDense ? Math.max(16, this.ids.denseLength) : 0);
  }

  setRoot(root: string): Promise<void> {
    const rootId = this.id(root);
    if (rootId === this.root) {
      return this.recalculatingPromise ?? Promise.resolve();
    }

    this.root = rootId;
    // Do not expose distances from the previous root while computing the new one.
    const distances = this.newDistanceTable();
    distances.set(rootId, 0);
    this.followDistanceByUser = distances;
    this.usersByFollowDistance = new Map([[0, new Set([rootId])]]);
    return this.recalculateFollowDistances();
  }

  /** Advance a BFS frontier up to `limit` queued users, avoiding empty-array allocations on leaves. */
  private stepFollowDistances(
    queue: number[],
    head: number,
    limit: number,
    distances: DistanceTable,
    usersByDistance: Map<number, Set<number>>,
  ): number {
    const followedByUser = this.followedByUser;
    for (; head < limit && head < queue.length; head++) {
      const user = queue[head];
      const followed = followedByUser.get(user);
      if (!followed || !followed.size) continue;
      const distance = distances.get(user)! + 1;
      let bucket = usersByDistance.get(distance);
      for (const target of followed) {
        if (distances.has(target)) continue;
        distances.set(target, distance);
        if (!bucket) usersByDistance.set(distance, bucket = new Set());
        bucket.add(target);
        queue.push(target);
      }
    }
    return head;
  }

  /** Synchronous full-graph BFS for freshly deserialized graphs with no concurrent readers. */
  recalculateFollowDistancesSync(): void {
    const root = this.root;
    const distances = this.newDistanceTable();
    distances.set(root, 0);
    const usersByDistance = new Map<number, Set<number>>([[0, new Set([root])]]);
    const queue = [root];
    this.stepFollowDistances(queue, 0, Infinity, distances, usersByDistance);
    this.followDistanceByUser = distances;
    this.usersByFollowDistance = usersByDistance;
  }

  recalculateFollowDistances(
    batchSize = 1_000,
    logEvery = 100_000,
    logger: (msg: string) => void = () => {}
  ): Promise<void> {
    if (!Number.isInteger(batchSize) || batchSize < 1) {
      return Promise.reject(new RangeError('batchSize must be a positive integer'));
    }
    this.recalculationRequested = true;
    // All callers share one calculation, including any pass needed for updates
    // received while it yields. Start in a microtask to coalesce synchronous bursts.
    this.recalculatingPromise ??= Promise.resolve().then(async () => {
      try {
        do {
          this.recalculationRequested = false;
          const root = this.root;
          const distances = this.newDistanceTable();
          distances.set(root, 0);
          const usersByDistance = new Map<number, Set<number>>([[0, new Set([root])]]);
          const queue = [root];
          let head = 0;
          const start = performance.now();
          logger(`recalculateFollowDistances: start (batchSize=${batchSize})`);

          while (head < queue.length) {
            // Newly discovered users count toward this batch too. A deep graph
            // should not need a separate timer for every degree of separation.
            head = this.stepFollowDistances(queue, head, head + batchSize, distances, usersByDistance);
            if (head > 0 && head % logEvery < batchSize) {
              logger(`recalculateFollowDistances: ${head} processed, ${queue.length - head} remaining`);
            }
            if (head < queue.length) await new Promise(resolve => setTimeout(resolve, 0));
          }
          if (!this.recalculationRequested) {
            this.followDistanceByUser = distances;
            this.usersByFollowDistance = usersByDistance;
          }
          logger(`recalculateFollowDistances: done (${head} users) in ${(performance.now() - start).toFixed(1)}ms`);
        } while (this.recalculationRequested);
      } finally {
        this.recalculatingPromise = null;
      }
    });
    return this.recalculatingPromise;
  }

  handleEvent(evs: NostrEvent | Array<NostrEvent>, allowUnknownAuthors = false, overmuteThreshold = 1): boolean {
    const maxCreatedAt = Math.floor(Date.now() / 1000) + 10 * 60;
    const isList = Array.isArray(evs);
    const count = isList ? evs.length : 1;
    let changed = false;

    for (let i = 0; i < count; i++) {
      const event = isList ? evs[i] : evs;
      if (event.kind !== 3 && event.kind !== 10000) continue;

      const createdAt = event.created_at;
      if (createdAt > maxCreatedAt) {
        console.debug("event.created_at more than 10 minutes in the future", event);
        continue;
      }

      let author: number;
      if (!allowUnknownAuthors) {
        const known = this.known(event.pubkey);
        if (known === undefined || !this.followDistanceByUser.has(known)) continue;
        author = known;
      } else {
        author = this.id(event.pubkey);
      }

      if (SocialGraphUtils.isOvermutedById(this, author, overmuteThreshold)) {
        continue;
      }

      if (event.kind === 3) {
        if (this.handleFollowList(event, author, createdAt)) changed = true;
      } else {
        if (this.handleMuteList(event, author, createdAt)) changed = true;
      }
    }

    return changed;
  }

  private parsePubkeyTags(tags: string[][], author: number): Set<number> {
    const targets = new Set<number>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i];
      if (tag[0] === 'p' && isValidPubKey(tag[1])) {
        const target = this.id(tag[1]);
        if (target !== author) targets.add(target);
      }
    }
    return targets;
  }

  private handleFollowList(event: NostrEvent, author: number, createdAt: number): boolean {
    if (!this.acceptRevision(event, this.followListCreatedAt.get(author))) {
      return false;
    }
    this.followListCreatedAt.set(author, createdAt);

    const followedInEvent = this.parsePubkeyTags(event.tags, author);
    const currentlyFollowed = this.followedByUser.get(author);

    if (currentlyFollowed) {
      for (const user of currentlyFollowed) {
        if (!followedInEvent.has(user)) {
          this.privateRemoveFollower(user, author);
        }
      }
      for (const user of followedInEvent) {
        if (!currentlyFollowed.has(user)) {
          this.privateAddFollower(user, author);
        }
      }
    } else {
      for (const user of followedInEvent) {
        this.privateAddFollower(user, author);
      }
    }

    return true;
  }

  private handleMuteList(event: NostrEvent, author: number, createdAt: number): boolean {
    if (!this.acceptRevision(event, this.muteListCreatedAt.get(author))) {
      return false;
    }
    this.muteListCreatedAt.set(author, createdAt);

    const mutedInEvent = this.parsePubkeyTags(event.tags, author);
    let currentlyMuted = this.mutedByUser.get(author);

    if (currentlyMuted) {
      for (const user of currentlyMuted) {
        if (!mutedInEvent.has(user)) {
          currentlyMuted.delete(user);
          removeCompactEdge(this.userMutedBy, user, author);
        }
      }
      if (currentlyMuted.size === 0) {
        this.mutedByUser.delete(author);
        currentlyMuted = undefined;
      }
    }

    if (mutedInEvent.size) {
      if (!currentlyMuted) this.mutedByUser.set(author, currentlyMuted = new Set<number>());
      for (const user of mutedInEvent) {
        if (currentlyMuted.has(user)) continue;
        currentlyMuted.add(user);
        addCompactEdge(this.userMutedBy, user, author, this.userMutedByMaterialized);
      }
    }

    return true;
  }

  isFollowing(follower: string, followedUser: string): boolean {
    const followedUserId = this.known(followedUser);
    const followerId = this.known(follower);
    if (followedUserId === undefined || followerId === undefined) return false;
    return !!this.followedByUser.get(followerId)?.has(followedUserId);
  }

  isMuting(muter: string, mutedUser: string): boolean {
    const mutedUserId = this.known(mutedUser);
    const muterId = this.known(muter);
    if (mutedUserId === undefined || muterId === undefined) return false;
    return !!this.mutedByUser.get(muterId)?.has(mutedUserId);
  }

  /**
   * Whether muters outweigh followers (`muters * threshold > followers`) among
   * the closest users who do either. A user muted by the root always is.
   */
  isOvermuted(user: string, threshold = 1): boolean {
    return SocialGraphUtils.isOvermuted(this, user, threshold);
  }

  getFollowDistance(user: string): number {
    const id = this.known(user);
    const distance = id === undefined ? undefined : this.followDistanceByUser.get(id);
    return distance === undefined ? 1000 : distance;
  }

  /** Keeps the distance map and its per-distance buckets in step. */
  private setDistance(user: number, distance: number | undefined) {
    const old = this.followDistanceByUser.get(user);
    if (old === distance) return;
    if (old !== undefined) {
      const bucket = this.usersByFollowDistance.get(old);
      if (bucket?.delete(user) && !bucket.size) this.usersByFollowDistance.delete(old);
    }
    if (distance === undefined) {
      this.followDistanceByUser.delete(user);
      return;
    }
    this.followDistanceByUser.set(user, distance);
    let bucket = this.usersByFollowDistance.get(distance);
    if (!bucket) this.usersByFollowDistance.set(distance, bucket = new Set());
    bucket.add(user);
  }

  /** Bring `user` to `distance` if that is closer, then everyone reached through them. */
  private lowerDistance(user: number, distance: number) {
    const current = this.followDistanceByUser.get(user);
    if (current !== undefined && current <= distance) return;
    this.setDistance(user, distance);
    const queue = [user];
    for (let head = 0; head < queue.length; head++) {
      const id = queue[head];
      const followed = this.followedByUser.get(id);
      if (!followed || !followed.size) continue;
      const next = this.followDistanceByUser.get(id)! + 1;
      for (const target of followed) {
        const known = this.followDistanceByUser.get(target);
        if (known === undefined || known > next) {
          this.setDistance(target, next);
          queue.push(target);
        }
      }
    }
  }

  /**
   * `user` lost a follower on a shortest path. Users whose every shortest path
   * ran through that edge move farther away or become unreachable.
   */
  private raiseDistance(user: number) {
    const distances = this.followDistanceByUser;
    const orphaned = new Set<number>();
    const hasCloserFollower = (id: number) => {
      const followers = this.followersByUser.get(id);
      if (followers === undefined) return false;
      const closer = distances.get(id)! - 1;
      if (typeof followers === 'number') {
        return distances.get(followers) === closer && !orphaned.has(followers);
      }
      for (const follower of followers) {
        if (distances.get(follower) === closer && !orphaned.has(follower)) return true;
      }
      return false;
    };
    if (hasCloserFollower(user)) return;
    orphaned.add(user);
    // Sets iterate in insertion order, including additions: closest users first.
    for (const id of orphaned) {
      const followed = this.followedByUser.get(id);
      if (!followed) continue;
      const next = distances.get(id)! + 1;
      for (const target of followed) {
        if (distances.get(target) === next && !orphaned.has(target) && !hasCloserFollower(target)) {
          orphaned.add(target);
        }
      }
    }
    for (const id of orphaned) this.setDistance(id, undefined);
    // Reattach through the nearest follower that is still reachable.
    for (const id of orphaned) {
      const followers = this.followersByUser.get(id);
      if (followers === undefined) continue;
      let nearest = Infinity;
      if (typeof followers === 'number') {
        const distance = distances.get(followers);
        if (distance !== undefined) nearest = distance;
      } else {
        for (const follower of followers) {
          const distance = distances.get(follower);
          if (distance !== undefined && distance < nearest) nearest = distance;
        }
      }
      if (nearest !== Infinity) this.lowerDistance(id, nearest + 1);
    }
  }

  private privateAddFollower(followedUser: number, follower: number) {
    if (typeof followedUser !== 'number' || typeof follower !== 'number') {
      throw new Error('Invalid user id');
    }

    let followed = this.followedByUser.get(follower);
    if (!followed) this.followedByUser.set(follower, followed = new Set());
    if (!followed.has(followedUser)) {
      followed.add(followedUser);
      if (this.recalculatingPromise) this.recalculationRequested = true;
    }

    // Maintain reverse index
    addCompactEdge(this.followersByUser, followedUser, follower, this.followersMaterialized);

    // A new edge can only shorten paths, and only from a reachable follower.
    const followerDistance = this.followDistanceByUser.get(follower);
    if (followerDistance !== undefined) this.lowerDistance(followedUser, followerDistance + 1);
  }

  addFollower(follower: string, followedUser: string) {
    this.markListIncomplete(follower, 3);
    this.privateAddFollower(this.id(followedUser), this.id(follower));
  }

  removeFollower(follower: string, followedUser: string) {
    this.markListIncomplete(follower, 3);
    this.privateRemoveFollower(this.id(followedUser), this.id(follower));
  }

  private privateRemoveFollower(unfollowedUser: number, follower: number) {
    const followed = this.followedByUser.get(follower);
    if (!followed?.delete(unfollowedUser)) return;
    if (followed.size === 0) this.followedByUser.delete(follower);
    if (this.recalculatingPromise) this.recalculationRequested = true;

    // Maintain reverse index
    removeCompactEdge(this.followersByUser, unfollowedUser, follower);

    // Only the loss of a shortest path can change distances.
    const distance = this.followDistanceByUser.get(unfollowedUser);
    if (distance && this.followDistanceByUser.get(follower) === distance - 1) {
      this.raiseDistance(unfollowedUser);
    }
  }

  followerCount(address: string) {
    const id = this.known(address);
    if (id === undefined) return 0;
    const followers = this.followersByUser.get(id);
    if (followers === undefined) return 0;
    return typeof followers === 'number' ? 1 : followers.size;
  }

  followedByFriendsCount(address: string) {
    const id = this.known(address);
    const followers = id === undefined ? undefined : this.followersByUser.get(id);
    const rootFollows = this.followedByUser.get(this.root);
    if (followers === undefined || !rootFollows) return 0;
    if (typeof followers === 'number') return rootFollows.has(followers) ? 1 : 0;
    let count = 0;
    for (const follower of followers) {
      if (rootFollows.has(follower)) count++;
    }
    return count;
  }

  mutedByFriendsCount(address: string) {
    const id = this.known(address);
    const muters = id === undefined ? undefined : this.userMutedBy.get(id);
    const rootFollows = this.followedByUser.get(this.root);
    if (muters === undefined || !rootFollows) return 0;
    if (typeof muters === 'number') return rootFollows.has(muters) ? 1 : 0;
    let count = 0;
    for (const muter of muters) {
      if (rootFollows.has(muter)) count++;
    }
    return count;
  }

  size() {
    let follows = 0;
    let mutes = 0;
    const sizeByDistance: { [distance: number]: number } = {};

    for (const followedSet of this.followedByUser.values()) follows += followedSet.size;
    for (const mutedSet of this.mutedByUser.values()) mutes += mutedSet.size;
    for (const [distance, users] of this.usersByFollowDistance.entries()) {
      sizeByDistance[distance] = users.size;
    }

    // If follow distances haven't been calculated (e.g. when we deliberately
    // skip them for memory reasons), fall back to counting the unique IDs we
    // know about.
    const usersCount = this.followDistanceByUser.size || this.ids.size;

    return {
      users: usersCount,
      follows,
      mutes,
      sizeByDistance,
    };
  }

  followedByFriends(address: string) {
    const set = new Set<string>();
    const id = this.known(address);
    const followers = id === undefined ? undefined : this.followersByUser.get(id);
    const rootFollows = this.followedByUser.get(this.root);
    if (followers !== undefined && rootFollows) {
      if (typeof followers === 'number') {
        if (rootFollows.has(followers)) set.add(this.str(followers));
      } else {
        for (const follower of followers) {
          if (rootFollows.has(follower)) set.add(this.str(follower));
        }
      }
    }
    return set;
  }

  getFollowedByUser(user: string, includeSelf = false): Set<string> {
    const set = this.strings(this.followedByUser, user);
    if (includeSelf) set.add(user);
    return set;
  }

  getFollowersByUser(address: string): Set<string> {
    return this.stringsCompact(this.followersByUser, address);
  }

  /** Public keys related to `user` in one of the forward edge maps. */
  private strings(edges: Map<number, Set<number>>, user: string): Set<string> {
    const set = new Set<string>();
    const id = this.known(user);
    const related = id === undefined ? undefined : edges.get(id);
    if (related) {
      for (const other of related) set.add(this.str(other));
    }
    return set;
  }

  /** Public keys related to `user` in a compact reverse edge map. */
  private stringsCompact(edges: Map<number, CompactEdgeEntry>, user: string): Set<string> {
    const set = new Set<string>();
    const id = this.known(user);
    const related = id === undefined ? undefined : edges.get(id);
    if (related !== undefined) {
      if (typeof related === 'number') set.add(this.str(related));
      else for (const other of related) set.add(this.str(other));
    }
    return set;
  }

  getUsersByFollowDistance(distance: number): Set<string> {
    const result = new Set<string>();
    const users = this.usersByFollowDistance.get(distance);
    if (users) {
      for (const user of users) result.add(this.str(user));
    }
    return result;
  }

  getFollowListCreatedAt(user: string) {
    const id = this.known(user);
    return id === undefined ? undefined : this.followListCreatedAt.get(id);
  }

  getMuteListCreatedAt(user: string) {
    const id = this.known(user);
    return id === undefined ? undefined : this.muteListCreatedAt.get(id);
  }

  async merge(other: SocialGraph): Promise<void> {
    // Lists belong to the source graph's ID space. Timestamp-only empty lists
    // and disconnected authors must survive a merge too.
    const users = new Set([
      ...other.followListCreatedAt.keys(), ...other.followedByUser.keys(),
      ...other.muteListCreatedAt.keys(), ...other.mutedByUser.keys(),
    ]);
    let processed = 0;
    for (const user of users) {
      this.mergeUserLists(user, other, this.followListCreatedAt, other.followListCreatedAt,
        this.followedByUser, other.followedByUser, this.followersByUser, this.followersMaterialized);
      this.mergeUserLists(user, other, this.muteListCreatedAt, other.muteListCreatedAt,
        this.mutedByUser, other.mutedByUser, this.userMutedBy, this.userMutedByMaterialized);
      // Yield in bounded batches rather than imposing one timer per identity.
      if (++processed % 256 === 0) await new Promise(resolve => setTimeout(resolve, 0));
    }
    await this.recalculateFollowDistances();
  }

  private mergeUserLists(
    theirUserId: number,
    other: SocialGraph,
    ourCreatedAtMap: Map<number, number>,
    theirCreatedAtMap: Map<number, number>,
    ourUserMap: Map<number, Set<number>>,
    theirUserMap: Map<number, Set<number>>,
    ourReverseMap: Map<number, CompactEdgeEntry>,
    reverseMaterialized: boolean
  ) {
    const theirCreatedAt = theirCreatedAtMap.get(theirUserId);
    const theirUsers = theirUserMap.get(theirUserId);
    // No source list is different from an authoritative empty list.
    if (theirCreatedAt === undefined && theirUsers === undefined) return;

    const ownerKey = other.str(theirUserId);
    const userId = this.id(ownerKey);
    const ourCreatedAt = ourCreatedAtMap.get(userId);
    const kind = ourCreatedAtMap === this.followListCreatedAt ? 3 : 10000;
    const key = `${kind}:${ownerKey}`;
    const incoming = other.listRevisions.get(key);
    const current = this.listRevisions.get(key);
    // A projection cannot erase a client's more complete crawl.
    if (incoming?.complete === false && (ourCreatedAt !== undefined || ourUserMap.has(userId))) return;
    if (ourCreatedAt !== undefined && (theirCreatedAt === undefined || theirCreatedAt < ourCreatedAt ||
      (theirCreatedAt === ourCreatedAt && (!incoming || (current &&
        (incoming.id > current.id || (incoming.id === current.id && (current.complete || !incoming.complete)))))))) return;
    if (incoming) this.listRevisions.set(key, {...incoming});
    else this.listRevisions.delete(key);

    // Numeric IDs are private to each graph, including IDs inside each list.
    const newUsers = new Set<number>();
    if (theirUsers) {
      for (const theirTarget of theirUsers) {
        newUsers.add(this.id(other.str(theirTarget)));
      }
    }
    const oldUsers = ourUserMap.get(userId);
    if (oldUsers) {
      for (const currentUser of oldUsers) {
        if (!newUsers.has(currentUser)) removeCompactEdge(ourReverseMap, currentUser, userId);
      }
    }
    for (const newUser of newUsers) {
      addCompactEdge(ourReverseMap, newUser, userId, reverseMaterialized);
    }
    if (newUsers.size) ourUserMap.set(userId, newUsers);
    else ourUserMap.delete(userId);
    if (theirCreatedAt !== undefined) ourCreatedAtMap.set(userId, theirCreatedAt);
  }

  *userIterator(upToDistance?: number): Generator<string> {
    const distances = Array.from(this.usersByFollowDistance.keys()).sort((a, b) => a - b);
    for (const distance of distances) {
      if (upToDistance !== undefined && distance > upToDistance) break;
      const users = this.usersByFollowDistance.get(distance);
      if (!users) continue;
      for (const user of users) yield this.str(user);
    }
  }

  [Symbol.iterator](): Generator<string> {
    return this.userIterator();
  }

  getMutedByUser(user: string): Set<string> {
    return this.strings(this.mutedByUser, user);
  }

  getUserMutedBy(user: string): Set<string> {
    return this.stringsCompact(this.userMutedBy, user);
  }

  // follower and muter counts by distance
  stats(user: string): { [distance: number]: { followers: number; muters: number } } {
    return SocialGraphUtils.stats(this, user);
  }

  /**
   * Remove users who are muted by someone AND have zero followers.
   * O(M) to scan muted users + O(incident edges) to remove them.
   * Async and non-blocking with batching.
   */
  removeMutedNotFollowedUsers(
    batchSize = 10_000,
    logger: (phase: string, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    return SocialGraphUtils.removeMutedNotFollowedUsers(this, batchSize, logger);
  }

  toBinaryChunks(
    maxNodes?: number,
    maxEdges?: number,
    maxDistance?: number,
    maxEdgesPerNode?: number,
    version?: 3 | 4 | 5
  ): AsyncGenerator<Uint8Array> {
    return Binary.toBinaryChunks(this, maxNodes, maxEdges, maxDistance, maxEdgesPerNode, version);
  }

  toBinary(
    maxNodes?: number,
    maxEdges?: number,
    maxDistance?: number,
    maxEdgesPerNode?: number,
    version?: 3 | 4 | 5
  ): Promise<Uint8Array> {
    return Binary.toBinary(this, maxNodes, maxEdges, maxDistance, maxEdgesPerNode, version);
  }

  static fromBinary(root: string, data: Uint8Array): Promise<SocialGraph> {
    return Binary.fromBinary(root, data);
  }

  static fromBinaryStream(root: string, stream: ReadableStream<Uint8Array>): Promise<SocialGraph> {
    return Binary.fromBinaryStream(root, stream);
  }
}
