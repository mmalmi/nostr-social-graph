import { SearchIndex, type SearchOptions, type SearchResult } from '@hashtree/index';
import { fromHex, nhashEncode, toHex, type CID, type Store } from './hashtreeAdapter';

export type ProfileSearchRecord = {
  pubKey: string;
  name: string;
  nip05?: string;
  aliases?: string[];
};

export type SerializedCid = {
  hash: string;
  key?: string;
};

export type ProfileSearchIndexOptions = {
  order?: number;
  minKeywordLength?: number;
  stopWords?: Set<string>;
  prefix?: string;
};

export function serializeCid(cid: CID): SerializedCid {
  return {
    hash: toHex(cid.hash),
    key: cid.key ? toHex(cid.key) : undefined,
  };
}

export function deserializeCid(serialized: SerializedCid): CID {
  return {
    hash: fromHex(serialized.hash),
    key: serialized.key ? fromHex(serialized.key) : undefined,
  };
}

export function profileSearchIndexNhash(root: CID | null): string | null {
  if (!root) {
    return null;
  }
  return nhashEncode(root);
}

export class ProfileSearchIndex {
  private searchIndex: SearchIndex;
  private prefix: string;

  constructor(store: Store, options: ProfileSearchIndexOptions = {}) {
    this.searchIndex = new SearchIndex(store, {
      order: options.order,
      minKeywordLength: options.minKeywordLength,
      stopWords: options.stopWords,
    });
    this.prefix = options.prefix ?? 'p:';
  }

  buildTerms(profile: ProfileSearchRecord): string[] {
    return this.searchIndex.parseKeywords(this.buildSearchText(profile));
  }

  async indexProfile(root: CID | null, profile: ProfileSearchRecord): Promise<CID> {
    const terms = this.buildTerms(profile);
    return this.searchIndex.index(root, this.prefix, terms, profile.pubKey, '');
  }

  async removeProfile(root: CID | null, profile: ProfileSearchRecord): Promise<CID | null> {
    if (!root) return null;
    const terms = this.buildTerms(profile);
    return this.searchIndex.remove(root, this.prefix, terms, profile.pubKey);
  }

  async updateProfile(
    root: CID | null,
    previous: ProfileSearchRecord | undefined,
    next: ProfileSearchRecord
  ): Promise<CID> {
    let newRoot = root;
    if (previous) {
      newRoot = await this.removeProfile(newRoot, previous);
    }
    return this.indexProfile(newRoot, next);
  }

  async search(
    root: CID | null,
    query: string,
    options?: SearchOptions
  ): Promise<SearchResult[]> {
    return this.searchIndex.search(root, this.prefix, query, options);
  }

  private buildSearchText(profile: ProfileSearchRecord): string {
    const parts = [profile.name, profile.nip05, profile.pubKey];
    if (profile.aliases?.length) {
      parts.push(...profile.aliases);
    }
    return parts.filter(Boolean).join(' ');
  }
}
