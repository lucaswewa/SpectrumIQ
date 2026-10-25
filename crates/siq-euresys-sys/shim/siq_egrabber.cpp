// The C ABI of siq_egrabber.h over eGrabber's C++ API (ADR-0004).
//
// Every entry point runs inside `guard`, which catches every exception and turns it into a
// status and a thread-local message: no exception unwinds across the ABI.

#include "siq_egrabber.h"

#include <EGrabber.h>

#include <algorithm>
#include <cstddef>
#include <cstring>
#include <memory>
#include <stdexcept>
#include <string>
#include <type_traits>
#include <vector>

using namespace Euresys;

// The layout the Rust bindings assume (src/bindings.rs, tests/layout.rs).
static_assert(sizeof(SiqEgCounts) == 12, "SiqEgCounts");
static_assert(sizeof(SiqEgCounters) == 48, "SiqEgCounters");
static_assert(sizeof(SiqBufferInfo) == 64, "SiqBufferInfo");
static_assert(offsetof(SiqBufferInfo, ptr) == 8 && offsetof(SiqBufferInfo, size) == 16 &&
                  offsetof(SiqBufferInfo, timestamp) == 24 &&
                  offsetof(SiqBufferInfo, frame_id) == 32 && offsetof(SiqBufferInfo, width) == 40 &&
                  offsetof(SiqBufferInfo, height) == 44 && offsetof(SiqBufferInfo, pixfmt) == 48 &&
                  offsetof(SiqBufferInfo, flags) == 56 &&
                  offsetof(SiqBufferInfo, pixfmt_namespace) == 60,
              "SiqBufferInfo's fields");

struct SiqEgSystem {
    std::unique_ptr<EGenTL> gentl;
    std::unique_ptr<EGrabberDiscovery> discovery;
};

struct SiqEgGrabber {
    explicit SiqEgGrabber(const EGrabberCameraInfo &camera) : grabber(camera) {}

    EGrabber<CallbackOnDemand> grabber;
    // The buffers announced, in eGrabber's indexes, one range each.
    std::vector<BufferIndexRange> announced;
    // Each buffer popped and not requeued, by our index: what pushing it back needs.
    std::vector<NewBufferData> popped;
    std::vector<bool> out;
    // Whether the producer has BUFFER_INFO_TIMESTAMP_NS, once known: -1 unknown, 0 no, 1 yes.
    int timestamp_ns = -1;
};

namespace {

thread_local std::string last_error;

// An argument the caller got wrong.
struct invalid_argument : std::invalid_argument {
    using std::invalid_argument::invalid_argument;
};

SiqEgStatus fail(SiqEgStatus status, const char *message) {
    try {
        last_error = message;
    } catch (...) {
        // Out of memory: the status alone will do.
    }
    return status;
}

template <typename F> SiqEgStatus guard(F &&body) {
    try {
        return body();
    } catch (const gentl_error &error) {
        switch (error.gc_err) {
        case gc::GC_ERR_TIMEOUT:
            return fail(SIQ_EG_TIMEOUT, error.what());
        case gc::GC_ERR_ABORT:
            return fail(SIQ_EG_ABORTED, error.what());
        default:
            return fail(SIQ_EG_GENTL_ERROR, error.what());
        }
    } catch (const invalid_argument &error) {
        return fail(SIQ_EG_INVALID_ARGUMENT, error.what());
    } catch (const std::exception &error) {
        return fail(SIQ_EG_ERROR, error.what());
    } catch (...) {
        return fail(SIQ_EG_ERROR, "an exception that isn't a std::exception");
    }
}

void require(bool ok, const char *what) {
    if (!ok) {
        throw invalid_argument(what);
    }
}

// Copies `text` out as a NUL-terminated string, truncated to `len`.
SiqEgStatus copy_out(const std::string &text, char *buf, size_t len, size_t *needed) {
    if (needed) {
        *needed = text.size() + 1;
    }
    if (buf && len > 0) {
        // In parentheses: windows.h defines a `min` macro.
        size_t n = (std::min)(text.size(), len - 1);
        std::memcpy(buf, text.data(), n);
        buf[n] = '\0';
    }
    return text.size() < len ? SIQ_EG_OK : SIQ_EG_TRUNCATED;
}

// Calls `body` with a null pointer to the module's tag type.
template <typename F> void on_module(SiqEgModule module, F &&body) {
    switch (module) {
    case SIQ_EG_INTERFACE:
        return body(static_cast<InterfaceModule *>(nullptr));
    case SIQ_EG_DEVICE:
        return body(static_cast<DeviceModule *>(nullptr));
    case SIQ_EG_STREAM:
        return body(static_cast<StreamModule *>(nullptr));
    case SIQ_EG_REMOTE:
        return body(static_cast<RemoteModule *>(nullptr));
    }
    throw invalid_argument("no such module");
}

template <typename M> using Module = std::remove_pointer_t<M>;

std::string line(const char *key, const std::string &value) {
    std::string clean = value;
    std::replace(clean.begin(), clean.end(), '\n', ' ');
    return std::string(key) + "=" + clean + "\n";
}

std::string describe(const EGrabberInfo &info) {
    return line("interface_index", std::to_string(info.interfaceIndex)) +
           line("device_index", std::to_string(info.deviceIndex)) +
           line("stream_index", std::to_string(info.streamIndex)) +
           line("interface_id", info.interfaceID) + line("device_id", info.deviceID) +
           line("stream_id", info.streamID) + line("vendor", info.deviceVendorName) +
           line("model", info.deviceModelName) + line("description", info.deviceDescription) +
           line("user_id", info.deviceUserID) + line("serial", info.deviceSerialNumber) +
           line("tl_type", info.tlType) + line("firmware_status", info.firmwareStatus) +
           line("license_status", info.licenseStatus) +
           line("remote_available", info.isRemoteAvailable ? "true" : "false");
}

EGrabberDiscovery &discovered(SiqEgSystem *system) {
    require(system != nullptr, "no system");
    require(system->discovery != nullptr, "nothing discovered yet");
    return *system->discovery;
}

// Discards the queues and revokes the announced buffers. The stream must be idle.
void revoke(SiqEgGrabber &g) {
    if (!g.announced.empty()) {
        g.grabber.flushBuffers(gc::ACQ_QUEUE_ALL_DISCARD);
    }
    while (!g.announced.empty()) {
        g.grabber.revoke(g.announced.back());
        g.announced.pop_back();
    }
    g.popped.clear();
    g.out.clear();
}

uint64_t timestamp_ns(SiqEgGrabber &g, Buffer &buffer) {
    if (g.timestamp_ns != 0) {
        try {
            uint64_t ns = buffer.getInfo<uint64_t>(g.grabber, gc::BUFFER_INFO_TIMESTAMP_NS);
            g.timestamp_ns = 1;
            return ns;
        } catch (const gentl_error &) {
            if (g.timestamp_ns == 1) {
                throw;
            }
            g.timestamp_ns = 0;
        }
    }
    // Euresys's producers give BUFFER_INFO_TIMESTAMP in µs.
    return buffer.getInfo<uint64_t>(g.grabber, gc::BUFFER_INFO_TIMESTAMP) * 1000;
}

void fill(SiqEgGrabber &g, Buffer &buffer, uint64_t index, SiqBufferInfo &info) {
    EGrabber<CallbackOnDemand> &e = g.grabber;
    info.index = index;
    info.ptr = buffer.getInfo<void *>(e, gc::BUFFER_INFO_BASE);
    info.size = buffer.getInfo<size_t>(e, gc::BUFFER_INFO_SIZE_FILLED);
    info.timestamp = timestamp_ns(g, buffer);
    info.frame_id = buffer.getInfo<uint64_t>(e, gc::BUFFER_INFO_FRAMEID);
    info.width = static_cast<uint32_t>(buffer.getInfo<size_t>(e, gc::BUFFER_INFO_WIDTH));
    info.height = static_cast<uint32_t>(buffer.getInfo<size_t>(e, gc::BUFFER_INFO_HEIGHT));
    info.pixfmt = buffer.getInfo<uint64_t>(e, gc::BUFFER_INFO_PIXELFORMAT);
    info.pixfmt_namespace = static_cast<uint32_t>(
        buffer.getInfo<uint64_t>(e, gc::BUFFER_INFO_PIXELFORMAT_NAMESPACE));
    bool incomplete = buffer.getInfo<bool8_t>(e, gc::BUFFER_INFO_IS_INCOMPLETE);
    info.flags = incomplete ? SIQ_EG_BUFFER_INCOMPLETE : 0u;
}

} // namespace

extern "C" {

uint32_t siq_eg_abi_version(void) { return SIQ_EG_ABI_VERSION; }

size_t siq_eg_last_error(char *buf, size_t len) {
    copy_out(last_error, buf, len, nullptr);
    return last_error.size() + 1;
}

SiqEgStatus siq_eg_system_open(const char *producer, SiqEgSystem **system) {
    return guard([&] {
        require(system != nullptr, "no place for the system");
        *system = nullptr;
        auto opened = std::make_unique<SiqEgSystem>();
        opened->gentl = producer ? std::make_unique<EGenTL>(std::string(producer))
                                 : std::make_unique<EGenTL>();
        *system = opened.release();
        return SIQ_EG_OK;
    });
}

void siq_eg_system_close(SiqEgSystem *system) {
    guard([&] {
        delete system;
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_discover(SiqEgSystem *system, SiqEgCounts *counts) {
    return guard([&] {
        require(system != nullptr && counts != nullptr, "no system or counts");
        system->discovery.reset();
        system->discovery = std::make_unique<EGrabberDiscovery>(*system->gentl);
        system->discovery->discover();
        counts->interfaces = system->discovery->interfaceCount();
        counts->grabbers = system->discovery->egrabberCount();
        counts->cameras = system->discovery->cameraCount();
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_discovery_info(SiqEgSystem *system, SiqEgFound found, int32_t index, char *buf,
                                  size_t len, size_t *needed) {
    return guard([&] {
        EGrabberDiscovery &discovery = discovered(system);
        std::string text;
        switch (found) {
        case SIQ_EG_FOUND_INTERFACE:
            require(index >= 0 && index < discovery.interfaceCount(), "no such interface");
            text = describe(discovery.interfaceInfo(index));
            break;
        case SIQ_EG_FOUND_GRABBER:
            require(index >= 0 && index < discovery.egrabberCount(), "no such grabber");
            text = describe(discovery.egrabbers(index));
            break;
        case SIQ_EG_FOUND_CAMERA: {
            require(index >= 0 && index < discovery.cameraCount(), "no such camera");
            EGrabberCameraInfo camera = discovery.cameras(index);
            text = line("grabbers", std::to_string(camera.grabbers.size()));
            if (!camera.grabbers.empty()) {
                text += describe(camera.grabbers.front());
            }
            break;
        }
        default:
            require(false, "no such kind of discovery");
        }
        return copy_out(text, buf, len, needed);
    });
}

SiqEgStatus siq_eg_open(SiqEgSystem *system, int32_t camera, SiqEgGrabber **grabber) {
    return guard([&] {
        require(grabber != nullptr, "no place for the grabber");
        *grabber = nullptr;
        EGrabberDiscovery &discovery = discovered(system);
        require(camera >= 0 && camera < discovery.cameraCount(), "no such camera");
        *grabber = new SiqEgGrabber(discovery.cameras(camera));
        return SIQ_EG_OK;
    });
}

void siq_eg_close(SiqEgGrabber *grabber) {
    if (!grabber) {
        return;
    }
    guard([&] {
        grabber->grabber.stop();
        return SIQ_EG_OK;
    });
    guard([&] {
        revoke(*grabber);
        return SIQ_EG_OK;
    });
    guard([&] {
        delete grabber;
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_get_int(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                           int64_t *value) {
    return guard([&] {
        require(grabber && feature && value, "no grabber, feature or value");
        on_module(module, [&](auto tag) {
            *value = grabber->grabber.getInteger<Module<decltype(tag)>>(feature);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_get_float(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                             double *value) {
    return guard([&] {
        require(grabber && feature && value, "no grabber, feature or value");
        on_module(module, [&](auto tag) {
            *value = grabber->grabber.getFloat<Module<decltype(tag)>>(feature);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_get_string(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                              char *buf, size_t len, size_t *needed) {
    return guard([&] {
        require(grabber && feature, "no grabber or feature");
        std::string value;
        on_module(module, [&](auto tag) {
            value = grabber->grabber.getString<Module<decltype(tag)>>(feature);
        });
        return copy_out(value, buf, len, needed);
    });
}

SiqEgStatus siq_eg_set_int(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                           int64_t value) {
    return guard([&] {
        require(grabber && feature, "no grabber or feature");
        on_module(module, [&](auto tag) {
            grabber->grabber.setInteger<Module<decltype(tag)>>(feature, value);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_set_float(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                             double value) {
    return guard([&] {
        require(grabber && feature, "no grabber or feature");
        on_module(module, [&](auto tag) {
            grabber->grabber.setFloat<Module<decltype(tag)>>(feature, value);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_set_string(SiqEgGrabber *grabber, SiqEgModule module, const char *feature,
                              const char *value) {
    return guard([&] {
        require(grabber && feature && value, "no grabber, feature or value");
        on_module(module, [&](auto tag) {
            grabber->grabber.setString<Module<decltype(tag)>>(feature, value);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_execute(SiqEgGrabber *grabber, SiqEgModule module, const char *command) {
    return guard([&] {
        require(grabber && command, "no grabber or command");
        on_module(module, [&](auto tag) {
            grabber->grabber.execute<Module<decltype(tag)>>(command);
        });
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_announce_user_buffers(SiqEgGrabber *grabber, void *const *ptrs, size_t size,
                                         size_t count) {
    return guard([&] {
        require(grabber && (ptrs || count == 0), "no grabber or buffers");
        SiqEgGrabber &g = *grabber;
        revoke(g);
        try {
            for (size_t i = 0; i < count; ++i) {
                require(ptrs[i] != nullptr, "a null buffer");
                // The user pointer carries our index, for siq_eg_pop.
                void *index = reinterpret_cast<void *>(static_cast<uintptr_t>(i));
                g.announced.push_back(g.grabber.announceAndQueue(UserMemory(ptrs[i], size, index)));
            }
        } catch (...) {
            revoke(g);
            throw;
        }
        g.popped.assign(count, NewBufferData());
        g.out.assign(count, false);
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_start(SiqEgGrabber *grabber, uint64_t frames) {
    return guard([&] {
        require(grabber != nullptr, "no grabber");
        grabber->grabber.start(frames == 0 ? GENTL_INFINITE : frames);
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_stop(SiqEgGrabber *grabber) {
    return guard([&] {
        require(grabber != nullptr, "no grabber");
        grabber->grabber.stop();
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_pop(SiqEgGrabber *grabber, uint64_t timeout_ms, SiqBufferInfo *info) {
    return guard([&] {
        require(grabber && info, "no grabber or info");
        SiqEgGrabber &g = *grabber;
        NewBufferData data = g.grabber.pop(timeout_ms);
        Buffer buffer(data);
        uint64_t index = reinterpret_cast<uintptr_t>(data.userPointer);
        try {
            require(index < g.popped.size(), "a buffer that wasn't announced here");
            fill(g, buffer, index, *info);
        } catch (...) {
            // Back to be filled, rather than lost to both sides.
            buffer.push(g.grabber);
            throw;
        }
        g.popped[index] = data;
        g.out[index] = true;
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_cancel_pop(SiqEgGrabber *grabber) {
    return guard([&] {
        require(grabber != nullptr, "no grabber");
        grabber->grabber.cancelPop();
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_requeue(SiqEgGrabber *grabber, uint64_t index) {
    return guard([&] {
        require(grabber != nullptr, "no grabber");
        SiqEgGrabber &g = *grabber;
        require(index < g.out.size() && g.out[index], "that buffer isn't popped");
        Buffer(g.popped[index]).push(g.grabber);
        g.out[index] = false;
        return SIQ_EG_OK;
    });
}

SiqEgStatus siq_eg_counters(SiqEgGrabber *grabber, SiqEgCounters *counters) {
    return guard([&] {
        require(grabber && counters, "no grabber or counters");
        EGrabber<CallbackOnDemand> &e = grabber->grabber;
        counters->announced = e.getInfo<StreamModule, size_t>(gc::STREAM_INFO_NUM_ANNOUNCED);
        counters->queued = e.getInfo<StreamModule, size_t>(gc::STREAM_INFO_NUM_QUEUED);
        counters->awaiting_delivery =
            e.getInfo<StreamModule, size_t>(gc::STREAM_INFO_NUM_AWAIT_DELIVERY);
        counters->delivered = e.getInfo<StreamModule, uint64_t>(gc::STREAM_INFO_NUM_DELIVERED);
        counters->underrun = e.getInfo<StreamModule, uint64_t>(gc::STREAM_INFO_NUM_UNDERRUN);
        counters->started = e.getInfo<StreamModule, uint64_t>(gc::STREAM_INFO_NUM_STARTED);
        return SIQ_EG_OK;
    });
}

} // extern "C"
