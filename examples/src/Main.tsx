import React, {useState, useEffect, useCallback} from "react";

import SearchBox from "./SearchBox";
import { Avatar } from "./Avatar";
import socialGraph from "./socialGraph";
import { Name } from "./Name";
import ProfileCard from "./ProfileCard";
import useLocalStorage from "./useLocalStorage";
import ndk from "./ndk";
import { throttle } from "lodash";

export const Main = () => {
    const [currentUser, setCurrentUser] = useLocalStorage("iris.search.currentUser", socialGraph.getRoot());
    const [selectedUser, setSelectedUser] = useState();
    const [followDistances, setFollowDistances] = useState([]);

    const debouncedSetFollowDistances = useCallback(throttle(() => {
        const distances = [1,2,3,4,5].map(d => ({
          distance: d,
          count: socialGraph.getUsersByFollowDistance(d).size
        })).filter(d => d.count > 0);
        setFollowDistances(distances);
    }, 1000), [])

    useEffect(() => {
      socialGraph.setRoot(currentUser);
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
    }, [currentUser]);
  
    const onSelect = (pubkey) => {
      console.log('new pubkey', pubkey);
      setSelectedUser(pubkey);
    };

    const onSetCurrentUser = () => {
        setSelectedUser(null);
        setCurrentUser(selectedUser);
    };
  
    return (
      <div className="flex flex-col gap-4 p-4 w-full max-w-prose">
        <div className="flex flex-row gap-4 items-center">
          <Avatar pubKey={currentUser} />
          <div className="flex flex-col">
            <Name pubKey={currentUser} />
            <small>Current user</small>
          </div>
        </div>
        <div>
            Known users by follow distance:
            {followDistances.map((d) => <div key={d.distance}><b>{d.distance}</b>: {d.count}</div>)}
        </div>
        <SearchBox onSelect={onSelect} />
        <ProfileCard pubKey={selectedUser ?? currentUser} currentUser={currentUser} onSetCurrentUser={onSetCurrentUser} />
        <div className="text-sm text-gray-500 mt-4 flex flex-row gap-4">
          <a href="https://github.com/mmalmi/nostr-social-graph" target="_blank" rel="noopener noreferrer" className="link">
            GitHub
          </a>
          <a href="https://www.npmjs.com/package/nostr-social-graph" target="_blank" rel="noopener noreferrer" className="link">
            npm
          </a>
        </div>
      </div>
    );
  };