import { SerializedUniqueIds, UniqueIds } from './UniqueIds';
import { pubKeyRegex, NostrEvent } from './utils';

export type SerializedUserList = [number, number[], number?]

export type SerializedSocialGraph = {
  uniqueIds: SerializedUniqueIds;
  followLists: SerializedUserList[];
  muteLists?: SerializedUserList[];
};

export class SocialGraph {
  private root: number;
  private followDistanceByUser = new Map<number, number>();
  private usersByFollowDistance = new Map<number, Set<number>>();
  private followedByUser = new Map<number, Set<number>>();
  private followersByUser = new Map<number, Set<number>>();
  private followListCreatedAt = new Map<number, number>();
  private mutedByUser = new Map<number, Set<number>>();
  private muteListCreatedAt = new Map<number, number>()
  private ids = new UniqueIds();

  constructor(root: string, serialized?: SerializedSocialGraph) {
    this.ids = new UniqueIds(serialized && serialized.uniqueIds);
    this.root = this.id(root);
    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));
    serialized && this.deserialize(serialized.followLists);
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
    const filtered = (Array.isArray(evs) ? evs : [evs]).filter((a) => [3/*, 10000*/].includes(a.kind));
    for (const event of filtered) {
      // TODO handle mute list, very similar
      const createdAt = event.created_at;
      if (createdAt > Math.floor(Date.now() / 1000) + 10 * 60) {
        console.debug("event.created_at more than 10 minutes in the future", event)
        continue
      }
      const author = this.id(event.pubkey);
      const existingCreatedAt = this.followListCreatedAt.get(author);
      if (existingCreatedAt && createdAt <= existingCreatedAt) {
        return;
      }
      this.followListCreatedAt.set(author, createdAt);

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
    const followLists: SerializedUserList[] = [];
    const muteLists: SerializedUserList[] = []
    for (let distance = 0; distance <= Math.max(...this.usersByFollowDistance.keys()); distance++) {
      const users = this.usersByFollowDistance.get(distance) || new Set<number>();
      for (const user of users) {
        const createdAt = this.followListCreatedAt.get(user)
        if (!createdAt) {
          continue
        }
        const followedUsers = this.followedByUser.get(user)
        if (!followedUsers) { // should not happen
          continue
        }
        followLists.push([user, [...followedUsers.values()], createdAt]);
        if (maxSize && followLists.length >= maxSize) {
          return { followLists, uniqueIds: this.ids.serialize() };
        }
        const muteListCreatedAt = this.muteListCreatedAt.get(user)
        if (!muteListCreatedAt) {
          continue
        }
        const mutedUsers = this.mutedByUser.get(user)
        if (!mutedUsers) {
          continue
        }
        muteLists.push([user, [...mutedUsers.values()], createdAt]);
      }
    }
    return { followLists, uniqueIds: this.ids.serialize(), muteLists };
  }

  private deserialize(followLists: SerializedUserList[], muteLists?: SerializedUserList[]): void {
    for (const [follower, followedUsers, createdAt] of followLists) {
      for (const followedUser of followedUsers) {
        this.addFollower(followedUser, follower);
      }
      this.followListCreatedAt.set(follower, createdAt ?? 0)
    }
    if (muteLists) {
      for (const [muter, mutedUsers, createdAt] of muteLists) {
        this.mutedByUser.set(muter, new Set(mutedUsers))
        this.muteListCreatedAt.set(muter, createdAt ?? 0)
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

  getFollowListCreatedAt(user: string) {
    return this.followListCreatedAt.get(this.id(user))
  }

  merge(other: SocialGraph) {
    console.log('size before merge', this.size())
    console.time('merge graph')
    for (const user of other) {
      const ourCreatedAt = this.getFollowListCreatedAt(user)
      const theirCreatedAt = other.getFollowListCreatedAt(user)
      if (!ourCreatedAt || (theirCreatedAt && ourCreatedAt < theirCreatedAt)) {
        const newFollows = other.getFollowedByUser(user)
        for (const follow of newFollows) {
          if (!this.followedByUser.has(this.id(follow))) {
            this.addFollower(this.id(follow), this.id(user))
          }
        }
        for (const follow of this.followedByUser.get(this.id(user)) || new Set()) {
          if (!newFollows.has(this.str(follow))) {
            this.removeFollower(follow, this.id(user))
          }
        }
      }
    }
    console.timeEnd('merge graph')
    console.log('size after merge', this.size())
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
}
