import { SerializedSocialGraph, SocialGraph } from "../../src"
import data from "../../data/socialGraph.json"

const DEFAULT_SOCIAL_GRAPH_ROOT =
  "4523be58d395b1b196a9b8c82b038b6895cb02b683d0c253a955068dba1facd0"

const socialGraph = new SocialGraph(DEFAULT_SOCIAL_GRAPH_ROOT, data as SerializedSocialGraph)

export default socialGraph