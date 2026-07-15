import { SocialGraph } from './SocialGraph';
import { UID } from './UniqueIds';

export class SocialGraphUtils {

  /**
   * Remove users who are muted by someone AND have zero followers.
   * O(E + M) where E = follows edges, M = mutes edges.
   * Now async and non-blocking with batching.
   */
  static removeMutedNotFollowedUsers(
    graph: SocialGraph,
    batchSize = 10_000,
    logger: (phase: string, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    console.time('removeMutedNotFollowedUsers');
    let currentPhase = '';
    
    return new Promise((resolve) => {
      const { followedByUser, userMutedBy } = graph.getInternalData();

      // Phase 1: Build set of users who have at least one follower
      const hasFollowers = new Set<number>();
      const followEntries = Array.from(followedByUser.entries());
      let followProcessed = 0;

      logger('Building follower counts', 0, 0);
      console.log('Building follower index...');
      console.time('Building follower index');
      currentPhase = 'building';

      const buildFollowerSet = () => {
        const end = Math.min(followProcessed + batchSize, followEntries.length);

        for (let i = followProcessed; i < end; i++) {
          const [, followedUsers] = followEntries[i];
          for (const user of followedUsers) {
            hasFollowers.add(user);
          }
        }

        followProcessed = end;

        if (followProcessed < followEntries.length) {
          setTimeout(buildFollowerSet, 0);
        } else {
          // Phase 1 complete, start Phase 2
          scanMutedUsers();
        }
      };

      // Phase 2: Scan muted users for those with zero followers
      const usersToRemove: number[] = [];
      const mutedEntries = Array.from(userMutedBy.entries());
      let mutedProcessed = 0;

      const scanMutedUsers = () => {
        const end = Math.min(mutedProcessed + batchSize, mutedEntries.length);

        for (let i = mutedProcessed; i < end; i++) {
          const [user, muters] = mutedEntries[i];
          if (muters.size > 0 && !hasFollowers.has(user)) {
            usersToRemove.push(user);
          }
        }

        mutedProcessed = end;

        if (mutedProcessed % (batchSize * 10) === 0 || mutedProcessed === end) {
          if (currentPhase !== 'scanning') {
            console.timeEnd('Building follower index');
            console.time('Scanning muted users');
            currentPhase = 'scanning';
          }
          console.log(`Scanned ${mutedProcessed.toLocaleString()} muted users, found ${usersToRemove.length.toLocaleString()} to remove`);
          logger('Scanning muted users', mutedProcessed, usersToRemove.length);
        }

        if (mutedProcessed < mutedEntries.length) {
          setTimeout(scanMutedUsers, 0);
        } else {
          // Phase 2 complete, start Phase 3
          console.timeEnd('Scanning muted users');
          console.time('Removing users');
          currentPhase = 'removing';
          logger('Removing users', mutedProcessed, usersToRemove.length);
          batchRemoveUsers();
        }
      };

      // Phase 3: Batch remove all identified users efficiently
      const batchRemoveUsers = () => {
        if (usersToRemove.length === 0) {
          console.timeEnd('Removing users');
          console.log(`✅ Cleanup complete: removed 0 muted users with zero followers`);
          logger('Cleanup complete', mutedEntries.length, 0);
          console.timeEnd('removeMutedNotFollowedUsers');
          resolve(0);
          return;
        }

        console.log(`Batch removing ${usersToRemove.length.toLocaleString()} users...`);
        SocialGraphUtils.batchRemoveUsers(graph, usersToRemove);
        
        console.timeEnd('Removing users');
        console.log(`✅ Cleanup complete: removed ${usersToRemove.length.toLocaleString()} muted users with zero followers`);
        logger('Cleanup complete', mutedEntries.length, usersToRemove.length);
        console.timeEnd('removeMutedNotFollowedUsers');
        resolve(usersToRemove.length);
      };

      // Start Phase 1
      setTimeout(buildFollowerSet, 0);
    });
  }

  /**
   * Efficiently remove multiple users in batch to avoid O(N) operations per user
   */
  static batchRemoveUsers(graph: SocialGraph, usersToRemove: number[]): void {
    const { ids } = graph.getInternalData();
    const graphAny = graph as any;

    // Phase 1: Collect relationships before removal
    const userRelationships = new Map<number, {
      followedUsers: Set<number>;
      mutedUsers: Set<number>;
      userMuters: Set<number>;
      followDistance?: number;
    }>();

    for (const user of usersToRemove) {
      userRelationships.set(user, {
        followedUsers: graphAny.followedByUser.get(user) || new Set(),
        mutedUsers: graphAny.mutedByUser.get(user) || new Set(),
        userMuters: graphAny.userMutedBy.get(user) || new Set(),
        followDistance: graphAny.followDistanceByUser.get(user)
      });
    }

    // Phase 2: Remove from all primary data structures
    for (const user of usersToRemove) {
      const relationships = userRelationships.get(user)!;
      
      // Remove from UniqueIds
      ids.remove(user);

      // Remove from distance tracking
      if (relationships.followDistance !== undefined) {
        graphAny.usersByFollowDistance.get(relationships.followDistance)?.delete(user);
      }

      // Remove from all maps
      graphAny.followDistanceByUser.delete(user);
      graphAny.followedByUser.delete(user);
      graphAny.followersByUser.delete(user);
      graphAny.followListCreatedAt.delete(user);
      graphAny.mutedByUser.delete(user);
      graphAny.userMutedBy.delete(user);
      graphAny.muteListCreatedAt.delete(user);
    }

    // Phase 3: Clean up follow relationships - single pass through all users
    for (const [, followedSet] of graphAny.followedByUser) {
      // Remove all users in batch from this follower's follow list
      for (const userToRemove of usersToRemove) {
        followedSet.delete(userToRemove);
      }
    }

    // Phase 3.5: Clean up reverse follow index - remove deleted users from others' follower lists
    for (const [, followersSet] of graphAny.followersByUser) {
      for (const userToRemove of usersToRemove) {
        followersSet.delete(userToRemove);
      }
    }

    // Phase 4: Clean up mute relationships using inverse relationships
    for (const user of usersToRemove) {
      const relationships = userRelationships.get(user)!;
      
      // Remove from muters' mute lists
      for (const muter of relationships.userMuters) {
        const muterMuteSet = graphAny.mutedByUser.get(muter);
        if (muterMuteSet) {
          muterMuteSet.delete(user);
        }
      }

      // Remove from userMutedBy sets of users they muted
      for (const mutedUser of relationships.mutedUsers) {
        const mutedUserMuterSet = graphAny.userMutedBy.get(mutedUser);
        if (mutedUserMuterSet) {
          mutedUserMuterSet.delete(user);
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
    const userId = graphAny.id(user);

    // Get followers set (using the private method)
    const followersSet = SocialGraphUtils.getFollowersSet(graph, userId);
    
    for (const follower of followersSet) {
      const distance = graphAny.followDistanceByUser.get(follower);
      if (distance !== undefined) {
        if (!stats[distance]) {
          stats[distance] = { followers: 0, muters: 0 };
        }
        stats[distance].followers++;
      }
    }

    for (const muter of graphAny.userMutedBy.get(userId) || []) {
      const distance = graphAny.followDistanceByUser.get(muter);
      if (distance !== undefined) {
        if (!stats[distance]) {
          stats[distance] = { followers: 0, muters: 0 };
        }
        stats[distance].muters++;
      }
    }
    return stats;
  }

  /**
   * Get followers set for a user ID (helper method)
   * Uses reverse index for O(1) lookup.
   */
  static getFollowersSet(graph: SocialGraph, id: number): Set<number> {
    const { followersByUser } = graph.getInternalData();
    
    // Use reverse index for fast lookup
    return followersByUser.get(id) || new Set<number>();
  }

  /**
   * Check if a user has any followers at any distance (efficient implementation)
   */
  static hasFollowers(graph: SocialGraph, user: string): boolean {
    const graphAny = graph as any;
    const userId = graphAny.id(user);
    const { followersByUser } = graph.getInternalData();
    
    // Use reverse index for O(1) lookup
    const followers = followersByUser.get(userId);
    return followers ? followers.size > 0 : false;
  }

  /**
   * Check if a user is "overmuted" - where muters * threshold > followers 
   * at the closest distance where they have any followers or muters (efficient implementation)
   */
  static isOvermuted(graph: SocialGraph, user: string, threshold: number = 1): boolean {
    // Graph root is never considered overmuted
    if (user === graph.getRoot()) {
      return false;
    }
    
    const graphAny = graph as any;
    const userId = graphAny.id(user);
    const { followersByUser, userMutedBy } = graph.getInternalData();
    
    // Early check: if no one has muted this user, they can't be overmuted
    const mutersSet = userMutedBy.get(userId);
    if (!mutersSet || mutersSet.size === 0) {
      return false;
    }

    if (mutersSet.has(graphAny.root)) {
      return true;
    }

    // Count followers and muters by distance
    const statsByDistance = new Map<number, { followers: number; muters: number }>();
    
    // Count followers using reverse index
    const followersSet = followersByUser.get(userId);
    if (followersSet) {
      for (const follower of followersSet) {
        const distance = graphAny.followDistanceByUser.get(follower);
        if (distance !== undefined) {
          if (!statsByDistance.has(distance)) {
            statsByDistance.set(distance, { followers: 0, muters: 0 });
          }
          statsByDistance.get(distance)!.followers++;
        }
      }
    }
    
    // Count muters  
    for (const muter of mutersSet) {
      const distance = graphAny.followDistanceByUser.get(muter);
      if (distance !== undefined) {
        if (!statsByDistance.has(distance)) {
          statsByDistance.set(distance, { followers: 0, muters: 0 });
        }
        statsByDistance.get(distance)!.muters++;
      }
    }
    
    // Find closest distance with any opinions
    const sortedDistances = Array.from(statsByDistance.keys()).sort((a, b) => a - b);
    for (const distance of sortedDistances) {
      const { followers, muters } = statsByDistance.get(distance)!;
      if (followers + muters > 0) {
        return muters * threshold > followers;
      }
    }
    
    // If no one has any opinion anywhere, not considered overmuted
    return false;
  }

  /**
   * Prune all overmuted users from the graph, processing by distance (closest first).
   * Returns the total number of users removed.
   */
  static async pruneOvermutedUsers(
    graph: SocialGraph, 
    threshold: number = 1,
    logger: (distance: number, scanned: number, removed: number) => void = () => {}
  ): Promise<number> {
    console.time('pruneOvermutedUsers');
    let totalRemoved = 0;
    
    const graphAny = graph as any;
    
    // Process each distance level, starting from closest (distance 1)
    // We skip distance 0 since that's just the root user
    let distance = 1;
    let hasUsersAtDistance = true;
    
    while (hasUsersAtDistance) {
      const usersAtDistance = graphAny.usersByFollowDistance.get(distance);
      if (!usersAtDistance || usersAtDistance.size === 0) {
        hasUsersAtDistance = false;
        continue;
      }
      
      console.log(`Processing distance ${distance} (${usersAtDistance.size.toLocaleString()} users)...`);
      
      // Find overmuted users at this distance
      const overmutedUsers: number[] = [];
      for (const userId of usersAtDistance) {
        const userStr = graphAny.str(userId);
        if (SocialGraphUtils.isOvermuted(graph, userStr, threshold)) {
          overmutedUsers.push(userId);
        }
      }
      
      logger(distance, usersAtDistance.size, overmutedUsers.length);
      
      if (overmutedUsers.length > 0) {
        console.log(`  Removing ${overmutedUsers.length.toLocaleString()} overmuted users at distance ${distance}`);
        SocialGraphUtils.batchRemoveUsers(graph, overmutedUsers);
        totalRemoved += overmutedUsers.length;
      }
      
      distance++;
      
      // Safety check to prevent infinite loops
      if (distance > 20) {
        console.warn('Stopping pruning at distance 20 to prevent infinite loop');
        break;
      }
    }
    
    console.timeEnd('pruneOvermutedUsers');
    console.log(`✅ Pruning complete: removed ${totalRemoved.toLocaleString()} overmuted users total`);
    await graph.recalculateFollowDistances();
    return totalRemoved;
  }
}
