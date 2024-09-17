* Build a social graph from Nostr follow events
* Make queries for followed users, followers and follow distances
* Change social graph root user if needed, efficiently recalculating follow distances
* Low memory consumption
* Efficient serialization for saving & loading from storage
* Optional: pre-crawled Nostr dataset consisting of 260 follow lists and 23 000 users (2.2 MB)
* Used on [beta.iris.to](https://beta.iris.to)
* See [tests](./tests/SocialGraph.test.ts) for usage