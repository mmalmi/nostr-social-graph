import { UniqueIds, canonicalNodeId } from './UniqueIds';
import { isValidPubKey, NostrEvent } from './utils';
import * as Binary from './SocialGraphBinary';
import { SocialGraphUtils } from './SocialGraphUtils';

export interface ListRevision { id: string; created_at: number; complete: boolean }
export interface GraphListRevision extends ListRevision { pubkey: string; kind: 3 | 10000 }

export class SocialGraph {
  private listRevisions = new Map<string, ListRevision>();

  /** Metadata is not a signature proof. Callers must verify events before ingestion. */
  getListRevision(pubkey: string, kind: number): ListRevision | undefined {
    const revision = this.listRevisions.get(`${kind}:${canonicalNodeId(pubkey)}`);
    return revision && {...revision};
  }

  getListRevisions(): GraphListRevision[] {
    return [...this.listRevisions].map(([key, revision]) => {
      const split = key.indexOf(':');
      return {pubkey: key.slice(split + 1), kind: Number(key.slice(0, split)) as 3 | 10000, ...revision};
    });
  }

  private acceptRevision(event: NostrEvent, previousAt: number | undefined): boolean {
    const key = `${event.kind}:${canonicalNodeId(event.pubkey)}`;
    const previous = this.listRevisions.get(key);
    const validId = /^[0-9a-f]{64}$/.test(event.id);
    if (previousAt !== undefined && (event.created_at < previousAt ||
      (event.created_at === previousAt && (!validId || (previous &&
        (event.id > previous.id || (event.id === previous.id && previous.complete))))))) return false;
    if (validId) this.listRevisions.set(key, {id:event.id, created_at:event.created_at, complete:true});
    else this.listRevisions.delete(key);
    return true;
  }

  markListIncomplete(pubkey: string, kind: number) {
    const revision = this.listRevisions.get(`${kind}:${canonicalNodeId(pubkey)}`);
    if (revision) revision.complete = false;
  }

  private root: number;
  private recalculatingPromise = null as Promise<void> | null;
  private followDistanceByUser = new Map<number, number>();
  private usersByFollowDistance = new Map<number, Set<number>>();
  private followedByUser = new Map<number, Set<number>>();
  private followersByUser = new Map<number, Set<number>>();
  private followListCreatedAt = new Map<number, number>();
  private mutedByUser = new Map<number, Set<number>>();
  private userMutedBy = new Map<number, Set<number>>();
  private muteListCreatedAt = new Map<number, number>()
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
    return this.str(this.root)
  }

  getInternalData() {
    return {
      listRevisions: this.listRevisions,
      followedByUser: this.followedByUser,
      followersByUser: this.followersByUser,
      mutedByUser: this.mutedByUser,
      userMutedBy: this.userMutedBy,
      followListCreatedAt: this.followListCreatedAt,
      muteListCreatedAt: this.muteListCreatedAt,
      ids: this.ids,
      str: (id: number) => this.str(id)
    };
  }

  setRoot(root: string): Promise<void> {
    const rootId = this.id(root);
    if (rootId === this.root) {
      return this.recalculatingPromise ?? Promise.resolve();
    }

    this.root = rootId;
    // Do not expose distances from the previous root while computing the new one.
    this.followDistanceByUser = new Map([[rootId, 0]]);
    this.usersByFollowDistance = new Map([[0, new Set([rootId])]]);
    return this.recalculateFollowDistances();
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
          const distances = new Map([[root, 0]]);
          const usersByDistance = new Map([[0, new Set([root])]]);
          const queue = [root];
          let head = 0;
          const start = performance.now();
          logger(`recalculateFollowDistances: start (batchSize=${batchSize})`);

          while (head < queue.length) {
            const end = head + batchSize;
            // Newly discovered users count toward this batch too. A deep graph
            // should not need a separate timer for every degree of separation.
            for (; head < end && head < queue.length; head++) {
              const user = queue[head];
              const distance = distances.get(user)! + 1;
              for (const followed of this.followedByUser.get(user) ?? []) {
                if (distances.has(followed)) continue;
                distances.set(followed, distance);
                let bucket = usersByDistance.get(distance);
                if (!bucket) usersByDistance.set(distance, bucket = new Set());
                bucket.add(followed);
                queue.push(followed);
              }
            }
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
    const filtered = (Array.isArray(evs) ? evs : [evs]).filter((a) => [3, 10000].includes(a.kind));
    let changed = false;

    for (const event of filtered) {
        const createdAt = event.created_at;
        if (createdAt > Math.floor(Date.now() / 1000) + 10 * 60) {
            console.debug("event.created_at more than 10 minutes in the future", event)
            continue
        }
        if (!allowUnknownAuthors) {
          const known = this.known(event.pubkey);
          if (known === undefined || !this.followDistanceByUser.has(known)) continue;
        }
        const author = this.id(event.pubkey);

        if (SocialGraphUtils.isOvermuted(this, event.pubkey, overmuteThreshold)) {
            continue;
        }

        if (event.kind === 3) {
            changed = this.handleFollowList(event, author, createdAt) || changed;
        } else if (event.kind === 10000) {
            changed = this.handleMuteList(event, author, createdAt) || changed;
        }
    }

    return changed;
  }

  private handleFollowList(event: NostrEvent, author: number, createdAt: number): boolean {
    const existingCreatedAt = this.followListCreatedAt.get(author);
    if (!this.acceptRevision(event, existingCreatedAt)) {
        return false;
    }
    this.followListCreatedAt.set(author, createdAt);

    const followedInEvent = new Set<number>();
    for (const tag of event.tags) {
        if (tag[0] === 'p') {
            if (!isValidPubKey(tag[1])) {
                continue;
            }
            const followedUser = this.id(tag[1]);
            if (followedUser !== author) {
                followedInEvent.add(followedUser);
            }
        }
    }

    const currentlyFollowed = this.followedByUser.get(author) || new Set<number>();

    for (const user of currentlyFollowed) {
        if (!followedInEvent.has(user)) {
            this.privateRemoveFollower(user, author);
        }
    }

    for (const user of followedInEvent) {
        this.privateAddFollower(user, author);
    }

    return true;
  }

  private handleMuteList(event: NostrEvent, author: number, createdAt: number): boolean {
    const existingCreatedAt = this.muteListCreatedAt.get(author);
    if (!this.acceptRevision(event, existingCreatedAt)) {
        return false;
    }
    this.muteListCreatedAt.set(author, createdAt);

    const mutedInEvent = new Set<number>();
    for (const tag of event.tags) {
        if (tag[0] === 'p') {
            if (!isValidPubKey(tag[1])) {
                continue;
            }
            const mutedUser = this.id(tag[1]);
            if (mutedUser !== author) {
                mutedInEvent.add(mutedUser);
            }
        }
    }

    const currentlyMuted = this.mutedByUser.get(author) || new Set<number>();

    for (const user of currentlyMuted) {
        if (!mutedInEvent.has(user)) {
            this.mutedByUser.get(author)?.delete(user);
            this.userMutedBy.get(user)?.delete(author);
        }
    }

    for (const user of mutedInEvent) {
        if (!this.mutedByUser.has(author)) {
            this.mutedByUser.set(author, new Set<number>());
        }
        this.mutedByUser.get(author)?.add(user);

        if (!this.userMutedBy.has(user)) {
            this.userMutedBy.set(user, new Set<number>());
        }
        this.userMutedBy.get(user)?.add(author);
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
      const next = this.followDistanceByUser.get(queue[head])! + 1;
      for (const followed of this.followedByUser.get(queue[head]) ?? []) {
        const known = this.followDistanceByUser.get(followed);
        if (known === undefined || known > next) {
          this.setDistance(followed, next);
          queue.push(followed);
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
      const closer = distances.get(id)! - 1;
      for (const follower of this.followersByUser.get(id) ?? []) {
        if (distances.get(follower) === closer && !orphaned.has(follower)) return true;
      }
      return false;
    };
    if (hasCloserFollower(user)) return;
    orphaned.add(user);
    // Sets iterate in insertion order, including additions: closest users first.
    for (const id of orphaned) {
      const next = distances.get(id)! + 1;
      for (const followed of this.followedByUser.get(id) ?? []) {
        if (distances.get(followed) === next && !orphaned.has(followed) && !hasCloserFollower(followed)) {
          orphaned.add(followed);
        }
      }
    }
    for (const id of orphaned) this.setDistance(id, undefined);
    // Reattach through the nearest follower that is still reachable.
    for (const id of orphaned) {
      let nearest = Infinity;
      for (const follower of this.followersByUser.get(id) ?? []) {
        const distance = distances.get(follower);
        if (distance !== undefined && distance < nearest) nearest = distance;
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
    let followers = this.followersByUser.get(followedUser);
    if (!followers) this.followersByUser.set(followedUser, followers = new Set());
    followers.add(follower);

    // A new edge can only shorten paths, and only from a reachable follower.
    const followerDistance = this.followDistanceByUser.get(follower);
    if (followerDistance !== undefined) this.lowerDistance(followedUser, followerDistance + 1);
  }

  addFollower(follower: string, followedUser: string) {
    this.markListIncomplete(follower, 3);
    this.privateAddFollower(this.id(followedUser), this.id(follower))
  }

  removeFollower(follower: string, followedUser: string) {
    this.markListIncomplete(follower, 3);
    this.privateRemoveFollower(this.id(followedUser), this.id(follower))
  }

  private privateRemoveFollower(unfollowedUser: number, follower: number) {
    if (!this.followedByUser.get(follower)?.delete(unfollowedUser)) return;
    if (this.recalculatingPromise) this.recalculationRequested = true;

    // Maintain reverse index
    this.followersByUser.get(unfollowedUser)?.delete(follower);

    // Only the loss of a shortest path can change distances.
    const distance = this.followDistanceByUser.get(unfollowedUser);
    if (distance && this.followDistanceByUser.get(follower) === distance - 1) {
      this.raiseDistance(unfollowedUser);
    }
  }

  followerCount(address: string) {
    return this.getFollowersSet(this.known(address)).size;
  }

  followedByFriendsCount(address: string) {
    let count = 0;
    const id = this.known(address);
    for (const follower of this.getFollowersSet(id)) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        count++;
      }
    }
    return count;
  }

  mutedByFriendsCount(address: string) {
    let count = 0;
    const id = this.known(address);
    for (const muter of (id === undefined ? undefined : this.userMutedBy.get(id)) ?? []) {
      if (this.followedByUser.get(this.root)?.has(muter)) {
        count++;
      }
    }
    return count;
  }

  size() {
    let follows = 0;
    let mutes = 0;
    const sizeByDistance: { [distance: number]: number } = {};

    for (const followedSet of this.followedByUser.values()) {
      follows += followedSet.size;
    }

    for (const mutedSet of this.mutedByUser.values()) {
      mutes += mutedSet.size;
    }

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
    const id = this.known(address);
    const set = new Set<string>();
    for (const follower of this.getFollowersSet(id)) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        set.add(this.str(follower));
      }
    }
    return set;
  }

  getFollowedByUser(user: string, includeSelf = false): Set<string> {
    const set = this.strings(this.followedByUser, user);
    if (includeSelf) {
      set.add(user);
    }
    return set;
  }

  getFollowersByUser(address: string): Set<string> {
    return this.strings(this.followersByUser, address);
  }

  /** Public keys related to `user` in one of the edge maps. */
  private strings(edges: Map<number, Set<number>>, user: string): Set<string> {
    const set = new Set<string>();
    const id = this.known(user);
    for (const other of (id === undefined ? undefined : edges.get(id)) ?? []) {
      set.add(this.str(other));
    }
    return set;
  }

  getUsersByFollowDistance(distance: number): Set<string> {
    const result = new Set<string>();
    for (const user of this.usersByFollowDistance.get(distance) ?? []) {
      result.add(this.str(user));
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
        this.followedByUser, other.followedByUser, this.followersByUser);
      this.mergeUserLists(user, other, this.muteListCreatedAt, other.muteListCreatedAt,
        this.mutedByUser, other.mutedByUser, this.userMutedBy);
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
    ourReverseMap: Map<number, Set<number>>
  ) {
    const theirCreatedAt = theirCreatedAtMap.get(theirUserId);
    const theirUsers = theirUserMap.get(theirUserId);
    // No source list is different from an authoritative empty list.
    if (theirCreatedAt === undefined && theirUsers === undefined) return;

    const userId = this.id(other.str(theirUserId));
    const ourCreatedAt = ourCreatedAtMap.get(userId);
    const kind = ourCreatedAtMap === this.followListCreatedAt ? 3 : 10000;
    const key = `${kind}:${other.str(theirUserId)}`;
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
    for (const theirTarget of theirUsers ?? []) {
      newUsers.add(this.id(other.str(theirTarget)));
    }
    for (const currentUser of ourUserMap.get(userId) ?? []) {
      if (!newUsers.has(currentUser)) ourReverseMap.get(currentUser)?.delete(userId);
    }
    for (const newUser of newUsers) {
      let reverse = ourReverseMap.get(newUser);
      if (!reverse) ourReverseMap.set(newUser, reverse = new Set());
      reverse.add(userId);
    }
    ourUserMap.set(userId, newUsers);
    if (theirCreatedAt !== undefined) ourCreatedAtMap.set(userId, theirCreatedAt);
  }

  *userIterator(upToDistance?: number): Generator<string> {
    const distances = Array.from(this.usersByFollowDistance.keys()).sort((a, b) => a - b);
    for (const distance of distances) {
      if (upToDistance !== undefined && distance > upToDistance) {
        break;
      }
      const users = this.usersByFollowDistance.get(distance) || new Set<number>();
      for (const user of users) {
        yield this.str(user);
      }
    }
  }

  [Symbol.iterator](): Generator<string> {
    return this.userIterator();
  }

  getMutedByUser(user: string): Set<string> {
    return this.strings(this.mutedByUser, user);
  }

  getUserMutedBy(user: string): Set<string> {
    return this.strings(this.userMutedBy, user);
  }

  // follower and muter counts by distance
  stats(user: string): { [distance: number]: { followers: number; muters: number } } {
    return SocialGraphUtils.stats(this, user);
  }

  /**
   * Remove users who are muted by someone AND have zero followers.
   * O(E + M) where E = follows edges, M = mutes edges.
   * Now async and non-blocking with batching.
   */
  removeMutedNotFollowedUsers(
    batchSize = 10_000,
    logger: (phase: string, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    return SocialGraphUtils.removeMutedNotFollowedUsers(this, batchSize, logger);
  }

  toBinaryChunks(maxNodes?: number, maxEdges?: number, maxDistance?: number, maxEdgesPerNode?: number): AsyncGenerator<Uint8Array> {
    return Binary.toBinaryChunks(this, maxNodes, maxEdges, maxDistance, maxEdgesPerNode);
  }

  toBinary(maxNodes?: number, maxEdges?: number, maxDistance?: number, maxEdgesPerNode?: number): Promise<Uint8Array> {
    return Binary.toBinary(this, maxNodes, maxEdges, maxDistance, maxEdgesPerNode);
  }

  static fromBinary(root: string, data: Uint8Array): Promise<SocialGraph> {
    return Binary.fromBinary(root, data);
  }

  static fromBinaryStream(root: string, stream: ReadableStream<Uint8Array>): Promise<SocialGraph> {
    return Binary.fromBinaryStream(root, stream);
  }

  private getFollowersSet(id: number | undefined): Set<number> {
    return (id === undefined ? undefined : this.followersByUser.get(id)) ?? new Set();
  }
}
