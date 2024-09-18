import {NDKEvent, NDKUserProfile} from "@nostr-dev-kit/ndk"
import {useEffect, useState} from "react"
import ndk from "./ndk"
import fuse from "./fuse"
import fuseData from "../../data/fuse_data.json"

const profileCache = new Map<string, NDKUserProfile>()
fuseData.forEach((v) => {
  if (v[0] && v[1]) {
    let pictureUrl = v[3]
    if (pictureUrl && !pictureUrl.startsWith('http://')) {
      pictureUrl = `https://${pictureUrl}`
    }
    profileCache.set(v[0], {name:v[1], picture: pictureUrl || undefined})
  }
})

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
        fuse.remove((item) => item.pubKey === pubKey) // Remove previous entry if exists
        fuse.add({ pubKey, name: profile.name || profile.username, nip05: profile.nip05?.slice(profile.nip05.indexOf("@")[0]) })
        setProfile(profile)
      }
    })
    return () => {
      sub.stop()
    }
  }, [pubKey, profile, subscribe])

  return profile
}
