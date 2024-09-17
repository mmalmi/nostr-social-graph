import { SerializedUniqueIds, UniqueIds } from './UniqueIds';
import { pubKeyRegex, NostrEvent } from './utils';

type SerializedSocialGraph = {
  follows: [number, number[]][];
  uniqueIds: SerializedUniqueIds;
};

export class SocialGraph {
  private root: number;
  private followDistanceByUser = new Map<number, number>();
  private usersByFollowDistance = new Map<number, Set<number>>();
  private followedByUser = new Map<number, Set<number>>();
  private followersByUser = new Map<number, Set<number>>();
  private latestFollowEventTimestamps = new Map<number, number>();
  private ids = new UniqueIds();

  constructor(root: string, serialized?: SerializedSocialGraph) {
    this.root = this.id(root);
    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));
    if (serialized) {
      this.ids = new UniqueIds(serialized.uniqueIds);
      this.deserialize(serialized.follows);
    }
  }

  private id(str: string): number {
    return this.ids.id(str);
  }

  private str(id: number): string {
    return this.ids.str(id);
  }

  setRoot(root: string) {
    const rootId = this.id(root);
    if (rootId === this.root) {
      return;
    }
    this.root = rootId;
    this.followDistanceByUser.clear();
    this.usersByFollowDistance.clear();
    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));

    const queue = [this.root];

    while (queue.length > 0) {
      const user = queue.shift()!;
      const distance = this.followDistanceByUser.get(user)!;

      const followedUsers = this.followedByUser.get(user) || new Set<number>();
      for (const followed of followedUsers) {
        if (!this.followDistanceByUser.has(followed)) {
          const newFollowDistance = distance + 1;
          this.followDistanceByUser.set(followed, newFollowDistance);
          if (!this.usersByFollowDistance.has(newFollowDistance)) {
            this.usersByFollowDistance.set(newFollowDistance, new Set());
          }
          this.usersByFollowDistance.get(newFollowDistance)!.add(followed);
          queue.push(followed);
        }
      }
    }
  }

  handleEvent(evs: NostrEvent | Array<NostrEvent>) {
    const filtered = (Array.isArray(evs) ? evs : [evs]).filter((a) => a.kind === 3);
    if (filtered.length === 0) {
      return;
    }
    for (const event of filtered) {
      const author = this.id(event.pubkey);
      const timestamp = event.created_at;
      const existingTimestamp = this.latestFollowEventTimestamps.get(author);
      if (existingTimestamp && timestamp <= existingTimestamp) {
        return;
      }
      this.latestFollowEventTimestamps.set(author, timestamp);

      const followedInEvent = new Set<number>();
      for (const tag of event.tags) {
        if (tag[0] === 'p') {
          if (!pubKeyRegex.test(tag[1])) {
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
          this.removeFollower(user, author);
        }
      }

      for (const user of followedInEvent) {
        this.addFollower(user, author);
      }
    }
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

  private addFollower(followedUser: number, follower: number) {
    if (typeof followedUser !== 'number' || typeof follower !== 'number') {
      throw new Error('Invalid user id');
    }
    if (!this.followersByUser.has(followedUser)) {
      this.followersByUser.set(followedUser, new Set<number>());
    }
    this.followersByUser.get(followedUser)?.add(follower);

    if (!this.followedByUser.has(follower)) {
      this.followedByUser.set(follower, new Set<number>());
    }

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

    this.followedByUser.get(follower)?.add(followedUser);
  }

  private removeFollower(unfollowedUser: number, follower: number) {
    this.followersByUser.get(unfollowedUser)?.delete(follower);
    this.followedByUser.get(follower)?.delete(unfollowedUser);

    if (unfollowedUser === this.root) {
      return;
    }

    let smallest = Infinity;
    for (const follower of this.followersByUser.get(unfollowedUser) || []) {
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
    return this.followersByUser.get(id)?.size ?? 0;
  }

  followedByFriendsCount(address: string) {
    let count = 0;
    const id = this.id(address);
    for (const follower of this.followersByUser.get(id) ?? []) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        count++;
      }
    }
    return count;
  }

  size() {
    let follows = 0;
    const sizeByDistance: { [distance: number]: number } = {};

    for (const followedSet of this.followedByUser.values()) {
      follows += followedSet.size;
    }

    for (const [distance, users] of this.usersByFollowDistance.entries()) {
      sizeByDistance[distance] = users.size;
    }

    return {
      users: this.followDistanceByUser.size,
      follows,
      sizeByDistance,
    };
  }

  followedByFriends(address: string) {
    const id = this.id(address);
    const set = new Set<string>();
    for (const follower of this.followersByUser.get(id) ?? []) {
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
    for (const id of this.followersByUser.get(userId) || []) {
      set.add(this.str(id));
    }
    return set;
  }

  serialize(maxSize?: number): SerializedSocialGraph {
    const follows: [number, number[]][] = [];
    for (let distance = 0; distance <= Math.max(...this.usersByFollowDistance.keys()); distance++) {
      const users = this.usersByFollowDistance.get(distance) || new Set<number>();
      for (const user of users) {
        const followedUsers = this.followedByUser.get(user)
        if (!followedUsers || followedUsers.size === 0) {
          continue
        }
        follows.push([user, [...followedUsers.values()]]);
        if (maxSize && follows.length >= maxSize) {
          return { follows, uniqueIds: this.ids.serialize() };
        }
      }
    }
    return { follows, uniqueIds: this.ids.serialize() };
  }

  private deserialize(follows: [number, number[]][]): void {
    for (const [follower, followedUsers] of follows) {
      for (const followedUser of followedUsers) {
        this.addFollower(followedUser, follower);
      }
    }
  }

  getUsersByFollowDistance(distance: number): Set<string> {
    const users = this.usersByFollowDistance.get(distance) || new Set<number>();
    const result = new Set<string>();
    for (const user of users) {
      result.add(this.str(user));
    }
    return result;
  }
}
