import React, { useState } from "react";
import SearchBox from "./SearchBox";
import ProfileCard from "./ProfileCard";
import useLocalStorage from "./useLocalStorage";
import socialGraph from "./socialGraph";
import CurrentUser from "./CurrentUser";

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
        <div className="flex flex-col gap-4 p-4 w-full max-w-prose">
            <CurrentUser pubKey={currentUser} setSelectedUser={setSelectedUser} />
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