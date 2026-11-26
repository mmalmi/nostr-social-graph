import { SocialGraph } from './SocialGraph.js';
import { UID } from './UniqueIds.js';

export class SocialGraphUtils {
  /**
   * Remove users who are muted by someone AND have zero followers.
   * Uses the reverse follower index in O(M) + O(incident edges).
   */
  static async removeMutedNotFollowedUsers(
    graph: SocialGraph,
    batchSize = 10_000,
    logger: (phase: string, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    const { followersByUser, userMutedBy } = graph.getInternalData();
    const mutedEntries = Array.from(userMutedBy.entries());
    const usersToRemove: number[] = [];

    logger('Scanning muted users', 0, 0);
    for (let i = 0; i < mutedEntries.length; ) {
      const end = Math.min(i + batchSize, mutedEntries.length);
      for (; i < end; i++) {
        const [user, muters] = mutedEntries[i];
        if (muters.size > 0 && !followersByUser.get(user)?.size) {
          usersToRemove.push(user);
        }
      }
      logger('Scanning muted users', i, usersToRemove.length);
      if (i < mutedEntries.length) await new Promise(resolve => setTimeout(resolve, 0));
    }

    if (usersToRemove.length > 0) {
      logger('Removing users', mutedEntries.length, usersToRemove.length);
      SocialGraphUtils.batchRemoveUsers(graph, usersToRemove);
    }
    logger('Cleanup complete', mutedEntries.length, usersToRemove.length);
    return usersToRemove.length;
  }

  /**
   * Remove multiple users in O(incident edges) using forward and reverse adjacency indexes.
   */
  static batchRemoveUsers(graph: SocialGraph, usersToRemove: number[]): void {
    if (!usersToRemove.length) return;
    const { ids, listRevisions } = graph.getInternalData();
    const graphAny = graph as any;

    const uniqueUsers = new Set<number>(usersToRemove);
    const removedData: Array<{
      user: number;
      followed: Set<number> | undefined;
      followers: Set<number> | undefined;
      muted: Set<number> | undefined;
      muters: Set<number> | undefined;
    }> = [];

    for (const revision of listRevisions.values()) revision.complete = false;

    for (const user of uniqueUsers) {
      const followed: Set<number> | undefined = graphAny.followedByUser.get(user);
      const followers: Set<number> | undefined = graphAny.followersByUser.get(user);
      const muted: Set<number> | undefined = graphAny.mutedByUser.get(user);
      const muters: Set<number> | undefined = graphAny.userMutedBy.get(user);
      removedData.push({ user, followed, followers, muted, muters });

      const str = (ids as any).uniqueIdToStr.get(user);
      if (str !== undefined) {
        listRevisions.delete(`3:${str}`);
        listRevisions.delete(`10000:${str}`);
      }
      ids.remove(user);

      const distance = graphAny.followDistanceByUser.get(user);
      if (distance !== undefined) {
        const bucket = graphAny.usersByFollowDistance.get(distance);
        if (bucket?.delete(user) && !bucket.size) graphAny.usersByFollowDistance.delete(distance);
      }

      graphAny.followDistanceByUser.delete(user);
      graphAny.followedByUser.delete(user);
      graphAny.followersByUser.delete(user);
      graphAny.followListCreatedAt.delete(user);
      graphAny.mutedByUser.delete(user);
      graphAny.userMutedBy.delete(user);
      graphAny.muteListCreatedAt.delete(user);
    }

    for (const { user, followed, followers, muted, muters } of removedData) {
      if (followers) {
        for (const follower of followers) {
          if (!uniqueUsers.has(follower)) graphAny.followedByUser.get(follower)?.delete(user);
        }
      }
      if (followed) {
        for (const target of followed) {
          if (!uniqueUsers.has(target)) graphAny.followersByUser.get(target)?.delete(user);
        }
      }
      if (muters) {
        for (const muter of muters) {
          if (!uniqueUsers.has(muter)) graphAny.mutedByUser.get(muter)?.delete(user);
        }
      }
      if (muted) {
        for (const target of muted) {
          if (!uniqueUsers.has(target)) graphAny.userMutedBy.get(target)?.delete(user);
        }
      }
    }
  }

  /**
   * Remove a user by ID from all internal data structures (backward compatibility)
   */
  static removeUserById(graph: SocialGraph, user: UID): void {
    SocialGraphUtils.batchRemoveUsers(graph, [user]);
  }

  /**
   * Get follower and muter counts by distance for a user
   */
  static stats(graph: SocialGraph, user: string): { [distance: number]: { followers: number; muters: number } } {
    const stats: { [distance: number]: { followers: number; muters: number } } = {};
    const graphAny = graph as any;
    const userId = graph.getInternalData().ids.get(user);
    if (userId === undefined) return stats;

    const distances: Map<number, number> = graphAny.followDistanceByUser;
    const followers = graphAny.followersByUser.get(userId);
    if (followers) {
      for (const follower of followers) {
        const distance = distances.get(follower);
        if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).followers++;
      }
    }

    const muters = graphAny.userMutedBy.get(userId);
    if (muters) {
      for (const muter of muters) {
        const distance = distances.get(muter);
        if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).muters++;
      }
    }
    return stats;
  }

  /**
   * Get followers set for a user ID (helper method)
   * Uses reverse index for O(1) lookup.
   */
  static getFollowersSet(graph: SocialGraph, id: number): Set<number> {
    return graph.getInternalData().followersByUser.get(id) ?? new Set<number>();
  }

  /**
   * Check if a user has any followers at any distance (efficient implementation)
   */
  static hasFollowers(graph: SocialGraph, user: string): boolean {
    const { followersByUser, ids } = graph.getInternalData();
    const userId = ids.get(user);
    return userId !== undefined && !!followersByUser.get(userId)?.size;
  }

  /**
   * Zero-allocation overmute check by internal numeric ID.
   * Only the closest distance with any follower or muter decides the outcome.
   */
  static isOvermutedById(graph: SocialGraph, userId: number, threshold = 1): boolean {
    const graphAny = graph as any;
    if (userId === graphAny.root) return false;

    const mutersSet: Set<number> | undefined = graphAny.userMutedBy.get(userId);
    if (!mutersSet?.size) return false;
    if (mutersSet.has(graphAny.root)) return true;

    const distances: Map<number, number> = graphAny.followDistanceByUser;
    let minDist = Infinity;
    let mutersAtMin = 0;
    for (const muter of mutersSet) {
      const d = distances.get(muter);
      if (d === undefined) continue;
      if (d < minDist) { minDist = d; mutersAtMin = 1; }
      else if (d === minDist) mutersAtMin++;
    }
    const maxFollowers = mutersAtMin * threshold;
    if (!mutersAtMin || !(maxFollowers > 0)) return false;

    const followersSet: Set<number> | undefined = graphAny.followersByUser.get(userId);
    if (followersSet) {
      let followersAtMin = 0;
      for (const follower of followersSet) {
        const d = distances.get(follower);
        if (d === undefined) continue;
        if (d < minDist) return false;
        if (d === minDist && ++followersAtMin >= maxFollowers) return false;
      }
    }
    return true;
  }

  /**
   * Check if a user is "overmuted" - where muters * threshold > followers
   * at the closest distance where they have any followers or muters.
   */
  static isOvermuted(graph: SocialGraph, user: string, threshold = 1): boolean {
    const userId = graph.getInternalData().ids.get(user);
    return userId !== undefined && SocialGraphUtils.isOvermutedById(graph, userId, threshold);
  }

  /**
   * Prune all overmuted users from the graph, processing by distance (closest first).
   * Returns the total number of users removed.
   */
  static async pruneOvermutedUsers(
    graph: SocialGraph,
    threshold = 1,
    logger: (distance: number, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    let totalRemoved = 0;
    const graphAny = graph as any;

    for (let distance = 1; distance <= 20; distance++) {
      const usersAtDistance: Set<number> | undefined = graphAny.usersByFollowDistance.get(distance);
      if (!usersAtDistance?.size) break;

      const overmutedUsers: number[] = [];
      for (const userId of usersAtDistance) {
        if (SocialGraphUtils.isOvermutedById(graph, userId, threshold)) {
          overmutedUsers.push(userId);
        }
      }

      logger(distance, usersAtDistance.size, overmutedUsers.length);

      if (overmutedUsers.length > 0) {
        SocialGraphUtils.batchRemoveUsers(graph, overmutedUsers);
        totalRemoved += overmutedUsers.length;
      }
    }

    await graph.recalculateFollowDistances();
    return totalRemoved;
  }
}
