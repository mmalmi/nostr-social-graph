import path from "path";

export const SOCIAL_GRAPH_ROOT = "336f319763657d6b0e65a5b5876719e8c8dcdcf9396852be71ee26b73368b29b";

// Data directory and file paths
export const DATA_DIR = path.resolve(process.cwd(), "data");
export const SOCIAL_GRAPH_BIN = path.join(DATA_DIR, "socialGraph.bin");
export const SOCIAL_GRAPH_LARGE_BIN = path.join(DATA_DIR, "socialGraph.large.bin");
export const FUSE_INDEX_FILE = path.join(DATA_DIR, "profileIndex.json");
export const DATA_FILE = path.join(DATA_DIR, "profileData.large.json");
export const PROFILE_SEARCH_INDEX_DIR = path.join(DATA_DIR, "profileSearchIndex");
export const PROFILE_SEARCH_INDEX_ROOT = path.join(DATA_DIR, "profileSearchIndex.root.json");
export const PROFILE_PICTURE_URL_MAX_LENGTH = 255;
export const PROFILE_NAME_MAX_LENGTH = 100;
export const CRAWL_DISTANCE_DEFAULT = 4;

// Relay URLs
export const RELAY_URLS = [
  "wss://relay.snort.social",
  "wss://relay.damus.io",
  "wss://relay.nostr.band",
  "wss://nostr.wine",
  "wss://soloco.nl",
  "wss://eden.nostr.land",
  "wss://temp.iris.to",
  "wss://vault.iris.to",
]; 
