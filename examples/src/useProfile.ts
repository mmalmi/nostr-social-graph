import NDK, {NDKEvent, NDKUserProfile} from "@nostr-dev-kit/ndk"
import {useEffect, useState} from "react"
import {LRUCache} from "typescript-lru-cache"

const ndk = new NDK({
  explicitRelayUrls: [
    "wss://relay.snort.social",
    "wss://relay.damus.io",
    "wss://relay.nostr.band",
    "wss://nostr.wine",
    "wss://soloco.nl",
    "wss://eden.nostr.land",
  ],
})
ndk.connect()

const profileCache = new LRUCache<string, NDKUserProfile>({maxSize: 500})

export default function useProfile(pubKey?: string, subscribe = false) {
  const [profile, setProfile] = useState(
    profileCache.get(pubKey || "") || null
  )

  // Reset profile state when pubKey changes
  useEffect(() => {
    setProfile(profileCache.get(pubKey || "") || null)
  }, [pubKey])

  useEffect(() => {
    if (!pubKey) {
      return
    }
    if (profile && !subscribe) {
      return
    }
    const sub = ndk.subscribe(
      {kinds: [0], authors: [pubKey]},
      {closeOnEose: false},
      undefined,
      true
    )
    let latest = 0
    sub.on("event", (event: NDKEvent) => {
      if (event.pubkey === pubKey && event.kind === 0) {
        if (!event.created_at || event.created_at <= latest) {
          return
        }
        latest = event.created_at
        const profile = JSON.parse(event.content)
        profile.created_at = event.created_at
        profileCache.set(pubKey, profile)
        setProfile(profile)
      }
    })
    return () => {
      sub.stop()
    }
  }, [pubKey, profile, subscribe])

  return profile
}
