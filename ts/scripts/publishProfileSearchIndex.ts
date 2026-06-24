import fs from "fs";
import { finalizeEvent, nip19 } from "nostr-tools";
import {
  BlossomStore,
  type BlossomServer,
  type BlossomSigner,
  HashTree,
  fromHex,
  type CID,
  type Store,
} from "./hashtreeAdapter";
import { FileStore } from "./hashtreeStore";
import {
  deserializeCid,
  profileSearchIndexNhash,
  type SerializedCid,
} from "./profileSearchIndex";
import { PROFILE_SEARCH_INDEX_DIR, PROFILE_SEARCH_INDEX_ROOT } from "../src/constants";

export type PublishStats = {
  pushed: number;
  skipped: number;
  failed: number;
  bytes: number;
  errors: Array<{ hash: Uint8Array; error: Error }>;
  cancelled: boolean;
};

export type PublishResult = {
  nhash: string;
  stats: PublishStats;
};

export type PushProfileSearchIndexOptions = {
  root: CID;
  sourceStore: Store;
  targetStore: Store;
  concurrency?: number;
};

const DEFAULT_BLOSSOM_SERVERS: BlossomServer[] = [
  { url: "https://upload.iris.to", read: false, write: true },
  { url: "https://cdn.iris.to", read: true, write: false },
];

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

function parseSecretKey(raw: string): Uint8Array {
  const trimmed = raw.trim();
  if (trimmed.startsWith("nsec1")) {
    const decoded = nip19.decode(trimmed);
    if (decoded.type !== "nsec") {
      throw new Error("Expected nsec key for BLOSSOM_NSEC");
    }
    return decoded.data as Uint8Array;
  }
  const hex = trimmed.startsWith("0x") ? trimmed.slice(2) : trimmed;
  if (!/^[0-9a-fA-F]{64}$/.test(hex)) {
    throw new Error("BLOSSOM_SECRET_KEY must be 64 hex chars or nsec1...");
  }
  return fromHex(hex);
}

function loadSignerFromEnv(): BlossomSigner {
  const raw =
    process.env.BLOSSOM_SECRET_KEY ??
    process.env.BLOSSOM_NSEC ??
    process.env.NOSTR_SECRET_KEY ??
    process.env.NOSTR_NSEC;
  if (!raw) {
    throw new Error("Missing BLOSSOM_SECRET_KEY (hex) or BLOSSOM_NSEC (nsec1...)");
  }
  const secretKey = parseSecretKey(raw);
  return async (event) => finalizeEvent(event, secretKey);
}

function loadIndexRoot(): CID {
  if (!fs.existsSync(PROFILE_SEARCH_INDEX_ROOT)) {
    throw new Error(`Missing profile search index root at ${PROFILE_SEARCH_INDEX_ROOT}`);
  }
  const raw = JSON.parse(fs.readFileSync(PROFILE_SEARCH_INDEX_ROOT, "utf-8")) as SerializedCid;
  return deserializeCid(raw);
}

function resolvePublishConcurrency(): number | undefined {
  const raw = process.env.PROFILE_SEARCH_PUBLISH_CONCURRENCY;
  if (!raw) {
    return undefined;
  }
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return undefined;
  }
  return parsed;
}

export async function pushProfileSearchIndex(
  options: PushProfileSearchIndexOptions
): Promise<PublishResult> {
  const tree = new HashTree({ store: options.sourceStore });
  const nhash = profileSearchIndexNhash(options.root);
  if (!nhash) {
    throw new Error("Missing profile search index root");
  }

  const stats = await tree.push(options.root, options.targetStore, {
    concurrency: options.concurrency,
  });

  return { nhash, stats };
}

export async function publishProfileSearchIndex(options: {
  root?: CID;
  sourceDir?: string;
  servers?: BlossomServer[];
  signer?: BlossomSigner;
  concurrency?: number;
} = {}): Promise<PublishResult> {
  const root = options.root ?? loadIndexRoot();
  const sourceDir = options.sourceDir ?? PROFILE_SEARCH_INDEX_DIR;

  if (!fs.existsSync(sourceDir)) {
    throw new Error(`Missing profile search index directory at ${sourceDir}`);
  }

  const sourceStore = new FileStore(sourceDir);
  const hasRoot = await sourceStore.has(root.hash);
  if (!hasRoot) {
    throw new Error(`Profile search index root block not found in ${sourceDir}`);
  }

  const servers =
    options.servers ??
    parseBlossomServers(
      process.env.PROFILE_SEARCH_BLOSSOM_SERVERS ?? process.env.BLOSSOM_SERVERS
    );
  const signer = options.signer ?? loadSignerFromEnv();
  const targetStore = new BlossomStore({ servers, signer });

  return pushProfileSearchIndex({
    root,
    sourceStore,
    targetStore,
    concurrency: options.concurrency ?? resolvePublishConcurrency(),
  });
}

const isDirectRun = process.argv.some((arg) => arg.includes("publishProfileSearchIndex"));

if (isDirectRun && process.argv.includes("--run")) {
  publishProfileSearchIndex()
    .then((result) => {
      console.log("Profile search index nhash:", result.nhash);
      console.log(
        "Publish stats:",
        JSON.stringify(
          {
            pushed: result.stats.pushed,
            skipped: result.stats.skipped,
            failed: result.stats.failed,
            bytes: result.stats.bytes,
            cancelled: result.stats.cancelled,
            errors: result.stats.errors.length,
          },
          null,
          2
        )
      );
    })
    .catch((error) => {
      console.error("Profile search index publish failed:", error);
      process.exit(1);
    });
}
