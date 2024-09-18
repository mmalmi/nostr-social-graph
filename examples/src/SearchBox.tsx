import React, {useEffect, useRef, useState, useCallback} from "react"
import {nip19} from "nostr-tools"
import Fuse from "fuse.js"
import fuseIndexData from "../../data/fuse_index.json"
import fuseData from "../../data/fuse_data.json"
import classNames from "classnames"
import {SearchIcon} from "./Icons"
import { Avatar } from "./Avatar"
import { debounce } from "lodash"
import FollowedBy from "./FollowedBy"

console.time('fuse init')
const fuseIndex = Fuse.parseIndex(fuseIndexData) // optional, you can just use fuseData if it's not super large
const fuse = new Fuse<SearchResult>(fuseData, { keys: ["name", "pubKey"] }, fuseIndex)
console.timeEnd('fuse init')
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

function SearchBox({onSelect}: {onSelect?: (string) => void }) {
  const [searchResults, setSearchResults] = useState([])
  const [activeResult, setActiveResult] = useState(0)
  const [value, setValue] = useState("")
  const inputRef = useRef(null)

  const debouncedSearch = useCallback(
    debounce((v) => {
      if (v.match(NOSTR_REGEX) || v.match(HEX_REGEX)) {
        setValue("")
        return
      }

      const results = fuse.search(v, { limit: MAX_RESULTS })
      console.log('results', results)
      setActiveResult(0)
      setSearchResults(results.map((result) => result.item))
    }, 200),
    []
  )

  useEffect(() => {
    const v = value.trim()
    if (v) {
      debouncedSearch(v)
    } else {
      setSearchResults([])
    }
  }, [value, debouncedSearch])

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
        onSelect?.(activeItem.pubKey)
        setValue("")
        setSearchResults([])
      }
    }
    window.addEventListener("keydown", handleKeyDown)
    return () => window.removeEventListener("keydown", handleKeyDown)
  }, [searchResults, activeResult])

  const handleSearchResultClick = (pubKey: string) => {
    setValue("")
    setSearchResults([])
  }

  const highlightMatch = (text: string, query: string) => {
    const parts = text.split(new RegExp(`(${query})`, 'gi'))
    return parts.map((part, index) =>
      part.toLowerCase() === query.toLowerCase() ? <b key={index}>{part}</b> : part
    )
  }

  return (
    <div className={"dropdown dropdown-open m-4"}>
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
              <div className="flex gap-2">
                <Avatar pubKey={result.pubKey} />
                <span>
                    {highlightMatch(result.name, value)}
                </span>
              </div>
              <FollowedBy pubkey={result.pubKey} />
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

export default SearchBox
