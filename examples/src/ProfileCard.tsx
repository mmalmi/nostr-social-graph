import React from "react"
import socialGraph from "./socialGraph"
import { Avatar } from "./Avatar"
import FollowedBy from "./FollowedBy"
import { Name } from "./Name"

const ProfileCard = ({pubKey, currentUser, onSetCurrentUser}: {pubKey:string, currentUser:string, onSetCurrentUser: () => void }) => {
    return (
        <div className="flex flex-col gap-4 p-4 rounded-xl bg-base-100">
            <div className="flex flex-row gap-4 justify-between items-center">
            <div className="flex flex-row gap-4 items-center">
            <Avatar pubKey={pubKey} />
            <Name pubKey={pubKey} />
            </div>
            {pubKey !== currentUser && <button className="btn btn-sm rounded-full btn-primary" onClick={onSetCurrentUser}>Set as current user</button>}
            </div>
            <FollowedBy pubkey={pubKey} />
            <div className="flex flex-row gap-4">
            <div>
                Following: {socialGraph.getFollowedByUser(pubKey).size}
            </div>
            <div>
                Known followers: {socialGraph.getFollowersByUser(pubKey).size}
            </div>
            </div>
        </div>
        )
}

export default ProfileCard