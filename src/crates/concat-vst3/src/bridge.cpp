// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

// C++ host bridge for VST3 audio-effect plugins.
//
// This file is compiled by concat-vst3's build.rs against the Steinberg VST3
// SDK headers (located via VST3_SDK_DIR). It exposes a flat C ABI that the
// Rust side calls through FFI (see src/bridge.rs).
//
// The bridge owns the VST3 C++ objects (IComponent, IEditController,
// IAudioProcessor). Rust holds an opaque handle and calls through this ABI.

#include "pluginterfaces/base/funknown.h"
#include "pluginterfaces/base/ibstream.h"
#include "pluginterfaces/base/ipluginbase.h"
#include "pluginterfaces/vst/ivstcomponent.h"
#include "pluginterfaces/vst/ivstaudioprocessor.h"
#include "pluginterfaces/vst/ivsteditcontroller.h"
#include "pluginterfaces/vst/vstspeaker.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <limits>
#include <mutex>
#include <new>
#include <string>
#include <unordered_map>
#include <vector>

#if defined(_WIN32)
#include <windows.h>
#else
#include <dlfcn.h>
#endif

using namespace Steinberg;
using namespace Steinberg::Vst;

using ModuleHandle = void*;
using PluginHandle = void*;

// C struct matching Rust's #[repr(C)] ClassId in bridge.rs.
struct ClassId {
    uint32_t data1;
    uint16_t data2;
    uint16_t data3;
    uint8_t data4[8];
};

// C struct matching Rust's #[repr(C)] ParameterInfo in bridge.rs. Named
// differently from Vst::ParameterInfo to keep both visible.
struct BridgeParameterInfo {
    int32_t id;
    char name[64];
    char units[64];
    double default_normalized;
    double step_size;
};

// C struct matching Rust's #[repr(C)] ProcessResult in bridge.rs.
struct ProcessResult {
    int32_t frames_processed;
    bool tail_pending;
};

// C struct matching Rust's #[repr(C)] ClassInfo in bridge.rs. The class ID is
// the raw 16-byte TUID used by the factory, so Rust can round-trip it without
// any host-endian interpretation losing bytes.
struct BridgeClassInfo {
    uint8_t class_id[16];
    char name[128];
    char vendor[128];
    char category[64];
    char sub_categories[128];
};

static bool result_ok(tresult result) {
    return result == kResultOk || result == kResultTrue;
}

// ---------------------------------------------------------------------------
// Minimal IComponentHandler implementation.
// The plugin calls back into this to report parameter edits and async state
// changes. Our host is parameter-driven (we set parameters from Rust), so the
// handler acknowledges everything.
// ---------------------------------------------------------------------------

class HostComponentHandler final : public IComponentHandler {
public:
    tresult PLUGIN_API beginEdit(ParamID /*id*/) override { return kResultOk; }
    tresult PLUGIN_API performEdit(ParamID /*id*/, ParamValue /*value*/) override {
        return kResultOk;
    }
    tresult PLUGIN_API endEdit(ParamID /*id*/) override { return kResultOk; }
    tresult PLUGIN_API restartComponent(int32 /*flags*/) override { return kResultOk; }

    // FUnknown
    tresult PLUGIN_API queryInterface(const TUID /*iid*/, void** obj) override {
        if (obj) *obj = nullptr;
        return kResultFalse;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }
};

static HostComponentHandler g_component_handler;

// ---------------------------------------------------------------------------
// Module loading
// ---------------------------------------------------------------------------

struct LoadedModule {
    void* handle = nullptr;
    int refs = 0;
};

static std::unordered_map<std::string, LoadedModule> g_loaded_modules;
static std::mutex g_loaded_modules_mutex;
#define BRIDGE_LOCK std::lock_guard<std::mutex> _bridge_lock(g_loaded_modules_mutex);

static void* load_module_library(const std::string& path) {
#if defined(_WIN32)
    return reinterpret_cast<void*>(::LoadLibraryA(path.c_str()));
#else
    return ::dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
#endif
}

static void unload_module_library(void* handle) {
#if defined(_WIN32)
    ::FreeLibrary(reinterpret_cast<HMODULE>(handle));
#else
    ::dlclose(handle);
#endif
}

static void* get_module_symbol(void* handle, const char* name) {
#if defined(_WIN32)
    return reinterpret_cast<void*>(
        ::GetProcAddress(reinterpret_cast<HMODULE>(handle), name));
#else
    return ::dlsym(handle, name);
#endif
}

static IPluginFactory* factory_for(void* handle) {
    auto get_factory = reinterpret_cast<IPluginFactory* (*)(void)>(
        get_module_symbol(handle, "GetPluginFactory"));
    return get_factory ? get_factory() : nullptr;
}

static std::vector<PClassInfo> audio_effect_classes(IPluginFactory* factory) {
    std::vector<PClassInfo> classes;
    const int32 count = factory->countClasses();
    for (int32 i = 0; i < count; ++i) {
        PClassInfo info{};
        if (factory->getClassInfo(i, &info) != kResultOk) continue;
        if (std::strcmp(info.category, kVstAudioEffectClass) == 0) {
            classes.push_back(info);
        }
    }
    return classes;
}

static void class_id_bytes(const ClassId& id, uint8_t out[16]) {
    out[0] = static_cast<uint8_t>(id.data1 & 0xFF);
    out[1] = static_cast<uint8_t>((id.data1 >> 8) & 0xFF);
    out[2] = static_cast<uint8_t>((id.data1 >> 16) & 0xFF);
    out[3] = static_cast<uint8_t>((id.data1 >> 24) & 0xFF);
    out[4] = static_cast<uint8_t>(id.data2 & 0xFF);
    out[5] = static_cast<uint8_t>((id.data2 >> 8) & 0xFF);
    out[6] = static_cast<uint8_t>(id.data3 & 0xFF);
    out[7] = static_cast<uint8_t>((id.data3 >> 8) & 0xFF);
    std::memcpy(out + 8, id.data4, 8);
}

// Copies a NUL-terminated char8 string into a fixed ABI buffer. VST3 metadata
// is allowed to fill the whole source array, so never rely on a terminator.
static void copy_ascii(char* dst, size_t capacity, const char* src) {
    if (!dst || capacity == 0) return;
    size_t i = 0;
    if (src) {
        while (i + 1 < capacity && src[i] != 0) {
            dst[i] = src[i];
            ++i;
        }
    }
    dst[i] = 0;
}

// ---------------------------------------------------------------------------
// UTF-16 (VST3 String128/TChar) to UTF-8 for the Rust-side metadata.
// ---------------------------------------------------------------------------

static void tchar_to_utf8(const TChar* src, char* dst, size_t capacity) {
    if (!dst || capacity == 0) return;
    size_t out = 0;
    for (size_t i = 0; src[i] != 0 && out + 1 < capacity; ++i) {
        uint32_t cp = static_cast<uint16_t>(src[i]);
        if (cp >= 0xD800 && cp <= 0xDBFF && src[i + 1] >= 0xDC00 &&
            src[i + 1] <= 0xDFFF) {
            cp = 0x10000 + ((cp - 0xD800) << 10) +
                 (static_cast<uint16_t>(src[i + 1]) - 0xDC00);
            ++i;
        }

        size_t needed = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
        if (out + needed >= capacity) break;

        if (needed == 1) {
            dst[out++] = static_cast<char>(cp);
        } else if (needed == 2) {
            dst[out++] = static_cast<char>(0xC0 | (cp >> 6));
            dst[out++] = static_cast<char>(0x80 | (cp & 0x3F));
        } else if (needed == 3) {
            dst[out++] = static_cast<char>(0xE0 | (cp >> 12));
            dst[out++] = static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            dst[out++] = static_cast<char>(0x80 | (cp & 0x3F));
        } else {
            dst[out++] = static_cast<char>(0xF0 | (cp >> 18));
            dst[out++] = static_cast<char>(0x80 | ((cp >> 12) & 0x3F));
            dst[out++] = static_cast<char>(0x80 | ((cp >> 6) & 0x3F));
            dst[out++] = static_cast<char>(0x80 | (cp & 0x3F));
        }
    }
    dst[out] = 0;
}

// Fills the flat ABI class metadata from the factory's best available view.
// PClassInfo2/3 metadata is preferred; PFactoryInfo supplies the vendor for
// factories (such as FabFilter) that leave the per-class vendor empty.
static void fill_class_metadata(IPluginFactory* factory, int32 index,
                                const PClassInfo& base, BridgeClassInfo* out) {
    std::memset(out, 0, sizeof(*out));
    std::memcpy(out->class_id, base.cid, sizeof(out->class_id));
    copy_ascii(out->name, sizeof(out->name), base.name);
    copy_ascii(out->category, sizeof(out->category), base.category);

    IPluginFactory3* factory3 = nullptr;
    if (result_ok(factory->queryInterface(
            IPluginFactory3_iid, reinterpret_cast<void**>(&factory3))) &&
        factory3) {
        PClassInfoW info{};
        if (result_ok(factory3->getClassInfoUnicode(index, &info))) {
            tchar_to_utf8(info.name, out->name, sizeof(out->name));
            tchar_to_utf8(info.vendor, out->vendor, sizeof(out->vendor));
            copy_ascii(out->category, sizeof(out->category), info.category);
        }
        factory3->release();
    }

    if (out->name[0] == 0) {
        copy_ascii(out->name, sizeof(out->name), base.name);
    }

    IPluginFactory2* factory2 = nullptr;
    if (result_ok(factory->queryInterface(
            IPluginFactory2_iid, reinterpret_cast<void**>(&factory2))) &&
        factory2) {
        PClassInfo2 info{};
        if (result_ok(factory2->getClassInfo2(index, &info))) {
            if (out->name[0] == 0) {
                copy_ascii(out->name, sizeof(out->name), info.name);
            }
            if (out->vendor[0] == 0) {
                copy_ascii(out->vendor, sizeof(out->vendor), info.vendor);
            }
            if (out->category[0] == 0) {
                copy_ascii(out->category, sizeof(out->category), info.category);
            }
            copy_ascii(out->sub_categories, sizeof(out->sub_categories), info.subCategories);
        }
        factory2->release();
    }

    if (out->vendor[0] == 0) {
        PFactoryInfo info{};
        if (result_ok(factory->getFactoryInfo(&info))) {
            copy_ascii(out->vendor, sizeof(out->vendor), info.vendor);
        }
    }

    if (out->vendor[0] == 0) {
        PFactoryInfo info{};
        if (result_ok(factory->getFactoryInfo(&info))) {
            copy_ascii(out->vendor, sizeof(out->vendor), info.vendor);
        }
    }

    if (out->name[0] == 0) {
        copy_ascii(out->name, sizeof(out->name), "Unknown");
    }
}

// ---------------------------------------------------------------------------
// IBStream backed by a byte vector, for getState/setState.
// ---------------------------------------------------------------------------

class MemoryStream final : public IBStream {
public:
    explicit MemoryStream(bool write) : writing_(write) {}

    MemoryStream(const void* data, size_t size) : writing_(false) {
        if (data && size > 0) {
            const auto* bytes = static_cast<const uint8_t*>(data);
            buffer_.assign(bytes, bytes + size);
        }
    }

    void rewind() { pos_ = 0; }

    tresult PLUGIN_API read(void* buffer, int32 numBytes, int32* numBytesRead) override {
        if (!buffer || numBytes < 0) return kResultFalse;
        const size_t available = pos_ < buffer_.size() ? buffer_.size() - pos_ : 0;
        const size_t count =
            static_cast<size_t>(numBytes) < available ? static_cast<size_t>(numBytes) : available;
        if (count > 0) {
            std::memcpy(buffer, buffer_.data() + pos_, count);
            pos_ += count;
        }
        if (numBytesRead) *numBytesRead = static_cast<int32>(count);
        return kResultOk;
    }

    tresult PLUGIN_API write(void* buffer, int32 numBytes, int32* numBytesWritten) override {
        if (!writing_ || numBytes < 0 || (!buffer && numBytes > 0)) return kResultFalse;
        const size_t count = static_cast<size_t>(numBytes);
        if (pos_ + count > buffer_.size()) buffer_.resize(pos_ + count);
        if (count > 0) {
            std::memcpy(buffer_.data() + pos_, buffer, count);
            pos_ += count;
        }
        if (numBytesWritten) *numBytesWritten = numBytes;
        return kResultOk;
    }

    tresult PLUGIN_API seek(int64 pos, int32 mode, int64* result) override {
        int64 target = pos;
        if (mode == IBStream::kIBSeekCur) {
            target = static_cast<int64>(pos_) + pos;
        } else if (mode == IBStream::kIBSeekEnd) {
            target = static_cast<int64>(buffer_.size()) + pos;
        }
        if (target < 0) target = 0;
        if (target > static_cast<int64>(buffer_.size())) {
            target = static_cast<int64>(buffer_.size());
        }
        pos_ = static_cast<size_t>(target);
        if (result) *result = static_cast<int64>(pos_);
        return kResultOk;
    }

    tresult PLUGIN_API tell(int64* pos) override {
        if (pos) *pos = static_cast<int64>(pos_);
        return kResultOk;
    }

    tresult PLUGIN_API queryInterface(const TUID /*iid*/, void** obj) override {
        if (obj) *obj = nullptr;
        return kResultFalse;
    }
    uint32 PLUGIN_API addRef() override { return 1; }
    uint32 PLUGIN_API release() override { return 1; }

    const std::vector<uint8_t>& bytes() const { return buffer_; }

private:
    bool writing_;
    size_t pos_ = 0;
    std::vector<uint8_t> buffer_;
};

// ---------------------------------------------------------------------------
// Opaque plugin instance state owned by the bridge.
// ---------------------------------------------------------------------------

struct PluginInstance {
    void* module_handle = nullptr;
    IPluginFactory* factory = nullptr;
    IComponent* component = nullptr;
    IEditController* controller = nullptr;
    IAudioProcessor* processor = nullptr;
    ClassId class_id{};
    bool controller_is_component = false;
    bool initialized = false;
    bool active = false;
    bool bypassed = false;
    bool tail_pending = false;
    int32_t num_channels = 2;
    double sample_rate = 48000.0;
    int32_t max_block_size = 1024;
    int32_t latency_samples = 0;
    double tail_seconds = 0.0;
};

// ---------------------------------------------------------------------------
// C ABI exports
// ---------------------------------------------------------------------------

extern "C" {

// vst3_module_load(path) -> ModuleHandle
ModuleHandle vst3_module_load(const char* path) {
    BRIDGE_LOCK
    if (!path) return nullptr;
    std::string p(path);
    auto it = g_loaded_modules.find(p);
    if (it != g_loaded_modules.end()) {
        ++it->second.refs;
        return reinterpret_cast<ModuleHandle>(it->second.handle);
    }
    void* handle = load_module_library(p);
    if (!handle) return nullptr;
    g_loaded_modules[p] = LoadedModule{handle, 1};
    return reinterpret_cast<ModuleHandle>(handle);
}

// vst3_module_unload(handle)
void vst3_module_unload(ModuleHandle handle) {
    BRIDGE_LOCK
    if (!handle) return;
    void* lib = reinterpret_cast<void*>(handle);
    for (auto it = g_loaded_modules.begin(); it != g_loaded_modules.end(); ++it) {
        if (it->second.handle != lib) continue;
        if (--it->second.refs > 0) return;
        g_loaded_modules.erase(it);
        unload_module_library(lib);
        return;
    }
    // Unknown handle: be conservative and do not unload an arbitrary library.
}

// vst3_plugin_create(module, class_id) -> PluginHandle
PluginHandle vst3_plugin_create(ModuleHandle handle, const ClassId* class_id) {
    BRIDGE_LOCK
    if (!handle || !class_id) return nullptr;

    void* lib = reinterpret_cast<void*>(handle);
    IPluginFactory* factory = factory_for(lib);
    if (!factory) return nullptr;

    uint8_t wanted[16];
    class_id_bytes(*class_id, wanted);

    for (const PClassInfo& info : audio_effect_classes(factory)) {
        if (std::memcmp(info.cid, wanted, 16) != 0) continue;

        FUnknown* component_unknown = nullptr;
        if (!result_ok(factory->createInstance(
                info.cid, IComponent_iid, reinterpret_cast<void**>(&component_unknown))) ||
            !component_unknown) {
            return nullptr;
        }
        auto* component = static_cast<IComponent*>(component_unknown);

        bool controller_is_component = false;
        IEditController* controller = nullptr;
        FUnknown* controller_unknown = nullptr;
        if (result_ok(component->queryInterface(
                IEditController_iid, reinterpret_cast<void**>(&controller_unknown))) &&
            controller_unknown) {
            controller = static_cast<IEditController*>(controller_unknown);
            controller_is_component = true;
        } else {
            // Many VST3 effects register their controller as a separate factory
            // class named identically to the audio component. Find it and
            // instantiate it rather than assuming the component is single-class.
            TUID controller_cid{};
            bool found_controller = false;
            const int32 factory_count = factory->countClasses();
            for (int32 i = 0; i < factory_count; ++i) {
                PClassInfo candidate{};
                if (!result_ok(factory->getClassInfo(i, &candidate))) continue;
                if (std::strcmp(candidate.category,
                                "Component Controller Class") != 0) {
                    continue;
                }
                if (std::strcmp(candidate.name, info.name) != 0) continue;
                std::memcpy(controller_cid, candidate.cid, sizeof(TUID));
                found_controller = true;
                break;
            }

            if (found_controller) {
                FUnknown* created = nullptr;
                if (result_ok(factory->createInstance(
                        controller_cid, IEditController_iid,
                        reinterpret_cast<void**>(&created))) &&
                    created) {
                    controller = static_cast<IEditController*>(created);
                }
            }
        }

        if (controller) controller->setComponentHandler(&g_component_handler);

        auto* instance = new (std::nothrow) PluginInstance();
        if (!instance) {
            if (controller) controller->release();
            component->release();
            return nullptr;
        }

        factory->addRef();
        instance->module_handle = handle;
        instance->factory = factory;
        instance->component = component;
        instance->controller = controller;
        instance->controller_is_component = controller_is_component;
        instance->class_id = *class_id;
        instance->num_channels = 2;
        return reinterpret_cast<PluginHandle>(instance);
    }

    return nullptr;
}

// vst3_plugin_destroy(plugin)
void vst3_plugin_destroy(PluginHandle handle) {
    BRIDGE_LOCK
    if (!handle) return;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);

    if (inst->processor) {
        if (inst->active) inst->processor->setProcessing(static_cast<TBool>(0));
        inst->processor->release();
        inst->processor = nullptr;
    }
    if (inst->component && inst->active) {
        inst->component->setActive(static_cast<TBool>(0));
    }
    if (inst->controller) {
        if (!inst->controller_is_component) inst->controller->terminate();
        inst->controller->release();
        inst->controller = nullptr;
    }
    if (inst->component) {
        inst->component->terminate();
        inst->component->release();
        inst->component = nullptr;
    }
    if (inst->factory) {
        inst->factory->release();
        inst->factory = nullptr;
    }

    delete inst;
}

// vst3_plugin_initialize(plugin, sample_rate, max_block_size, num_channels) -> bool
int32_t vst3_plugin_initialize(PluginHandle handle, double sample_rate,
                                int32_t max_block_size, int32_t num_channels) {
    BRIDGE_LOCK
    if (!handle) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->component || !inst->controller) return 0;
    if ((num_channels != 1 && num_channels != 2) || max_block_size < 1 ||
        sample_rate <= 0.0) {
        return 0;
    }

    inst->sample_rate = sample_rate;
    inst->max_block_size = max_block_size;
    inst->num_channels = num_channels;

    if (!result_ok(inst->component->initialize(nullptr))) return 0;
    if (!inst->controller_is_component &&
        !result_ok(inst->controller->initialize(nullptr))) {
        inst->component->terminate();
        return 0;
    }

    FUnknown* processor_unknown = nullptr;
    if (!result_ok(inst->component->queryInterface(
            IAudioProcessor_iid, reinterpret_cast<void**>(&processor_unknown))) ||
        !processor_unknown) {
        return 0;
    }
    inst->processor = static_cast<IAudioProcessor*>(processor_unknown);

    // Ask for the requested channel count on the main input and output bus.
    // setBusArrangements is an IAudioProcessor method, not an IComponent one.
    SpeakerArrangement arrangement =
        (num_channels == 1) ? SpeakerArr::kMono : SpeakerArr::kStereo;
    if (!result_ok(inst->processor->setBusArrangements(&arrangement, 1, &arrangement, 1))) {
        inst->processor->release();
        inst->processor = nullptr;
        return 0;
    }

    ProcessSetup setup{};
    setup.processMode = kRealtime;
    setup.symbolicSampleSize = kSample32;
    setup.maxSamplesPerBlock = max_block_size;
    setup.sampleRate = sample_rate;
    if (!result_ok(inst->processor->setupProcessing(setup))) {
        inst->processor->release();
        inst->processor = nullptr;
        return 0;
    }

    inst->latency_samples = inst->processor->getLatencySamples();
    const uint32 tail = inst->processor->getTailSamples();
    inst->tail_pending = tail != 0 || tail == kInfiniteTail;
    if (tail == kInfiniteTail) {
        inst->tail_seconds = 30.0;
    } else if (tail > 0) {
        inst->tail_seconds = std::min(static_cast<double>(tail) / sample_rate, 30.0);
    } else {
        inst->tail_seconds = 0.0;
    }
    inst->initialized = true;
    return 1;
}

// vst3_plugin_activate(plugin, active) -> bool
int32_t vst3_plugin_activate(PluginHandle handle, int32_t active) {
    BRIDGE_LOCK
    if (!handle) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->component || !inst->processor || !inst->initialized) return 0;

    if (active) {
        if (!result_ok(inst->component->setActive(static_cast<TBool>(1)))) return 0;
        if (!result_ok(inst->processor->setProcessing(static_cast<TBool>(1)))) {
            inst->component->setActive(static_cast<TBool>(0));
            return 0;
        }
        inst->active = true;
        inst->tail_pending = false;
    } else {
        inst->processor->setProcessing(static_cast<TBool>(0));
        inst->component->setActive(static_cast<TBool>(0));
        inst->active = false;
        inst->tail_pending = false;
    }
    return 1;
}

// vst3_plugin_get_latency(plugin) -> int32
int32_t vst3_plugin_get_latency(PluginHandle handle) {
    BRIDGE_LOCK
    if (!handle) return 0;
    return reinterpret_cast<PluginInstance*>(handle)->latency_samples;
}

// vst3_plugin_get_tail_seconds(plugin) -> double
double vst3_plugin_get_tail_seconds(PluginHandle handle) {
    BRIDGE_LOCK
    if (!handle) return 0.0;
    return reinterpret_cast<PluginInstance*>(handle)->tail_seconds;
}

// vst3_plugin_num_parameters(plugin) -> int32
int32_t vst3_plugin_num_parameters(PluginHandle handle) {
    BRIDGE_LOCK
    if (!handle) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->controller) return 0;
    return inst->controller->getParameterCount();
}

// vst3_plugin_get_parameter_info(plugin, index, out) -> bool
int32_t vst3_plugin_get_parameter_info(PluginHandle handle, int32_t index,
                                        BridgeParameterInfo* out) {
    BRIDGE_LOCK
    if (!handle || !out || index < 0) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->controller) return 0;

    Vst::ParameterInfo info{};
    if (inst->controller->getParameterInfo(index, info) != kResultOk) return 0;

    out->id = static_cast<int32_t>(info.id);
    tchar_to_utf8(info.title, out->name, sizeof(out->name));
    tchar_to_utf8(info.units, out->units, sizeof(out->units));
    out->default_normalized = info.defaultNormalizedValue;
    out->step_size = info.stepCount > 0 ? 1.0 / static_cast<double>(info.stepCount) : 0.0;
    return 1;
}

// vst3_plugin_get_parameter(plugin, param_id) -> double
double vst3_plugin_get_parameter(PluginHandle handle, int32_t param_id) {
    BRIDGE_LOCK
    if (!handle) return 0.0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->controller) return 0.0;
    return inst->controller->getParamNormalized(static_cast<ParamID>(param_id));
}

// vst3_plugin_set_parameter(plugin, param_id, value)
void vst3_plugin_set_parameter(PluginHandle handle, int32_t param_id, double value) {
    BRIDGE_LOCK
    if (!handle) return;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->controller) return;
    inst->controller->setParamNormalized(static_cast<ParamID>(param_id), value);
}

// vst3_plugin_get_state(plugin, out_data, out_size) -> bool
int32_t vst3_plugin_get_state(PluginHandle handle, void** out_data, int32_t* out_size) {
    BRIDGE_LOCK
    if (!handle || !out_data || !out_size) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->component) return 0;

    MemoryStream component_stream(true);
    if (!result_ok(inst->component->getState(&component_stream))) return 0;

    MemoryStream controller_stream(true);
    const bool has_controller_state =
        inst->controller && !inst->controller_is_component &&
        result_ok(inst->controller->getState(&controller_stream));
    const auto& component = component_stream.bytes();
    const auto& controller = controller_stream.bytes();
    constexpr uint8_t state_magic[] = {'C', 'V', 'S', 'T', '3', 'S', '0', '1'};
    if (component.size() > std::numeric_limits<uint32_t>::max() ||
        (has_controller_state && controller.size() > std::numeric_limits<uint32_t>::max())) {
        return 0;
    }

    std::vector<uint8_t> state;
    state.insert(state.end(), state_magic, state_magic + sizeof(state_magic));
    const uint32_t component_size = static_cast<uint32_t>(component.size());
    const uint32_t controller_size =
        has_controller_state ? static_cast<uint32_t>(controller.size()) : 0;
    for (unsigned shift = 0; shift < 32; shift += 8) {
        state.push_back(static_cast<uint8_t>(component_size >> shift));
    }
    for (unsigned shift = 0; shift < 32; shift += 8) {
        state.push_back(static_cast<uint8_t>(controller_size >> shift));
    }
    state.insert(state.end(), component.begin(), component.end());
    if (has_controller_state) {
        state.insert(state.end(), controller.begin(), controller.end());
    }
    const size_t size = state.size();
    if (size > static_cast<size_t>(std::numeric_limits<int32_t>::max())) return 0;
    void* data = std::malloc(size > 0 ? size : 1);
    if (!data) return 0;
    if (size > 0) std::memcpy(data, state.data(), size);

    *out_data = data;
    *out_size = static_cast<int32_t>(size);
    return 1;
}

// vst3_plugin_set_state(plugin, data, size) -> bool
int32_t vst3_plugin_set_state(PluginHandle handle, const void* data, int32_t size) {
    BRIDGE_LOCK
    if (!handle || !data || size < 0) return 0;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->component) return 0;

    const auto* bytes = static_cast<const uint8_t*>(data);
    constexpr uint8_t state_magic[] = {'C', 'V', 'S', 'T', '3', 'S', '0', '1'};
    constexpr size_t header_size = sizeof(state_magic) + 2 * sizeof(uint32_t);
    const bool framed = size >= static_cast<int32_t>(header_size) &&
                        std::memcmp(bytes, state_magic, sizeof(state_magic)) == 0;
    uint32_t component_size = static_cast<uint32_t>(size);
    uint32_t controller_size = 0;
    const uint8_t* component_data = bytes;
    const uint8_t* controller_data = nullptr;
    if (framed) {
        const auto read_u32 = [bytes](size_t offset) {
            return static_cast<uint32_t>(bytes[offset]) |
                   (static_cast<uint32_t>(bytes[offset + 1]) << 8) |
                   (static_cast<uint32_t>(bytes[offset + 2]) << 16) |
                   (static_cast<uint32_t>(bytes[offset + 3]) << 24);
        };
        component_size = read_u32(sizeof(state_magic));
        controller_size = read_u32(sizeof(state_magic) + sizeof(uint32_t));
        const uint64_t payload_size =
            static_cast<uint64_t>(component_size) + static_cast<uint64_t>(controller_size);
        if (payload_size != static_cast<uint64_t>(size) - header_size) return 0;
        component_data = bytes + header_size;
        controller_data = component_data + component_size;
    }

    MemoryStream component_stream(component_data, component_size);
    if (!result_ok(inst->component->setState(&component_stream))) return 0;

    if (inst->controller && !inst->controller_is_component) {
        component_stream.rewind();
        if (!result_ok(inst->controller->setComponentState(&component_stream))) return 0;
        if (framed && controller_size > 0) {
            MemoryStream controller_stream(controller_data, controller_size);
            if (!result_ok(inst->controller->setState(&controller_stream))) return 0;
        }
    }
    return 1;
}

// vst3_plugin_state_free(data)
void vst3_plugin_state_free(void* data) {
    BRIDGE_LOCK
    std::free(data);
}

// vst3_plugin_process(plugin, inputs, outputs, num_channels, num_frames) -> ProcessResult
ProcessResult vst3_plugin_process(PluginHandle handle, const float* const* inputs,
                                   float** outputs, int32_t num_channels,
                                   int32_t num_frames) {
    BRIDGE_LOCK
    ProcessResult result{0, false};
    if (!handle || !inputs || !outputs || num_channels <= 0 || num_frames <= 0) {
        return result;
    }
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    if (!inst->processor || !inst->active) {
        result.frames_processed = num_frames;
        return result;
    }
    if ((num_channels != 1 && num_channels != 2) ||
        num_channels != inst->num_channels ||
        num_frames > inst->max_block_size) {
        return result;
    }

    if (inst->bypassed) {
        for (int32_t ch = 0; ch < num_channels; ++ch) {
            if (inputs[ch] && outputs[ch]) {
                std::memmove(outputs[ch], inputs[ch],
                             static_cast<size_t>(num_frames) * sizeof(float));
            }
        }
        result.frames_processed = num_frames;
        result.tail_pending = inst->tail_pending;
        return result;
    }

    std::vector<Sample32*> input_channels(static_cast<size_t>(num_channels));
    std::vector<Sample32*> output_channels(static_cast<size_t>(num_channels));
    for (int32_t ch = 0; ch < num_channels; ++ch) {
        input_channels[static_cast<size_t>(ch)] =
            const_cast<Sample32*>(inputs[ch]);
        output_channels[static_cast<size_t>(ch)] = outputs[ch];
    }

    AudioBusBuffers input_bus{};
    input_bus.numChannels = num_channels;
    input_bus.channelBuffers32 = input_channels.data();

    AudioBusBuffers output_bus{};
    output_bus.numChannels = num_channels;
    output_bus.channelBuffers32 = output_channels.data();

    ProcessData data{};
    data.processMode = kRealtime;
    data.symbolicSampleSize = kSample32;
    data.numSamples = num_frames;
    data.inputs = &input_bus;
    data.numInputs = 1;
    data.outputs = &output_bus;
    data.numOutputs = 1;
    data.processContext = nullptr;

    if (result_ok(inst->processor->process(data))) {
        result.frames_processed = num_frames;
        const uint32 tail = inst->processor->getTailSamples();
        inst->tail_pending = tail == kInfiniteTail || tail > 0;
        if (tail == kInfiniteTail) {
            inst->tail_seconds = 30.0;
        } else if (tail > 0) {
            inst->tail_seconds = std::min(static_cast<double>(tail) / inst->sample_rate, 30.0);
        } else {
            inst->tail_seconds = 0.0;
        }
        result.tail_pending = inst->tail_pending;
    }
    return result;
}

// vst3_plugin_set_bypass(plugin, bypass)
void vst3_plugin_set_bypass(PluginHandle handle, int32_t bypass) {
    BRIDGE_LOCK
    if (!handle) return;
    auto* inst = reinterpret_cast<PluginInstance*>(handle);
    inst->bypassed = bypass != 0;
    if (inst->bypassed) {
        inst->tail_pending = false;
    }
}

// vst3_module_num_classes(module) -> int32
int32_t vst3_module_num_classes(ModuleHandle handle) {
    BRIDGE_LOCK
    if (!handle) return 0;
    IPluginFactory* factory =
        factory_for(reinterpret_cast<void*>(handle));
    if (!factory) return 0;
    return static_cast<int32_t>(audio_effect_classes(factory).size());
}

// vst3_module_get_class_info(module, index, out) -> bool
// `index` addresses the filtered audio-effect list, not the raw factory list.
int32_t vst3_module_get_class_info(ModuleHandle handle, int32_t index,
                                    BridgeClassInfo* out) {
    BRIDGE_LOCK
    if (!handle || !out || index < 0) return 0;
    IPluginFactory* factory =
        factory_for(reinterpret_cast<void*>(handle));
    if (!factory) return 0;

    int32_t effect_index = 0;
    const int32 count = factory->countClasses();
    for (int32_t i = 0; i < count; ++i) {
        PClassInfo base{};
        if (!result_ok(factory->getClassInfo(i, &base))) continue;
        if (std::strcmp(base.category, kVstAudioEffectClass) != 0) continue;
        if (effect_index != index) {
            ++effect_index;
            continue;
        }
        fill_class_metadata(factory, i, base, out);
        return 1;
    }
    return 0;
}

// vst3_module_get_class_id(module, index) -> char*
// Returns a newly allocated hex-encoded class ID string (caller frees).
char* vst3_module_get_class_id(ModuleHandle handle, int32_t index) {
    BRIDGE_LOCK
    BridgeClassInfo info{};
    if (!vst3_module_get_class_info(handle, index, &info)) return nullptr;

    auto* text = static_cast<char*>(std::malloc(33));
    if (!text) return nullptr;
    for (int i = 0; i < 16; ++i) {
        std::snprintf(text + i * 2, 3, "%02x",
                      static_cast<unsigned char>(info.class_id[i]));
    }
    text[32] = 0;
    return text;
}

} // extern "C"
