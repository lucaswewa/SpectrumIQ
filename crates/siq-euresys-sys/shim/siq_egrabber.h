/* SpectrumIQ's C ABI over Euresys eGrabber (ADR-0004).
 *
 * Plain C types and opaque handles only: the C++ behind it catches every exception and turns it
 * into a status and a message, which siq_eg_last_error gives back on the same thread.
 *
 * - A system loads one GenTL producer (a .cti) and discovers its grabbers and cameras.
 * - A grabber is a discovered camera, opened: its four modules' features, its buffers, and its
 *   acquisition.
 *
 * A grabber is used from one thread at a time; siq_eg_cancel_pop may be called from any thread.
 * Every grabber is closed before its system.
 */
#ifndef SIQ_EGRABBER_H
#define SIQ_EGRABBER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The version of this ABI: siq_eg_abi_version returns it. */
#define SIQ_EG_ABI_VERSION 1

/* What a call did. */
typedef enum SiqEgStatus {
    SIQ_EG_OK = 0,
    /* siq_eg_pop timed out: no buffer was filled in time. */
    SIQ_EG_TIMEOUT = 1,
    /* siq_eg_pop was cancelled by siq_eg_cancel_pop, or acquisition stopped. */
    SIQ_EG_ABORTED = 2,
    /* A string didn't fit: the buffer holds its start, and `needed` its full size. */
    SIQ_EG_TRUNCATED = 3,
    /* A null pointer, an index out of range, or a buffer not popped. */
    SIQ_EG_INVALID_ARGUMENT = 4,
    /* The GenTL producer reported an error: the message has its code. */
    SIQ_EG_GENTL_ERROR = 5,
    /* Any other failure. */
    SIQ_EG_ERROR = 6
} SiqEgStatus;

/* A grabber's GenTL modules, whose features are read and written by name. */
typedef enum SiqEgModule {
    /* The frame grabber card: I/O, PCIe and firmware. */
    SIQ_EG_INTERFACE = 0,
    /* The grabber's side of the camera link: triggers, strobes. */
    SIQ_EG_DEVICE = 1,
    /* The data stream: buffers. */
    SIQ_EG_STREAM = 2,
    /* The camera itself, through CoaXPress. */
    SIQ_EG_REMOTE = 3
} SiqEgModule;

/* What a discovery found. */
typedef enum SiqEgFound {
    /* The frame grabber cards. */
    SIQ_EG_FOUND_INTERFACE = 0,
    /* Each interface, device and stream that could grab. */
    SIQ_EG_FOUND_GRABBER = 1,
    /* The cameras: each one's grabbers, the master first. */
    SIQ_EG_FOUND_CAMERA = 2
} SiqEgFound;

/* The counts of a discovery. */
typedef struct SiqEgCounts {
    int32_t interfaces;
    int32_t grabbers;
    int32_t cameras;
} SiqEgCounts;

/* A filled buffer, from siq_eg_pop. It stays ours until siq_eg_requeue(index). */
typedef struct SiqBufferInfo {
    /* Its index in the order announced, from 0. */
    uint64_t index;
    /* Its memory, as announced. */
    void *ptr;
    /* The bytes filled. */
    uint64_t size;
    /* When it was filled, in ns of the grabber's clock. */
    uint64_t timestamp;
    /* The camera's frame number. */
    uint64_t frame_id;
    uint32_t width;
    uint32_t height;
    /* The pixel format's code, in `pixfmt_namespace`. */
    uint64_t pixfmt;
    /* SIQ_EG_BUFFER_* flags. */
    uint32_t flags;
    /* GenTL's PIXELFORMAT_NAMESPACE_ID: 4 is PFNC, 1000 and above the producer's own. */
    uint32_t pixfmt_namespace;
} SiqBufferInfo;

/* The buffer was filled, but something went wrong while filling it. */
#define SIQ_EG_BUFFER_INCOMPLETE 1u

/* The data stream's counts, since acquisition started (GenTL STREAM_INFO_NUM_*). */
typedef struct SiqEgCounters {
    /* Buffers announced. */
    uint64_t announced;
    /* Buffers waiting to be filled. */
    uint64_t queued;
    /* Buffers filled and not yet popped. */
    uint64_t awaiting_delivery;
    /* Buffers filled. */
    uint64_t delivered;
    /* Frames lost because no buffer was waiting to be filled. */
    uint64_t underrun;
    /* Buffers the acquisition engine started to fill. */
    uint64_t started;
} SiqEgCounters;

typedef struct SiqEgSystem SiqEgSystem;
typedef struct SiqEgGrabber SiqEgGrabber;

/* SIQ_EG_ABI_VERSION, as the shim was built. */
uint32_t siq_eg_abi_version(void);

/* The last failure's message on this thread, as a NUL-terminated string truncated to `len`.
 * Returns the message's full size, with its NUL: 1 when there was none. */
size_t siq_eg_last_error(char *buf, size_t len);

/* Loads the GenTL producer at `producer`, a .cti path, or the default one if it is null
 * (EURESYS_DEFAULT_GENTL_PRODUCER). */
SiqEgStatus siq_eg_system_open(const char *producer, SiqEgSystem **system);
void siq_eg_system_close(SiqEgSystem *system);

/* Discovers the interfaces, grabbers and cameras, replacing what an earlier discovery found. */
SiqEgStatus siq_eg_discover(SiqEgSystem *system, SiqEgCounts *counts);

/* What the last discovery found at `index` of `found`, as lines of `key=value`. Writes at most
 * `len` bytes with the NUL, and the full size to `needed` if it isn't null. */
SiqEgStatus siq_eg_discovery_info(SiqEgSystem *system, SiqEgFound found, int32_t index, char *buf,
                                  size_t len, size_t *needed);

/* Opens camera `camera` of the last discovery. */
SiqEgStatus siq_eg_open(SiqEgSystem *system, int32_t camera, SiqEgGrabber **grabber);
/* Stops acquisition, revokes the buffers, and closes the grabber. */
void siq_eg_close(SiqEgGrabber *grabber);

SiqEgStatus siq_eg_get_int(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                           int64_t *value);
SiqEgStatus siq_eg_get_float(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                             double *value);
/* As siq_eg_discovery_info, for the feature's value. */
SiqEgStatus siq_eg_get_string(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                              char *buf, size_t len, size_t *needed);
SiqEgStatus siq_eg_set_int(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                           int64_t value);
SiqEgStatus siq_eg_set_float(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                             double value);
SiqEgStatus siq_eg_set_string(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                              const char *value);
SiqEgStatus siq_eg_execute(SiqEgGrabber *grabber, SiqEgModule module, const char *command);

/* Announces `count` buffers of `size` bytes each, at `ptrs`, and queues them to be filled. Our
 * memory is filled by DMA: it must stay valid, and locked, until the grabber is closed. Revokes
 * any announced before. */
SiqEgStatus siq_eg_announce_user_buffers(SiqEgGrabber *grabber, void *const *ptrs, size_t size,
                                         size_t count);

/* Starts acquisition of `frames` frames, or until stopped if it is 0. */
SiqEgStatus siq_eg_start(SiqEgGrabber *grabber, uint64_t frames);
SiqEgStatus siq_eg_stop(SiqEgGrabber *grabber);

/* Waits up to `timeout_ms` for a filled buffer. */
SiqEgStatus siq_eg_pop(SiqEgGrabber *grabber, uint64_t timeout_ms, SiqBufferInfo *info);
/* Makes siq_eg_pop, waiting on another thread, return SIQ_EG_ABORTED. */
SiqEgStatus siq_eg_cancel_pop(SiqEgGrabber *grabber);
/* Queues popped buffer `index` to be filled again. */
SiqEgStatus siq_eg_requeue(SiqEgGrabber *grabber, uint64_t index);

SiqEgStatus siq_eg_counters(SiqEgGrabber *grabber, SiqEgCounters *counters);

#ifdef __cplusplus
}
#endif

#endif /* SIQ_EGRABBER_H */
