#ifndef DQLITE_INTERNAL_H
#define DQLITE_INTERNAL_H

#include <stdbool.h>
#include <stddef.h>

#define UV__PATH_SZ 1024
#define UV__FILENAME_LEN 128
#define UV__SEP_LEN 1
#define UV__DIR_LEN (UV__PATH_SZ - UV__SEP_LEN - UV__FILENAME_LEN - 1)
#define UV__MAX_SEGMENT_SIZE (8 * 1024 * 1024)

#define UV__SEGMENT_FILENAME_BUF_SIZE 34

/* Template string for snapshot metadata filenames: snapshot term,  snapshot
 * index, creation timestamp (milliseconds since epoch). */
#define UV__SNAPSHOT_META_TEMPLATE                                             \
  UV__SNAPSHOT_TEMPLATE UV__SNAPSHOT_META_SUFFIX

#define RAFT_ERRMSG_BUF_SIZE 256
typedef unsigned long long raft_id, raft_term, raft_index, raft_time;

/* Information persisted in a single metadata file. */
struct uvMetadata {
  unsigned long long version; /* Monotonically increasing version */
  raft_term term;             /* Current term */
  raft_id voted_for;          /* Server ID of last vote, or 0 */
};

/* Metadata about a segment file. */
struct uvSegmentInfo {
  bool is_open; /* Whether the segment is open */
  union {
    struct {
      raft_index first_index; /* First index in a closed segment */
      raft_index end_index;   /* Last index in a closed segment */
    } closed;
    struct {
      unsigned long long counter; /* Open segment counter */
    } open;
  } info;
  char filename[UV__SEGMENT_FILENAME_BUF_SIZE]; /* Segment filename */
};

struct uvSnapshotInfo {
  raft_term term;
  raft_index index;
  unsigned long long timestamp;
  char filename[UV__FILENAME_LEN];
};

int UvList(const char *dir, struct uvSnapshotInfo *snapshots[],
           size_t *n_snapshots, struct uvSegmentInfo *segments[],
           size_t *n_segments, char errmsg[RAFT_ERRMSG_BUF_SIZE]);

#endif /* DQLITE_INTERNAL_H */
