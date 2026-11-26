export type UID = number;

const DENSE_ARRAY_LIMIT = 1_000_000;

export function canonicalNodeId(value: string): string {
  return /^[0-9a-fA-F]{64}$/.test(value) ? value.toLowerCase() : value;
}

/**
 * Save memory and storage by mapping repeatedly used long strings such as public keys to internal unique ID numbers.
 * Uses a flat array for dense IDs (0..N-1) and falls back to a Map only for sparse legacy IDs.
 */
export class UniqueIds {
  private strToUniqueId = new Map<string, UID>();
  private idToStrArr: (string | undefined)[] = [];
  private sparseIdToStr: Map<UID, string> | null = null;
  private currentUniqueId = 0;

  constructor() {
  }

  private switchToSparse(): Map<UID, string> {
    if (this.sparseIdToStr) return this.sparseIdToStr;
    const map = new Map<UID, string>();
    for (let i = 0; i < this.idToStrArr.length; i++) {
      const value = this.idToStrArr[i];
      if (value !== undefined) map.set(i, value);
    }
    this.idToStrArr.length = 0;
    this.sparseIdToStr = map;
    return map;
  }

  /** @internal Reset internal storage before bulk binary load. */
  clear(): void {
    this.strToUniqueId.clear();
    this.idToStrArr.length = 0;
    this.sparseIdToStr = null;
    this.currentUniqueId = 0;
  }

  /** @internal Insert a known-canonical `(id, value)` pair if both are unused. */
  tryInsertUnique(id: UID, value: string): boolean {
    if (this.hasId(id) || this.strToUniqueId.has(value)) return false;
    this.strToUniqueId.set(value, id);
    if (this.sparseIdToStr) {
      this.sparseIdToStr.set(id, value);
    } else if (id >= 0 && Number.isInteger(id) && id < DENSE_ARRAY_LIMIT && id <= this.idToStrArr.length + 1024) {
      this.idToStrArr[id] = value;
    } else {
      this.switchToSparse().set(id, value);
    }
    if (id + 1 > this.currentUniqueId) this.currentUniqueId = id + 1;
    return true;
  }

  /** @internal Fast dense append for v5 decoding (`id === currentUniqueId`). */
  pushDenseUnique(value: string): boolean {
    if (this.strToUniqueId.has(value)) return false;
    const id = this.idToStrArr.length;
    this.strToUniqueId.set(value, id);
    this.idToStrArr.push(value);
    if (id + 1 > this.currentUniqueId) this.currentUniqueId = id + 1;
    return true;
  }

  /** @internal Whether numeric ID `id` is currently assigned. */
  hasId(id: UID): boolean {
    return this.sparseIdToStr ? this.sparseIdToStr.has(id) : this.idToStrArr[id] !== undefined;
  }

  /** @internal Look up string for `id` without throwing. */
  getStr(id: UID): string | undefined {
    return this.sparseIdToStr ? this.sparseIdToStr.get(id) : this.idToStrArr[id];
  }

  /** @internal True when IDs live in a compact dense array (no sparse Map). */
  get isDense(): boolean {
    return this.sparseIdToStr === null;
  }

  /** @internal Upper bound (exclusive) of dense IDs when dense, else 0. */
  get denseLength(): number {
    return this.sparseIdToStr === null ? this.idToStrArr.length : 0;
  }

  /** Existing ID, without allocating one for a string seen for the first time. */
  get(str: string): UID | undefined {
    // Stored keys are canonical, so a direct hit needs no normalization.
    const id = this.strToUniqueId.get(str);
    if (id !== undefined) return id;
    const canonical = canonicalNodeId(str);
    return canonical === str ? undefined : this.strToUniqueId.get(canonical);
  }

  id(str: string): UID {
    const known = this.strToUniqueId.get(str);
    if (known !== undefined) return known;
    str = canonicalNodeId(str);
    // Prevent empty strings from being stored
    if (!str || str.trim() === '') {
      throw new Error('Cannot store empty or whitespace-only strings');
    }

    const existing = this.strToUniqueId.get(str);
    if (existing !== undefined) {
      return existing;
    }
    const newId = this.currentUniqueId++;
    this.strToUniqueId.set(str, newId);
    if (this.sparseIdToStr) {
      this.sparseIdToStr.set(newId, str);
    } else if (newId < DENSE_ARRAY_LIMIT && newId <= this.idToStrArr.length + 1024) {
      this.idToStrArr[newId] = str;
    } else {
      this.switchToSparse().set(newId, str);
    }
    return newId;
  }

  str(id: UID): string {
    const pub = this.getStr(id);
    if (!pub) {
      throw new Error('pub: invalid id ' + id);
    }
    return pub;
  }

  has(str: string): boolean {
    return this.get(str) !== undefined;
  }

  get size(): number {
    return this.strToUniqueId.size;
  }

  *[Symbol.iterator](): Generator<[UID, string]> {
    if (this.sparseIdToStr) {
      yield* this.sparseIdToStr.entries();
      return;
    }
    for (let i = 0; i < this.idToStrArr.length; i++) {
      const value = this.idToStrArr[i];
      if (value !== undefined) yield [i, value];
    }
  }

  remove(id: UID): void {
    const str = this.getStr(id);
    if (str !== undefined) {
      if (this.sparseIdToStr) {
        this.sparseIdToStr.delete(id);
      } else {
        this.idToStrArr[id] = undefined;
      }
      this.strToUniqueId.delete(str);
    }
  }
}
