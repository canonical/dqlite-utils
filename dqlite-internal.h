#ifndef DQLITE_INTERNAL_H
#define DQLITE_INTERNAL_H

#include <stdbool.h>
#include <stddef.h>

#define UV__DISK_FORMAT 1
#define UV__FILENAME_LEN 128
#define UV__SEGMENT_FILENAME_BUF_SIZE 34

/* Template string for snapshot metadata filenames: snapshot term,  snapshot
 * index, creation timestamp (milliseconds since epoch). */
#define UV__SNAPSHOT_META_TEMPLATE                                             \
  UV__SNAPSHOT_TEMPLATE UV__SNAPSHOT_META_SUFFIX

typedef struct uv_buf_t {
  char *base;
  size_t len;
} uv_buf_t;

#define RAFT_ERRMSG_BUF_SIZE 256
typedef unsigned long long raft_id, raft_term, raft_index, raft_time;

enum raft_result_code {
  RAFT_OK = 0,
  RAFT_NOMEM,            /* Out of memory */
  RAFT_BADID,            /* Server ID is not valid */
  RAFT_DUPLICATEID,      /* Server ID already in use */
  RAFT_DUPLICATEADDRESS, /* Server address already in use */
  RAFT_BADROLE,          /* Server role is not valid */
  RAFT_MALFORMED,
  RAFT_NOTLEADER,
  RAFT_LEADERSHIPLOST,
  RAFT_SHUTDOWN,
  RAFT_CANTBOOTSTRAP,
  RAFT_CANTCHANGE,
  RAFT_CORRUPT,
  RAFT_CANCELED,
  RAFT_NAMETOOLONG,
  RAFT_TOOBIG,
  RAFT_NOCONNECTION,
  RAFT_BUSY,
  RAFT_IOERR,        /* File system or storage error */
  RAFT_NOTFOUND,     /* Resource not found */
  RAFT_INVALID,      /* Invalid parameter */
  RAFT_UNAUTHORIZED, /* No access to a resource */
  RAFT_NOSPACE,      /* Not enough space on disk */
  RAFT_TOOMANY,      /* Some system or raft limit was hit */
  RAFT_ERROR,        /* Generic error */
};

typedef int raft_result;
const char *raft_strerror(raft_result err);

void *raft_malloc(size_t size);
void raft_free(void *ptr);

struct raft_buffer {
  void *base; /* Pointer to the buffer data. */
  size_t len; /* Length of the buffer. */
};

enum {
  RAFT_COMMAND = 1, /* Command for the application FSM. */
  RAFT_BARRIER,     /* Wait for all previous commands to be applied. */
  RAFT_CHANGE       /* Raft configuration change. */
};

struct raft_entry {
  raft_term term;      /* Term in which the entry was created. */
  unsigned short type; /* Type (FSM command, barrier, config change). */
  bool is_local;       /* Placed here so it goes in the padding after @type. */
  struct raft_buffer buf; /* Entry data. */
  void *batch;            /* Batch that buf's memory points to, if any. */
};

/* Information persisted in a single metadata file. */
struct uvMetadata {
  unsigned long long version; /* Monotonically increasing version */
  raft_term term;             /* Current term */
  raft_id voted_for;          /* Server ID of last vote, or 0 */
};

raft_result uvMetadataLoad(const char *dir, struct uvMetadata *metadata,
                           char *errmsg);
raft_result uvMetadataStore(const char *dir, const struct uvMetadata *metadata,
                            char *errmsg);

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

struct uvSegmentBuffer {
  size_t block_size; /* Disk block size for direct I/O */
  uv_buf_t arena;    /* Previously allocated memory that can be re-used */
  size_t n;          /* Write offset */
};

void uvSegmentBufferInit(struct uvSegmentBuffer *b, size_t block_size);
raft_result uvSegmentBufferFormat(struct uvSegmentBuffer *b);
raft_result uvSegmentBufferAppend(struct uvSegmentBuffer *b,
                                  const struct raft_entry entries[],
                                  unsigned n_entries);
void uvSegmentBufferFinalize(struct uvSegmentBuffer *b, uv_buf_t *out);
raft_result uvSegmentBufferClose(struct uvSegmentBuffer *b);

struct uvSnapshotInfo {
  raft_term term;
  raft_index index;
  unsigned long long timestamp;
  char filename[UV__FILENAME_LEN];
};

enum raft_role {
  RAFT_STANDBY, /* Replicate log, does not participate in quorum. */
  RAFT_VOTER,   /* Replicate log, does participate in quorum. */
  RAFT_SPARE    /* Does not replicate log, or participate in quorum. */
};

struct raft_server {
  raft_id id;    /* Server ID, must be greater than zero. */
  char *address; /* Server address. User defined. */
  int role;      /* Server role. */
};

struct raft_configuration {
  struct raft_server *servers; /* Array of servers member of the cluster. */
  unsigned n;                  /* Number of servers in the array. */
};

void configurationInit(struct raft_configuration *c);
raft_result configurationAdd(struct raft_configuration *c, raft_id id,
                             const char *address, int role);
raft_result configurationEncode(const struct raft_configuration *c,
                                struct raft_buffer *buf);
void configurationClose(struct raft_configuration *c);

struct raft_snapshot {
  /* Index and term of last entry included in the snapshot. */
  raft_index index;
  raft_term term;

  /* Last committed configuration included in the snapshot, along with the
   * index it was committed at. */
  struct raft_configuration configuration;
  raft_index configuration_index;

  /* Content of the snapshot. When a snapshot is taken, the user FSM can
   * fill the bufs array with more than one buffer. When a snapshot is
   * restored, there will always be a single buffer. */
  struct raft_buffer *bufs;
  unsigned n_bufs;
};

void snapshotClose(struct raft_snapshot *s);

void formatSnapshotMetaHeader(void *header, raft_index index,
                              const struct raft_buffer *content);

raft_result uvSnapshotLoadMeta(const char *dir,
                               const struct uvSnapshotInfo *info,
                               struct raft_snapshot *snapshot, char *errmsg);

raft_result encodeSnapshotHeader(size_t n, struct raft_buffer *buf);

raft_result UvList(const char *dir, struct uvSnapshotInfo *snapshots[],
                   size_t *n_snapshots, struct uvSegmentInfo *segments[],
                   size_t *n_segments, char errmsg[RAFT_ERRMSG_BUF_SIZE]);

raft_result uvLoadEntriesBatch(const struct raft_buffer *content,
                               struct raft_entry **entries, unsigned *n_entries,
                               size_t *offset, /* Offset of last batch */
                               bool *last, char errmsg[RAFT_ERRMSG_BUF_SIZE]);

#endif /* DQLITE_INTERNAL_H */
