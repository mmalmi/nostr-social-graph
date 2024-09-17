export const pubKeyRegex = /^[0-9a-fA-F]{64}$/;

export type NostrEvent = {
created_at: number;
content: string;
tags: string[][];
kind: number;
pubkey: string;
id: string;
sig: string;
};