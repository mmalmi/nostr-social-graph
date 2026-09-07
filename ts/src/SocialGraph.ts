import { UniqueIds } from './UniqueIds';
import { isValidPubKey, NostrEvent } from './utils';
import * as Binary from './SocialGraphBinary';
import { SocialGraphUtils } from './SocialGraphUtils';

export class SocialGraph {
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

  private str(id: number): string {
    return this.ids.str(id);
  }

  getRoot() {
    return this.str(this.root)
  }

  getInternalData() {
    return {
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
    logger: (msg: string) => void = console.log
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
        const author = this.id(event.pubkey);

        if (!allowUnknownAuthors && !this.followDistanceByUser.has(author)) {
          continue;
        }

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
    if (existingCreatedAt && createdAt <= existingCreatedAt) {
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
    if (existingCreatedAt && createdAt <= existingCreatedAt) {
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
    const followedUserId = this.id(followedUser);
    const followerId = this.id(follower);
    return !!this.followedByUser.get(followerId)?.has(followedUserId);
  }

  getFollowDistance(user: string): number {
    const distance = this.followDistanceByUser.get(this.id(user));
    return distance === undefined ? 1000 : distance;
  }

  private addUserByFollowDistance(distance: number, user: number) {
    if (!this.usersByFollowDistance.has(distance)) {
      this.usersByFollowDistance.set(distance, new Set());
    }
    this.usersByFollowDistance.get(distance)?.add(user);
    for (const d of this.usersByFollowDistance.keys()) {
      if (d > distance) {
        this.usersByFollowDistance.get(d)?.delete(user);
      }
    }
  }

  private privateAddFollower(followedUser: number, follower: number) {
    if (typeof followedUser !== 'number' || typeof follower !== 'number') {
      throw new Error('Invalid user id');
    }

    if (!this.followedByUser.has(follower)) {
      this.followedByUser.set(follower, new Set<number>());
    }
    const followed = this.followedByUser.get(follower)!;
    if (!followed.has(followedUser)) {
      followed.add(followedUser);
      if (this.recalculatingPromise) this.recalculationRequested = true;
    }

    // Maintain reverse index
    if (!this.followersByUser.has(followedUser)) {
      this.followersByUser.set(followedUser, new Set<number>());
    }
    this.followersByUser.get(followedUser)!.add(follower);

    if (followedUser !== this.root) {
      let newFollowDistance;
      if (follower === this.root) {
        newFollowDistance = 1;
        this.addUserByFollowDistance(newFollowDistance, followedUser);
        this.followDistanceByUser.set(followedUser, newFollowDistance);
      } else {
        const existingFollowDistance = this.followDistanceByUser.get(followedUser);
        const followerDistance = this.followDistanceByUser.get(follower);
        newFollowDistance = followerDistance && followerDistance + 1;
        if (
          existingFollowDistance === undefined ||
          (newFollowDistance && newFollowDistance < existingFollowDistance)
        ) {
          this.followDistanceByUser.set(followedUser, newFollowDistance!);
          this.addUserByFollowDistance(newFollowDistance!, followedUser);
        }
      }
    }
  }

  addFollower(follower: string, followedUser: string) {
    this.privateAddFollower(this.id(followedUser), this.id(follower))
  }

  removeFollower(follower: string, followedUser: string) {
    this.privateRemoveFollower(this.id(followedUser), this.id(follower))
  }

  private privateRemoveFollower(unfollowedUser: number, follower: number) {
    if (!this.followedByUser.get(follower)?.delete(unfollowedUser)) return;
    if (this.recalculatingPromise) this.recalculationRequested = true;

    // Maintain reverse index
    this.followersByUser.get(unfollowedUser)?.delete(follower);

    if (unfollowedUser === this.root) {
      return;
    }

    let smallest = Infinity;
    for (const follower of this.getFollowersSet(unfollowedUser)) {
      const followerDistance = this.followDistanceByUser.get(follower);
      if (followerDistance !== undefined && followerDistance + 1 < smallest) {
        smallest = followerDistance + 1;
      }
    }

    if (smallest === Infinity) {
      this.followDistanceByUser.delete(unfollowedUser);
    } else {
      this.followDistanceByUser.set(unfollowedUser, smallest);
    }
  }

  followerCount(address: string) {
    const id = this.id(address);
    return this.getFollowersSet(id).size;
  }

  followedByFriendsCount(address: string) {
    let count = 0;
    const id = this.id(address);
    for (const follower of this.getFollowersSet(id)) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        count++;
      }
    }
    return count;
  }

  mutedByFriendsCount(address: string) {
    let count = 0;
    const id = this.id(address);
    for (const muter of this.userMutedBy.get(id) ?? []) {
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
    const usersCount = this.followDistanceByUser.size || (this.ids as any).uniqueIdToStr?.size || 0;

    return {
      users: usersCount,
      follows,
      mutes,
      sizeByDistance,
    };
  }

  followedByFriends(address: string) {
    const id = this.id(address);
    const set = new Set<string>();
    for (const follower of this.getFollowersSet(id)) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        set.add(this.str(follower));
      }
    }
    return set;
  }

  getFollowedByUser(user: string, includeSelf = false): Set<string> {
    const userId = this.id(user);
    const set = new Set<string>();
    for (const id of this.followedByUser.get(userId) || []) {
      set.add(this.str(id));
    }
    if (includeSelf) {
      set.add(user);
    }
    return set;
  }

  getFollowersByUser(address: string): Set<string> {
    const userId = this.id(address);
    const set = new Set<string>();
    for (const id of this.getFollowersSet(userId)) {
      set.add(this.str(id));
    }
    return set;
  }





  getUsersByFollowDistance(distance: number): Set<string> {
    const users = this.usersByFollowDistance.get(distance) || new Set<number>();
    const result = new Set<string>();
    for (const user of users) {
      result.add(this.str(user));
    }
    return result;
  }

  getFollowListCreatedAt(user: string) {
    return this.followListCreatedAt.get(this.id(user))
  }

  getMuteListCreatedAt(user: string) {
    return this.muteListCreatedAt.get(this.id(user))
  }

  merge(other: SocialGraph): Promise<void> {
    return new Promise((resolve) => {
      console.log('size before merge', this.size());
      console.time('merge graph');
      
      const users = Array.from(other);
      let processedCount = 0;

      const processNextUser = () => {
        if (processedCount >= users.length) {
          // All users processed, now recalculate distances
          this.recalculateFollowDistances().then(() => {
            console.timeEnd('merge graph');
            console.log('size after merge', this.size());
            resolve();
          });
          return;
        }

        const user = users[processedCount];
        
        this.mergeUserLists(
          user,
          this.followListCreatedAt,
          other.followListCreatedAt,
          this.followedByUser,
          other.followedByUser
        );

        this.mergeUserLists(
          user,
          this.muteListCreatedAt,
          other.muteListCreatedAt,
          this.mutedByUser,
          other.mutedByUser
        );
        
        processedCount++;

        // Schedule next user processing
        setTimeout(processNextUser, 0);
      };

      // Start processing
      setTimeout(processNextUser, 0);
    });
  }




  private mergeUserLists(
    user: string,
    ourCreatedAtMap: Map<number, number>,
    theirCreatedAtMap: Map<number, number>,
    ourUserMap: Map<number, Set<number>>, 
    theirUserMap: Map<number, Set<number>>
  ) {
    const userId = this.id(user);
    const ourCreatedAt = ourCreatedAtMap.get(userId);
    const theirCreatedAt = theirCreatedAtMap.get(userId);

    if (!ourCreatedAt || (theirCreatedAt && ourCreatedAt < theirCreatedAt)) {
      const newUsers = theirUserMap.get(userId) || new Set<number>();
      const currentUsers = ourUserMap.get(userId) || new Set<number>();

      for (const newUser of newUsers) {
        if (!currentUsers.has(newUser)) {
          if (!ourUserMap.has(userId)) {
            ourUserMap.set(userId, new Set<number>());
          }
          ourUserMap.get(userId)!.add(newUser);
        }
      }

      for (const currentUser of currentUsers) {
        if (!newUsers.has(currentUser)) {
          ourUserMap.get(userId)!.delete(currentUser);
        }
      }

      ourCreatedAtMap.set(userId, theirCreatedAt ?? 0);
    }
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
    const userId = this.id(user);
    const set = new Set<string>();
    for (const id of this.mutedByUser.get(userId) || []) {
      set.add(this.str(id));
    }
    return set;
  }

  getUserMutedBy(user: string): Set<string> {
    const userId = this.id(user);
    const set = new Set<string>();
    for (const id of this.userMutedBy.get(userId) || []) {
      set.add(this.str(id));
    }
    return set;
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

  private getFollowersSet(id: number): Set<number> {
    return SocialGraphUtils.getFollowersSet(this, id);
  }
}
