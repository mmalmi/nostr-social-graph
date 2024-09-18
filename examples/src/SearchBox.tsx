import React, {useEffect, useRef, useState} from "react"
import {nip19} from "nostr-tools"
import Fuse from "fuse.js"
import fuseIndexData from "../../data/fuse_index.json"
import fuseData from "../../data/fuse_data.json"
import classNames from "classnames"
import {SearchIcon} from "./Icons"
import { Avatar } from "./Avatar"

const fuseIndex = Fuse.parseIndex(fuseIndexData)
const fuse = new Fuse<SearchResult>(fuseData, { keys: ["name", "pubKey"] }, fuseIndex)
console.log(fuse)

const NOSTR_REGEX = /(npub|note|nevent)1[a-zA-Z0-9]{58,300}/gi
const HEX_REGEX = /[0-9a-fA-F]{64}/gi
const MAX_RESULTS = 10

type SearchResult = {
    query?: string
    name: string
    pubKey: string
    nip05?: string
  }

// this component is used for global search in the Header.tsx
// and for searching assignees in Issues & PRs
interface SearchBoxProps {
  redirect?: boolean
}

function SearchBox({redirect = true}: SearchBoxProps) {
  const [searchResults, setSearchResults] = useState([])
  const [activeResult, setActiveResult] = useState(0)
  const [value, setValue] = useState("")
  const inputRef = useRef(null)

  useEffect(() => {
    const v = value.trim()
    if (v) {
      if (v.match(NOSTR_REGEX)) {
        setValue("")
        return
      } else if (v.match(HEX_REGEX)) {
        setValue("")
        return
      }

      const results = fuse.search(v, { limit: MAX_RESULTS })
      console.log('results', results)
      if (!redirect) {
        setActiveResult(1)
      } else {
        setActiveResult(0)
      }
      setSearchResults(results.map((result) => result.item))
    } else {
      setSearchResults([])
    }
  }, [value])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!value) return

      if (e.key === "ArrowDown") {
        e.preventDefault()
        setActiveResult((prev) => (prev + 1) % MAX_RESULTS)
      } else if (e.key === "ArrowUp") {
        e.preventDefault()
        setActiveResult((prev) => (prev - 1 + MAX_RESULTS) % MAX_RESULTS)
      } else if (e.key === "Escape") {
        setValue("")
        setSearchResults([])
      } else if (e.key === "Enter" && searchResults.length > 0) {
        const activeItem = searchResults[activeResult]
        if (redirect) {
          // navigate(`/${nip19.npubEncode(activeItem.pubKey)}`)
        }
        setValue("")
        setSearchResults([])
      }
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [searchResults, activeResult])

  const handleSearchResultClick = (pubKey: string) => {
    if (redirect) {
      setValue("")
      setSearchResults([])
      // navigate(`/${nip19.npubEncode(pubKey)}`)
    }
  }

  return (
    <div className={"dropdown dropdown-open"}>
      <label className="input flex items-center gap-2">
        <input
          type="text"
          className="grow"
          placeholder="Search"
          value={value}
          ref={inputRef}
          onChange={(e) => setValue(e.target.value)}
        />
        <SearchIcon />
      </label>
      {searchResults.length > 0 && (
        <ul className="dropdown-content menu shadow bg-base-100 rounded-box z-10 w-full">
          {searchResults.slice(0, MAX_RESULTS).map((result, index) => result && (
            <li
              key={result.pubKey}
              className={classNames("cursor-pointer rounded-md", {
                "bg-primary text-primary-content": index === activeResult,
                "hover:bg-primary/50": index !== activeResult,
              })}
              onClick={() => handleSearchResultClick(result.pubKey)}
            >
              <div className="flex gap-1">
                <Avatar pubKey={result.pubKey} />
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

export default SearchBox
