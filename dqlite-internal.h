#ifndef DQLITE_INTERNAL_H
#define DQLITE_INTERNAL_H

#include <stdbool.h>
#include <stddef.h>
#include <uv.h>

struct queue {
  struct queue *next;
  struct queue *prev;
};

typedef struct queue queue;

#define UV__PATH_SZ 1024
#define UV__FILENAME_LEN 128
#define UV__SEP_LEN 1
#define UV__DIR_LEN (UV__PATH_SZ - UV__SEP_LEN - UV__FILENAME_LEN - 1)
#define UV__MAX_SEGMENT_SIZE (8 * 1024 * 1024)

#define UV__CLOSED_TEMPLATE "%016llu-%016llu"
#define UV__OPEN_TEMPLATE "open-%llu"
#define UV__SNAPSHOT_TEMPLATE "snapshot-%llu-%llu-%llu"
#define UV__SNAPSHOT_META_SUFFIX ".meta"

#define UV__SEGMENT_FILENAME_BUF_SIZE 34

/* Template string for snapshot metadata filenames: snapshot term,  snapshot
 * index, creation timestamp (milliseconds since epoch). */
#define UV__SNAPSHOT_META_TEMPLATE                                             \
  UV__SNAPSHOT_TEMPLATE UV__SNAPSHOT_META_SUFFIX

#define RAFT_ERRMSG_BUF_SIZE 256
typedef unsigned long long raft_id, raft_term, raft_index, raft_time;
typedef unsigned long long uvCounter;
typedef void (*raft_io_tick_cb)(struct raft_io *io);
typedef void (*raft_io_recv_cb)(struct raft_io *io, struct raft_message *msg);
typedef void (*raft_io_close_cb)(struct raft_io *io);

/* Information persisted in a single metadata file. */
struct uvMetadata {
  unsigned long long version; /* Monotonically increasing version */
  raft_term term;             /* Current term */
  raft_id voted_for;          /* Server ID of last vote, or 0 */
};

/* Hold state of a libuv-based raft_io implementation. */
struct uv {
  struct raft_io *io;                  /* I/O object we're implementing */
  struct uv_loop_s *loop;              /* UV event loop */
  char dir[UV__DIR_LEN];               /* Data directory */
  struct raft_uv_transport *transport; /* Network transport */
  struct raft_tracer *tracer;          /* Debug tracing */
  raft_id id;                          /* Server ID */
  int state;                           /* Current state */
  bool snapshot_compression;           /* If compression is enabled */
  bool errored;                        /* If a disk I/O error was hit */
  bool direct_io;                      /* Whether direct I/O is supported */
  bool async_io;                       /* Whether async I/O is supported */
  bool fallocate;                      /* Whether fallocate is supported */
  size_t segment_size;                 /* Initial size of open segments. */
  size_t block_size;                   /* Block size of the data dir */
  queue clients;                       /* Outbound connections */
  queue servers;                       /* Inbound connections */
  unsigned connect_retry_delay;        /* Client connection retry delay */
  void *prepare_inflight;              /* Segment being prepared */
  queue prepare_reqs;                  /* Pending prepare requests. */
  queue prepare_pool;                  /* Prepared open segments */
  uvCounter prepare_next_counter;      /* Counter of next open segment */
  raft_index append_next_index;        /* Index of next entry to append */
  queue append_segments;               /* Open segments in use. */
  queue append_pending_reqs;           /* Pending append requests. */
  queue append_writing_reqs;           /* Append requests in flight */
  struct UvBarrier *barrier;           /* Inflight barrier request */
  queue finalize_reqs;                 /* Segments waiting to be closed */
  struct uv_work_s finalize_work;      /* Resize and rename segments */
  struct uv_work_s truncate_work;      /* Execute truncate log requests */
  queue snapshot_get_reqs;             /* Inflight get snapshot requests */
  queue async_work_reqs;               /* Inflight async work requests */
  struct uv_work_s snapshot_put_work;  /* Execute snapshot put requests */
  struct uvMetadata metadata;          /* Cache of metadata on disk */
  struct uv_timer_s timer;             /* Timer for periodic ticks */
  raft_io_tick_cb tick_cb;             /* Invoked when the timer expires */
  raft_io_recv_cb recv_cb;             /* Invoked when upon RPC messages */
  queue aborting;                      /* Cleanups upon errors or shutdown */
  bool closing;                        /* True if we are closing */
  raft_io_close_cb close_cb;           /* Invoked when finishing closing */
  bool auto_recovery; /* Try to recover from corrupt segments */
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

int UvList(struct uv *uv, struct uvSnapshotInfo *snapshots[],
           size_t *n_snapshots, struct uvSegmentInfo *segments[],
           size_t *n_segments, char *errmsg);

#endif /* DQLITE_INTERNAL_H */
