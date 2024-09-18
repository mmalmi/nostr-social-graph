import React, {useMemo} from "react"
import socialGraph from "./socialGraph"
import { Avatar } from "./Avatar"
import FollowedBy from "./FollowedBy"
import { Name } from "./Name"
import { nip19 } from "nostr-tools"
import useFollows from "./useFollows"
import useProfile from "./useProfile"

const ProfileCard = ({pubKey, currentUser, onSetCurrentUser}: {pubKey:string, currentUser:string, onSetCurrentUser: () => void }) => {
    const npub = useMemo(() => nip19.npubEncode(pubKey), [pubKey])
    useFollows(pubKey) // subscribe to follows list & update on change
    const profile = useProfile(pubKey)

    return (
        <div className="flex flex-col gap-4 p-4 rounded-xl bg-base-100">
            <div className="flex flex-row gap-4 justify-between items-center flex-wrap">
                <div className="flex flex-row gap-4 items-center">
                    <Avatar pubKey={pubKey} />
                    <Name pubKey={pubKey} />
                </div>
                {pubKey !== currentUser && <button className="btn btn-sm rounded-full btn-primary" onClick={onSetCurrentUser}>Set as current user</button>}
            </div>
            <FollowedBy pubkey={pubKey} />
            <div className="flex flex-row flex-wrap gap-4 items-end">
                <div className="badge badge-neutral">
                    Following: {socialGraph.getFollowedByUser(pubKey).size}
                </div>
                <div className="badge badge-neutral">
                    Known followers: {socialGraph.getFollowersByUser(pubKey).size}
                </div>
                {socialGraph.getFollowersByUser(pubKey).has(currentUser) && (
                    <div className="badge badge-accent badge-sm">Follows you</div>
                )}
            </div>
            <div className="flex flex-row gap-4 text-sm">
                View profile on:
                <a href={`https://beta.iris.to/${npub}`} className="link" target="_blank" rel="noopener noreferrer">Iris</a>
                <a href={`https://primal.net/p/${npub}`} className="link" target="_blank" rel="noopener noreferrer">Primal</a>
                <a href={`https://snort.social/${npub}`} className="link" target="_blank" rel="noopener noreferrer">Snort</a>
                <a href={`https://coracle.social/people/${npub}`} className="link" target="_blank" rel="noopener noreferrer">Coracle</a>
            </div>
            <div className="text-xs break-all">{npub}</div>
            <div className="text-sm">
                {profile?.about?.slice(0, 2000)}
                {profile?.about && profile.about.length > 2000 && "..."
            }</div>
        </div>
        )
}

export default ProfileCard