import React, { useState } from "react";
import SearchBox from "./SearchBox";
import ProfileCard from "./ProfileCard";
import useLocalStorage from "./useLocalStorage";
import socialGraph from "./socialGraph";
import Explore from "./Explore";
import { Avatar } from "./Avatar";

export const Main = () => {
    const [currentUser, setCurrentUser] = useLocalStorage("iris.search.currentUser", socialGraph.getRoot());
    const [selectedUser, setSelectedUser] = useState();

    const onSelect = (pubkey) => {
        setSelectedUser(pubkey);
    };

    const onSetCurrentUser = () => {
        setSelectedUser(null);
        setCurrentUser(selectedUser);
    };

    return (
        <div className="flex flex-col gap-8 p-4 py-8 w-full max-w-prose">
            <div className="flex flex-row justify-between flex-wrap items-center">
                <h1 className="text-2xl">Nostr Social Graph</h1>
                <div className="cursor-pointer" onClick={() => setSelectedUser(currentUser)}>
                    <Avatar pubKey={currentUser} />
                </div>
            </div>
            <SearchBox onSelect={onSelect} />
            <ProfileCard pubKey={selectedUser ?? currentUser} currentUser={currentUser} onSetCurrentUser={onSetCurrentUser} />
            <Explore pubKey={currentUser} selectedUser={selectedUser} setSelectedUser={setSelectedUser} />
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