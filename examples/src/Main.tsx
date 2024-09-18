import React, { useState, useEffect } from "react";
import SearchBox from "./SearchBox";
import ProfileCard from "./ProfileCard";
import useLocalStorage from "./useLocalStorage";
import {socialGraphLoaded, LOCALSTORAGE_PUBLICKEY, DEFAULT_SOCIAL_GRAPH_ROOT} from "./socialGraph";
import Explore from "./Explore";
import { Avatar } from "./Avatar";

export const Main = () => {
    const [currentUser, setCurrentUser] = useLocalStorage(LOCALSTORAGE_PUBLICKEY, DEFAULT_SOCIAL_GRAPH_ROOT);
    const [selectedUser, setSelectedUser] = useState();
    const [ready, setReady] = useState(false)

    useEffect(() => {
        socialGraphLoaded.then(() => setReady(true))
    }, [])

    const onSelect = (pubkey) => {
        setSelectedUser(pubkey);
    };

    const onSetCurrentUser = () => {
        setSelectedUser(null);
        setCurrentUser(selectedUser);
    };

    if (!ready) {
        return null
    }

    return (
        <div className="flex flex-col gap-8 p-4 w-full max-w-prose">
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