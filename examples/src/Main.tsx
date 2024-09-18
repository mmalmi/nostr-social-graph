import React, {useState, useEffect} from "react";

import SearchBox from "./SearchBox";
import { Avatar } from "./Avatar";
import socialGraph from "./socialGraph";
import { Name } from "./Name";
import ProfileCard from "./ProfileCard";
import useLocalStorage from "./useLocalStorage";

export const Main = () => {
    const [currentUser, setCurrentUser] = useLocalStorage("iris.search.currentUser", socialGraph.getRoot());
    const [selectedUser, setSelectedUser] = useState();
    const [followDistances, setFollowDistances] = useState([]);

    useEffect(() => {
      socialGraph.setRoot(currentUser);
      const distances = [1,2,3,4,5].map(d => ({
        distance: d,
        count: socialGraph.getUsersByFollowDistance(d).size
      })).filter(d => d.count > 0);
      setFollowDistances(distances);
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