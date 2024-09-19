import { SerializedSocialGraph, SocialGraph } from "../../../src"
import throttle from "lodash/throttle"
import localForage from "localforage"

export const LOCALSTORAGE_PUBLICKEY = "iris.search.currentUser"
export const DEFAULT_SOCIAL_GRAPH_ROOT =
  "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0"
const LOCALFORAGE_KEY = "iris.socialGraph"

let publicKey = DEFAULT_SOCIAL_GRAPH_ROOT
try {
  const k = localStorage.getItem(LOCALSTORAGE_PUBLICKEY)
  if (k) {
    publicKey = k
  }
} catch (e) {
  //
}

let graph: SocialGraph

export const saveGraph = throttle(async () => {
  try {
    const data = graph.serialize()
    await localForage.setItem(LOCALFORAGE_KEY, JSON.stringify(data))
  } catch (e) {
    console.error("Error saving graph", e)
  }
}, 5000)

const initGraph = async () => {
  try {
    const data = await localForage.getItem(LOCALFORAGE_KEY)
    if (typeof data === 'string' && data) {
      graph = new SocialGraph(publicKey, JSON.parse(data) as SerializedSocialGraph)
    }
  } catch (e) {
    console.error('Error loading graph')
    localForage.removeItem(LOCALFORAGE_KEY)
  }
  if (!graph) {
    const { default: socialGraphData } = await import("../../../data/socialGraph.json")
    graph = new SocialGraph(publicKey, socialGraphData as SerializedSocialGraph)
  }
}

export const socialGraphLoaded = new Promise(async (resolve) => {
  await initGraph()
  resolve(true)
})

export default () => graph