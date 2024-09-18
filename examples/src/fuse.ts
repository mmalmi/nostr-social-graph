import Fuse from "fuse.js"
import fuseIndexData from "../../data/fuse_index.json"
import fuseData from "../../data/fuse_data.json"

type SearchResult = {
    query?: string
    name: string
    pubKey: string
    nip05?: string
  }

console.time('fuse init')
const fuseIndex = Fuse.parseIndex(fuseIndexData) // optional, you can just use fuseData if it's not super large
const fuse = new Fuse<SearchResult>(fuseData, { keys: ["name", "pubKey"] }, fuseIndex)
console.timeEnd('fuse init')
console.log(fuse)

export default fuse