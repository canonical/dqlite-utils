# Demo

A short walkthrough of a few `dqlite-utils` demo scenarios: leadership changes, behavior under load, and bootstrapping a new cluster from existing data.

## Table of contents

- [Summary](#demo)
- [Leadership changes](#leadership-changes)
- [Under load](#under-load)
- [Bootstrapping a new cluster](#bootstrapping-a-new-cluster)

## Initial setup:

- 9001 is the leader
- vegeta is attacking 9002, writes are being redirected to 9001
- `dqlite-utils` is watching 9003

## Leadership changes

1. Observe the state of each node with:

   ```bash
   watch "dqlite-utils -c '.status;.log'"
   ```

   Once this gets long enough, restart with `--compact` to show how we can have both detailed and overview outputs.

2. Bootstrap three nodes by calling the following thrice (different ports):

   ```bash
   ./dqlite-demo --dir "test-data" --api=127.0.0.1:8001 --db=127.0.0.1:9001 --join
   ```

3. Kill leader, observe log changes on each nodes.
   Note how the term increases.

## Under load

1. Set everything back up

2. Spam load:

   ```bash
   vegeta attack -targets=targets.txt -rate=20 | vegeta report
   ```

3. Observe how the raft index is increasing

4. Kill the leader

## Bootstrapping a new cluster

1. Open with `dqlite-utils`

   ```bash
   $ dqlite-utils --dir test-data/127.0.0.1:9001
   > .log                           # Show the latest data
   > .open
   open@latest> SELECT * FROM model;
   open@latest> .index 12345; # Index of interesting entry in the past
   open@12345> SELECT * FROM model;
   open@12345> VACUUM demo INTO './backup.sqlite';

   $ sqlite3 backup.sqlite
   > SELECT * FROM model; # Oh look, the data!

   $ dqlite-utils
   > .snapshot
   snapshot> .add-server 127.0.0.1:9009 # New server!
   snapshot> ATTACH 'backup.sqlite' AS demo; # NOTE SAME NAME!
   snapshot> INSERT INTO demo (key, value) VALUES ('new-key', 'new-value');
   snapshot> .finish 127.0.0.1:9009
   ```

2. Then, start a new node with id `127.0.0.1:9009` **no --join!**:
   - See how this is now a separate cluster!

3. Now, start the node again, new cluster, no `--join` (delete the dumbass `join` file if present)

// TODO(kcza): shell-expand `~`
