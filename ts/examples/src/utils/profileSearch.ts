import fuse from "./fuse";
import profileData from "../../../data/profileData.json";
import { ProfileSearchIndex } from "../../../scripts/profileSearchIndex";
import { BlossomStore, nhashDecode, type BlossomServer } from "../../../scripts/hashtreeAdapter";

export type SearchProfile = {
  pubKey: string;
  name: string;
  nip05?: string;
};

export type SearchHit = {
  item: SearchProfile;
  score: number;
};

type SearchOptions = {
  limit?: number;
  fullMatch?: boolean;
};

const PROFILE_MAP = new Map<string, SearchProfile>();
for (const entry of profileData as string[][]) {
  const [pubKey, name, nip05] = entry;
  if (!pubKey || !name) continue;
  PROFILE_MAP.set(pubKey, { pubKey, name, nip05: nip05 || undefined });
}

const DEFAULT_BLOSSOM_SERVERS: BlossomServer[] = [
  { url: "https://upload.iris.to", read: false, write: true },
  { url: "https://cdn.iris.to", read: true, write: false },
];

function parseBlossomServers(raw?: string): BlossomServer[] {
  if (!raw) {
    return DEFAULT_BLOSSOM_SERVERS;
  }
  const urls = raw
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
  if (urls.length === 0) {
    return DEFAULT_BLOSSOM_SERVERS;
  }
  return urls.map((url) => ({
    url,
    ...inferBlossomRole(url),
  }));
}

function inferBlossomRole(url: string): Pick<BlossomServer, "read" | "write"> {
  try {
    const host = new URL(url).hostname;
    if (host.startsWith("upload.")) {
      return { read: false, write: true };
    }
    if (host.startsWith("cdn.") || host.startsWith("hashtree.")) {
      return { read: true, write: false };
    }
  } catch {
    // Fall through to defaults when URL parsing fails.
  }
  return { read: true, write: false };
}

function resolveIndexRef(): string | null {
  const raw = import.meta.env.VITE_PROFILE_SEARCH_INDEX ?? import.meta.env.VITE_PROFILE_SEARCH_NHASH;
  if (!raw || typeof raw !== "string") {
    return null;
  }
  const trimmed = raw.trim();
  return trimmed.length ? trimmed : null;
}

function createHashtreeSearch() {
  const indexRef = resolveIndexRef();
  if (!indexRef || !indexRef.startsWith("nhash1")) {
    return null;
  }
  try {
    const root = nhashDecode(indexRef);
    const store = new BlossomStore({
      servers: parseBlossomServers(import.meta.env.VITE_BLOSSOM_SERVERS),
    });
    const index = new ProfileSearchIndex(store);
    return { index, root };
  } catch (error) {
    console.warn("Invalid profile search index ref:", error);
    return null;
  }
}

const HASHTREE_SEARCH = createHashtreeSearch();

function toSearchProfile(pubKey: string): SearchProfile {
  const cached = PROFILE_MAP.get(pubKey);
  if (cached) {
    return cached;
  }
  return { pubKey, name: pubKey };
}

function normalizeScore(score: number): number {
  return 1 / Math.max(score, 1);
}

export async function searchProfiles(
  query: string,
  options: SearchOptions = {}
): Promise<SearchHit[]> {
  const trimmed = query.trim();
  if (!trimmed) {
    return [];
  }

  if (!HASHTREE_SEARCH) {
    return fuse.search(trimmed, { limit: options.limit }).map((result) => ({
      item: result.item,
      score: result.score ?? 1,
    }));
  }

  const results = await HASHTREE_SEARCH.index.search(HASHTREE_SEARCH.root, trimmed, {
    limit: options.limit,
    fullMatch: options.fullMatch,
  });

  return results.map((result) => ({
    item: toSearchProfile(result.id),
    score: normalizeScore(result.score),
  }));
}
