import { SocialGraph, CompactEdgeEntry, DistanceTable, removeCompactEdge } from './SocialGraph.js';
import { UID } from './UniqueIds.js';

function entrySize(entry: CompactEdgeEntry | undefined): number {
  if (entry === undefined) return 0;
  return typeof entry === 'number' ? 1 : entry.size;
}

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
    const graphAny = graph as any;
    const followersByUser: Map<number, CompactEdgeEntry> = graphAny.followersByUser;
    const userMutedBy: Map<number, CompactEdgeEntry> = graphAny.userMutedBy;
    const mutedEntries = Array.from(userMutedBy.entries());
    const usersToRemove: number[] = [];

    logger('Scanning muted users', 0, 0);
    for (let i = 0; i < mutedEntries.length; ) {
      const end = Math.min(i + batchSize, mutedEntries.length);
      for (; i < end; i++) {
        const [user, muters] = mutedEntries[i];
        if (entrySize(muters) > 0 && entrySize(followersByUser.get(user)) === 0) {
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
    const graphAny = graph as any;
    const ids = graphAny.ids;
    const listRevisions: Map<string, { complete: boolean }> = graphAny.listRevisions;

    const uniqueUsers = new Set<number>(usersToRemove);
    const removedData: Array<{
      user: number;
      followed: Set<number> | undefined;
      followers: CompactEdgeEntry | undefined;
      muted: Set<number> | undefined;
      muters: CompactEdgeEntry | undefined;
    }> = [];

    for (const revision of listRevisions.values()) revision.complete = false;

    for (const user of uniqueUsers) {
      const followed: Set<number> | undefined = graphAny.followedByUser.get(user);
      const followers: CompactEdgeEntry | undefined = graphAny.followersByUser.get(user);
      const muted: Set<number> | undefined = graphAny.mutedByUser.get(user);
      const muters: CompactEdgeEntry | undefined = graphAny.userMutedBy.get(user);
      removedData.push({ user, followed, followers, muted, muters });

      const str = ids.getStr(user);
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
      if (followers !== undefined) {
        if (typeof followers === 'number') {
          if (!uniqueUsers.has(followers)) graphAny.followedByUser.get(followers)?.delete(user);
        } else {
          for (const follower of followers) {
            if (!uniqueUsers.has(follower)) graphAny.followedByUser.get(follower)?.delete(user);
          }
        }
      }
      if (followed) {
        for (const target of followed) {
          if (!uniqueUsers.has(target)) removeCompactEdge(graphAny.followersByUser, target, user);
        }
      }
      if (muters !== undefined) {
        if (typeof muters === 'number') {
          if (!uniqueUsers.has(muters)) graphAny.mutedByUser.get(muters)?.delete(user);
        } else {
          for (const muter of muters) {
            if (!uniqueUsers.has(muter)) graphAny.mutedByUser.get(muter)?.delete(user);
          }
        }
      }
      if (muted) {
        for (const target of muted) {
          if (!uniqueUsers.has(target)) removeCompactEdge(graphAny.userMutedBy, target, user);
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
    const userId = graphAny.ids.get(user);
    if (userId === undefined) return stats;

    const distances: DistanceTable = graphAny.followDistanceByUser;
    const followers: CompactEdgeEntry | undefined = graphAny.followersByUser.get(userId);
    if (followers !== undefined) {
      if (typeof followers === 'number') {
        const distance = distances.get(followers);
        if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).followers++;
      } else {
        for (const follower of followers) {
          const distance = distances.get(follower);
          if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).followers++;
        }
      }
    }

    const muters: CompactEdgeEntry | undefined = graphAny.userMutedBy.get(userId);
    if (muters !== undefined) {
      if (typeof muters === 'number') {
        const distance = distances.get(muters);
        if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).muters++;
      } else {
        for (const muter of muters) {
          const distance = distances.get(muter);
          if (distance !== undefined) (stats[distance] ??= { followers: 0, muters: 0 }).muters++;
        }
      }
    }
    return stats;
  }

  /**
   * Get followers set for a user ID (helper method)
   * Uses reverse index for O(1) lookup.
   */
  static getFollowersSet(graph: SocialGraph, id: number): Set<number> {
    const entry: CompactEdgeEntry | undefined = (graph as any).followersByUser.get(id);
    if (entry === undefined) return new Set<number>();
    return typeof entry === 'number' ? new Set([entry]) : entry;
  }

  /**
   * Check if a user has any followers at any distance (efficient implementation)
   */
  static hasFollowers(graph: SocialGraph, user: string): boolean {
    const graphAny = graph as any;
    const userId = graphAny.ids.get(user);
    return userId !== undefined && entrySize(graphAny.followersByUser.get(userId)) > 0;
  }

  /**
   * Zero-allocation overmute check by internal numeric ID.
   * Only the closest distance with any follower or muter decides the outcome.
   */
  static isOvermutedById(graph: SocialGraph, userId: number, threshold = 1): boolean {
    const graphAny = graph as any;
    const root: number = graphAny.root;
    if (userId === root) return false;

    const mutersEntry: CompactEdgeEntry | undefined = graphAny.userMutedBy.get(userId);
    if (mutersEntry === undefined) return false;

    const distances: DistanceTable = graphAny.followDistanceByUser;
    let minDist = Infinity;
    let mutersAtMin = 0;
    if (typeof mutersEntry === 'number') {
      if (mutersEntry === root) return true;
      const d = distances.get(mutersEntry);
      if (d !== undefined) { minDist = d; mutersAtMin = 1; }
    } else {
      if (!mutersEntry.size) return false;
      if (mutersEntry.has(root)) return true;
      for (const muter of mutersEntry) {
        const d = distances.get(muter);
        if (d === undefined) continue;
        if (d < minDist) { minDist = d; mutersAtMin = 1; }
        else if (d === minDist) mutersAtMin++;
      }
    }
    const maxFollowers = mutersAtMin * threshold;
    if (!mutersAtMin || !(maxFollowers > 0)) return false;

    const followersEntry: CompactEdgeEntry | undefined = graphAny.followersByUser.get(userId);
    if (followersEntry !== undefined) {
      if (typeof followersEntry === 'number') {
        const d = distances.get(followersEntry);
        if (d !== undefined && (d < minDist || (d === minDist && 1 >= maxFollowers))) return false;
      } else {
        let followersAtMin = 0;
        for (const follower of followersEntry) {
          const d = distances.get(follower);
          if (d === undefined) continue;
          if (d < minDist) return false;
          if (d === minDist && ++followersAtMin >= maxFollowers) return false;
        }
      }
    }
    return true;
  }

  /**
   * Check if a user is "overmuted" - where muters * threshold > followers
   * at the closest distance where they have any followers or muters.
   */
  static isOvermuted(graph: SocialGraph, user: string, threshold = 1): boolean {
    const userId = (graph as any).ids.get(user);
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
