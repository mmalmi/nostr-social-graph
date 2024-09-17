import { ID, STR, UID } from './UniqueIds';
import { pubKeyRegex, NostrEvent } from './utils';

/**
 * A social graph, or "web of trust," for Nostr users and their followers, built from Nostr follow list events.
 *
 * Uses include:
 *
 * - Data Filtering: Display content only from users you follow or those followed by your friends.
 * - Connection Indicators: Show on user avatars and profiles, for example:
 *     - "2nd degree connection": A user followed by someone you follow.
 *     - "Follows you": A user who follows you.
 *     - Mutual friends: Indicates how many of your friends follow a user.
 *     - Follower count: Shows how many known followers a user has.
 */
export class SocialGraph {
  private root: UID;
  private followDistanceByUser = new Map<UID, number>();
  private usersByFollowDistance = new Map<number, Set<UID>>();
  private followedByUser = new Map<UID, Set<UID>>();
  private followersByUser = new Map<UID, Set<UID>>();
  private latestFollowEventTimestamps = new Map<UID, number>();

  /**
   * @param root "Root user" of the social graph.
   * Follow distance to this user is 0. Users followed by this user are at distance 1, and so on.
   */
  constructor(root: string) {
    this.root = ID(root);
    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));
  }

  /**
   * Change the root user of the social graph, recalculating follow distances.
   * @param root
   */
  setRoot(root: string) {
    const rootId = ID(root);
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

      const followers = this.followersByUser.get(user) || new Set<UID>();
      for (const follower of followers) {
        if (!this.followDistanceByUser.has(follower)) {
          const newFollowDistance = distance + 1;
          this.followDistanceByUser.set(follower, newFollowDistance);
          if (!this.usersByFollowDistance.has(newFollowDistance)) {
            this.usersByFollowDistance.set(newFollowDistance, new Set());
          }
          this.usersByFollowDistance.get(newFollowDistance)!.add(follower);
          queue.push(follower);
        }
      }
    }
  }

  /**
   * Handle a follow event, updating the social graph.
   * @param evs
   */
  handleEvent(evs: NostrEvent | Array<NostrEvent>) {
    const filtered = (Array.isArray(evs) ? evs : [evs]).filter((a) => a.kind === 3);
    if (filtered.length === 0) {
      return;
    }
    for (const event of filtered) {
      const author = ID(event.pubkey);
      const timestamp = event.created_at;
      const existingTimestamp = this.latestFollowEventTimestamps.get(author);
      if (existingTimestamp && timestamp <= existingTimestamp) {
        return;
      }
      this.latestFollowEventTimestamps.set(author, timestamp);

      // Collect all users followed in the new event.
      const followedInEvent = new Set<UID>();
      for (const tag of event.tags) {
        if (tag[0] === 'p') {
          if (!pubKeyRegex.test(tag[1])) {
            continue;
          }
          const followedUser = ID(tag[1]);
          if (followedUser !== author) {
            followedInEvent.add(followedUser);
          }
        }
      }

      // Get the set of users currently followed by the author.
      const currentlyFollowed = this.followedByUser.get(author) || new Set<UID>();

      // Find users that need to be removed.
      for (const user of currentlyFollowed) {
        if (!followedInEvent.has(user)) {
          this.removeFollower(user, author);
        }
      }

      // Add or update the followers based on the new event.
      for (const user of followedInEvent) {
        this.addFollower(user, author);
      }
    }
  }

  /**
   * Check if a user is following another user.
   * @param follower
   * @param followedUser
   */
  isFollowing(follower: string, followedUser: string): boolean {
    const followedUserId = ID(followedUser);
    const followerId = ID(follower);
    return !!this.followedByUser.get(followerId)?.has(followedUserId);
  }

  /**
   * Get the follow distance from the root user to another user.
   * @param user
   */
  getFollowDistance(user: string): number {
    const distance = this.followDistanceByUser.get(ID(user));
    return distance === undefined ? 1000 : distance;
  }

  private addUserByFollowDistance(distance: number, user: UID) {
    if (!this.usersByFollowDistance.has(distance)) {
      this.usersByFollowDistance.set(distance, new Set());
    }
    this.usersByFollowDistance.get(distance)?.add(user);
    // remove from higher distances
    for (const d of this.usersByFollowDistance.keys()) {
      if (d > distance) {
        this.usersByFollowDistance.get(d)?.delete(user);
      }
    }
  }

  private addFollower(followedUser: UID, follower: UID) {
    if (typeof followedUser !== 'number' || typeof follower !== 'number') {
      throw new Error('Invalid user id');
    }
    if (!this.followersByUser.has(followedUser)) {
      this.followersByUser.set(followedUser, new Set<UID>());
    }
    this.followersByUser.get(followedUser)?.add(follower);

    if (!this.followedByUser.has(follower)) {
      this.followedByUser.set(follower, new Set<UID>());
    }

    if (followedUser !== this.root) {
      let newFollowDistance;
      if (follower === this.root) {
        // basically same as the next "else" block, but faster
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

  private removeFollower(unfollowedUser: UID, follower: UID) {
    this.followersByUser.get(unfollowedUser)?.delete(follower);
    this.followedByUser.get(follower)?.delete(unfollowedUser);

    if (unfollowedUser === this.root) {
      return;
    }

    // iterate over remaining followers and set the smallest follow distance
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

  // TODO subscription methods for followersByUser and followedByUser. and maybe messagesByTime. and replies
  /**
   * Get the number of known followers for a user.
   * @param address
   */
  followerCount(address: string) {
    const id = ID(address);
    return this.followersByUser.get(id)?.size ?? 0;
  }

  /**
   * Get the number of users you follow that follow a given user.
   * @param address
   */
  followedByFriendsCount(address: string) {
    let count = 0;
    const id = ID(address);
    for (const follower of this.followersByUser.get(id) ?? []) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        count++; // should we stop at 10?
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

  /**
   * Get the users you follow that are following a given user.
   * @param address
   */
  followedByFriends(address: string) {
    const id = ID(address);
    const set = new Set<string>();
    for (const follower of this.followersByUser.get(id) ?? []) {
      if (this.followedByUser.get(this.root)?.has(follower)) {
        set.add(STR(follower));
      }
    }
    return set;
  }

  /**
   * Get the users followed by a given user.
   * @param user
   * @param includeSelf
   */
  getFollowedByUser(user: string, includeSelf = false): Set<string> {
    const userId = ID(user);
    const set = new Set<string>();
    for (const id of this.followedByUser.get(userId) || []) {
      set.add(STR(id));
    }
    if (includeSelf) {
      set.add(user);
    }
    return set;
  }

  /**
   * Get the known followers of a given user.
   * @param address
   */
  getFollowersByUser(address: string): Set<string> {
    const userId = ID(address);
    const set = new Set<string>();
    for (const id of this.followersByUser.get(userId) || []) {
      set.add(STR(id));
    }
    return set;
  }

  /**
   * Serialize the social graph to a JSON string.
   * @param maxSize Optional maximum number of follow relationships to include in the serialized output.
   * @returns A JSON string representing the serialized social graph.
   */
  serialize(maxSize?: number): string {
    const pairs: [number, number][] = [];
    for (let distance = 0; distance <= Math.max(...this.usersByFollowDistance.keys()); distance++) {
      const users = this.usersByFollowDistance.get(distance) || new Set<UID>();
      for (const user of users) {
        const followers = this.followersByUser.get(user) || new Set<UID>();
        for (const follower of followers) {
          pairs.push([follower, user]);
          if (maxSize && pairs.length >= maxSize) {
            return JSON.stringify(pairs);
          }
        }
      }
    }
    return JSON.stringify(pairs);
  }

  /**
   * Deserialize a JSON string to reconstruct the social graph.
   * @param serialized A JSON string representing the serialized social graph.
   */
  deserialize(serialized: string): void {
    this.followDistanceByUser.clear();
    this.usersByFollowDistance.clear();
    this.followedByUser.clear();
    this.followersByUser.clear();
    this.latestFollowEventTimestamps.clear();

    this.followDistanceByUser.set(this.root, 0);
    this.usersByFollowDistance.set(0, new Set([this.root]));

    const pairs: [number, number][] = JSON.parse(serialized);
    for (const [follower, followedUser] of pairs) {
      this.addFollower(followedUser, follower);
    }
  }

  /**
   * Get the users at a specified follow distance.
   * @param distance
   */
  getUsersByFollowDistance(distance: number): Set<string> {
    const users = this.usersByFollowDistance.get(distance) || new Set<UID>();
    const result = new Set<string>();
    for (const user of users) {
      result.add(STR(user));
    }
    return result;
  }
}
