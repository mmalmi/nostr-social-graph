import socialGraph from "./socialGraph"
import { CheckMarkIcon } from "./Icons"
import React from "react"

let loggedIn = false

export const Badge = ({
  pubKey,
  className,
}: {
  pubKey: string
  className?: string
}) => {
  if (!loggedIn) {
    return null
  }
  const distance = socialGraph.getFollowDistance(pubKey)
  if (distance <= 2) {
    let tooltip
    let badgeClass
    if (distance === 0) {
      tooltip = "You"
      badgeClass = "bg-primary"
    } else if (distance === 1) {
      tooltip = "Following"
      badgeClass = "bg-primary"
    } else if (distance === 2) {
      const followedByFriends = socialGraph.followedByFriends(pubKey)
      tooltip = `Followed by ${followedByFriends.size} friends`
      badgeClass = followedByFriends.size > 10 ? "bg-accent" : "bg-neutral"
    }
    return (
      <span
        className={`rounded-full aspect-square p-[2px] text-white ${badgeClass} ${className}`}
        title={tooltip}
      >
        <CheckMarkIcon size={12} />
      </span>
    )
  }
  return null
}
