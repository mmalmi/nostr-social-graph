import React, {useState, useEffect} from "react";

import SearchBox from "./SearchBox";
import { Avatar } from "./Avatar";
import socialGraph from "./socialGraph";
import { Name } from "./Name";
import ProfileCard from "./ProfileCard";

export const Main = () => {
    const [currentUser, setCurrentUser] = useState(socialGraph.getRoot())
    const [selectedUser, setSelectedUser] = useState()
  
    useEffect(() => {
      socialGraph.setRoot(currentUser)
    }, [currentUser])
  
    const onSelect = (pubkey) => {
      console.log('new pubkey', pubkey)
      setSelectedUser(pubkey)
    }

    const onSetCurrentUser = () => {
        setSelectedUser(null)
        setCurrentUser(selectedUser)
    }
  
    return (
      <div className="flex flex-col gap-4 p-4 w-full max-w-prose">
        <div className="flex flex-row gap-4 items-center">
          <Avatar pubKey={currentUser} />
          <div className="flex flex-col">
            <Name pubKey={currentUser} />
            <small>Current user</small>
          </div>
        </div>
        <SearchBox onSelect={onSelect} />
        <ProfileCard pubKey={selectedUser ?? currentUser} currentUser={currentUser} onSetCurrentUser={onSetCurrentUser} />
      </div>
    );
  };