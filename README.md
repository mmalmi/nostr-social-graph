* Build a social graph from Nostr follow events
* Make queries for followed users, followers and follow distances
* Change social graph root user if needed, efficiently recalculating follow distances
* Low memory consumption
* Efficient serialization for saving & loading from storage
* Optional: pre-crawled Nostr [datasets](./data)
  * follows, consisting of 260 follow lists and 23 000 users (2.2 MB)
  * in [public/large_social_graph.json](./public/large_social_graph.json), 36.8 MB dataset of 161K users and 5.3M follows
  * profile names and pictures for 19 000 users (2.8 MB)
* Demo: [search.iris.to](https://search.iris.to) ([examples dir](./examples/))
* Also used on [beta.iris.to](https://beta.iris.to)
* See [tests](./tests/SocialGraph.test.ts) for usage
* The magic happens mostly in [SocialGraph.ts](./src/SocialGraph.ts)
