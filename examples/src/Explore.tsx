import React, { useState, useEffect, useCallback } from "react";
import { Avatar } from "./Avatar";
import socialGraph from "./socialGraph";
import ndk from "./ndk";
import { throttle } from "lodash";

interface ExploreProps {
  pubKey: string;
  selectedUser: string;
  setSelectedUser: (pubKey: string) => void;
}

const Explore = ({ pubKey, selectedUser, setSelectedUser }) => {
  const [followDistances, setFollowDistances] = useState([]);

  const debouncedSetFollowDistances = useCallback(throttle(() => {
    const distances = [1,2,3,4,5].map(d => {
      const users = Array.from(socialGraph.getUsersByFollowDistance(d));
      const randomUsers = users.sort(() => 0.5 - Math.random()).slice(0, 3);
      return {
        distance: d,
        count: users.length,
        randomUsers
      };
    }).filter(d => d.count > 0);
    setFollowDistances(distances);
  }, 1000), [])

  useEffect(() => {
    socialGraph.setRoot(pubKey);
    debouncedSetFollowDistances()
    const missing = [] as string[]
    for (const k of socialGraph.getUsersByFollowDistance(1).values()) {
      if (socialGraph.getFollowedByUser(k).size === 0) {
          missing.push(k)
      }
    }
    const sub = ndk.subscribe({kinds:[3], authors: missing})
    sub.on("event", (e) => {
      socialGraph.handleEvent(e)
      debouncedSetFollowDistances()
    })
    return () => sub.stop()
  }, [pubKey, selectedUser]);

  return (
    <div className="flex flex-col gap-4 p-4 rounded-xl bg-base-100">
      Known users by follow distance:
      {followDistances.length === 0 && <div className="flex p-4 items-center justify-center">None</div>}
      {followDistances.map((d) => (
        <div key={d.distance} className="flex flex-row justify-between items-center my-4">
          <div>
            <b>{d.distance}</b>: {d.count}
          </div>
          <div className="flex flex-row gap-2 items-center text-lg">
            {d.randomUsers.map((user) => (
              <div className="cursor-pointer" onClick={() => setSelectedUser(user)}>
                  <Avatar pubKey={user} />
              </div>
            ))}
            +
          </div>
        </div>
      ))}
    </div>
  );
};

export default Explore;