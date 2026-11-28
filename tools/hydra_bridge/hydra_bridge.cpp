// Hydra Bridge — ScriptHook VI (ABI 1.7) game-plane plugin for VIctory-Hydra.
//
// Registers a thread-safe C ABI callback (`HydraGameApi`) with `victory.exe`
// via `victory_hydra_register_game`, enqueues incoming JSON commands from the
// Hydra Named Pipe thread, and drains them strictly on RAGE 9's script thread
// inside `tick_register`.
//
// Threading & safety invariants:
// - Never calls any GTA 6 native off the script thread (`OnScriptTick`).
// - Never blocks the script thread waiting on IPC.
// - Wraps pool/native queries with `native_exists` checks and SEH guards.
// - Respects `api->is_blocked()` (fails closed if the online guard latches).

#include <shvi/shvi.h>
#include <shvi/shvi.hpp>
#include <shvi/natives_full.hpp>

#include <windows.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <mutex>
#include <queue>
#include <string>
#include <vector>

namespace {

constexpr uint32_t kHydraGameApiVersion = 1;

struct HydraGameApi {
    uint32_t struct_size;
    uint32_t version;
    int (*call)(const uint8_t* cmd_json,
                size_t cmd_len,
                uint8_t* out_buf,
                size_t out_cap,
                size_t* out_len,
                uint32_t timeout_ms);
};

using RegisterGameFn = int (*)(const HydraGameApi*);

struct PendingCmd {
    std::string json;
    std::string reply;
    bool done = false;
    std::mutex mu;
    std::condition_variable cv;
};

const shvi_api* g_api = nullptr;
std::mutex g_queue_mu;
std::queue<std::shared_ptr<PendingCmd>> g_queue;
std::atomic<uint64_t> g_ticks{0};
std::atomic<bool> g_ready{false};

uint32_t Joaat(const char* s) {
    uint32_t h = 0;
    for (; s && *s; ++s) {
        char c = *s;
        if (c >= 'A' && c <= 'Z') c = static_cast<char>(c - 'A' + 'a');
        else if (c == '\\') c = '/';
        h += static_cast<uint8_t>(c);
        h += h << 10;
        h ^= h >> 6;
    }
    h += h << 3;
    h ^= h >> 11;
    h += h << 15;
    return h;
}

std::string JsonEscape(const std::string& s) {
    std::string out;
    out.reserve(s.size() + 2);
    out.push_back('"');
    for (unsigned char c : s) {
        switch (c) {
            case '"': out += "\\\""; break;
            case '\\': out += "\\\\"; break;
            case '\n': out += "\\n"; break;
            case '\r': out += "\\r"; break;
            case '\t': out += "\\t"; break;
            default:
                if (c < 0x20) {
                    char buf[8];
                    std::snprintf(buf, sizeof(buf), "\\u%04x", c);
                    out += buf;
                } else {
                    out.push_back(static_cast<char>(c));
                }
        }
    }
    out.push_back('"');
    return out;
}

bool JsonGetString(const std::string& json, const char* key, std::string& out) {
    std::string needle = std::string("\"") + key + "\"";
    size_t pos = 0;
    while ((pos = json.find(needle, pos)) != std::string::npos) {
        pos += needle.size();
        while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
        if (pos >= json.size() || json[pos] != ':') continue;
        ++pos;
        while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
        if (pos >= json.size() || json[pos] != '"') return false;
        ++pos;
        out.clear();
        bool esc = false;
        for (; pos < json.size(); ++pos) {
            char c = json[pos];
            if (esc) {
                if (c == 'n') out.push_back('\n');
                else if (c == 'r') out.push_back('\r');
                else if (c == 't') out.push_back('\t');
                else out.push_back(c);
                esc = false;
            } else if (c == '\\') {
                esc = true;
            } else if (c == '"') {
                return true;
            } else {
                out.push_back(c);
            }
        }
        return false;
    }
    return false;
}

bool JsonGetNumber(const std::string& json, const char* key, double& out) {
    std::string needle = std::string("\"") + key + "\"";
    size_t pos = 0;
    while ((pos = json.find(needle, pos)) != std::string::npos) {
        pos += needle.size();
        while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
        if (pos >= json.size() || json[pos] != ':') continue;
        ++pos;
        while (pos < json.size() && (json[pos] == ' ' || json[pos] == '\t')) ++pos;
        if (pos >= json.size()) return false;
        char* end = nullptr;
        double v = std::strtod(json.c_str() + pos, &end);
        if (end && end != json.c_str() + pos) {
            out = v;
            return true;
        }
        return false;
    }
    return false;
}

bool HasNative(const char* name) {
    if (!g_api || !SHVI_API_HAS(g_api, native_exists)) return true;
    return g_api->native_exists(shvi_engine_hash(name)) != 0;
}

// Generic native invocation:
// {"cmd":"game.native","name":"SET_CLOCK_TIME","args":[{"t":"i","v":12},...]}
// or raw hex hash "0x..."
std::string HandleGenericNative(const std::string& json) {
    std::string name;
    if (!JsonGetString(json, "name", name) || name.empty()) {
        return R"({"ok":false,"err":"missing native name"})";
    }
    uint64_t hash = 0;
    if (name.size() > 2 && name[0] == '0' && (name[1] == 'x' || name[1] == 'X')) {
        hash = std::strtoull(name.c_str(), nullptr, 16);
    } else {
        hash = shvi_engine_hash(name.c_str());
    }
    if (SHVI_API_HAS(g_api, native_exists) && !g_api->native_exists(hash)) {
        return std::string(R"({"ok":false,"err":"native not bound in build","name":)") + JsonEscape(name) + "}";
    }

    // Parse compact args array: [{"t":"i","v":12},{"t":"f","v":1.5},{"t":"s","v":"abc"},...]
    std::vector<std::string> owned_strings;
    std::vector<uint64_t> slots;
    size_t args_pos = json.find("\"args\"");
    if (args_pos != std::string::npos) {
        size_t lb = json.find('[', args_pos);
        size_t rb = (lb != std::string::npos) ? json.find(']', lb) : std::string::npos;
        if (lb != std::string::npos && rb != std::string::npos && rb > lb) {
            size_t cur = lb;
            while (cur < rb) {
                size_t ob = json.find('{', cur);
                if (ob == std::string::npos || ob >= rb) break;
                size_t cb = json.find('}', ob);
                if (cb == std::string::npos || cb > rb) break;
                std::string elem = json.substr(ob, cb - ob + 1);
                std::string ty;
                if (JsonGetString(elem, "t", ty)) {
                    if (ty == "s") {
                        std::string sv;
                        JsonGetString(elem, "v", sv);
                        owned_strings.push_back(std::move(sv));
                        slots.push_back(0); // patched after vector stabilizes
                    } else if (ty == "f") {
                        double fv = 0.0;
                        JsonGetNumber(elem, "v", fv);
                        slots.push_back(shvi_f2u(static_cast<float>(fv)));
                    } else if (ty == "b") {
                        bool bv = elem.find("true") != std::string::npos || elem.find(":1") != std::string::npos;
                        slots.push_back(bv ? 1ULL : 0ULL);
                    } else if (ty == "u") {
                        double uv = 0.0;
                        JsonGetNumber(elem, "v", uv);
                        slots.push_back(static_cast<uint64_t>(uv));
                    } else {
                        double iv = 0.0;
                        JsonGetNumber(elem, "v", iv);
                        slots.push_back(static_cast<uint64_t>(static_cast<int64_t>(iv)));
                    }
                }
                cur = cb + 1;
            }
        }
    }

    // Re-pass to populate string pointers once `owned_strings` will not reallocate.
    if (!owned_strings.empty()) {
        size_t str_idx = 0;
        size_t slot_idx = 0;
        size_t args_p = json.find("\"args\"");
        size_t lb = json.find('[', args_p);
        size_t rb = json.find(']', lb);
        size_t cur = lb;
        while (cur < rb && slot_idx < slots.size()) {
            size_t ob = json.find('{', cur);
            if (ob == std::string::npos || ob >= rb) break;
            size_t cb = json.find('}', ob);
            if (cb == std::string::npos || cb > rb) break;
            std::string elem = json.substr(ob, cb - ob + 1);
            std::string ty;
            if (JsonGetString(elem, "t", ty)) {
                if (ty == "s" && str_idx < owned_strings.size()) {
                    slots[slot_idx] = shvi_p2u(owned_strings[str_idx].c_str());
                    ++str_idx;
                }
                ++slot_idx;
            }
            cur = cb + 1;
        }
    }

    g_api->native_init(hash);
    for (uint64_t v : slots) g_api->native_push64(v);
    uint64_t* ret = g_api->native_call();
    uint64_t r0 = ret ? ret[0] : 0;
    uint64_t r1 = ret ? ret[1] : 0;
    uint64_t r2 = ret ? ret[2] : 0;
    float f0 = shvi_u2f(r0);

    char buf[384];
    std::snprintf(
        buf, sizeof(buf),
        R"({"ok":true,"name":%s,"hash":"0x%016llx","argc":%zu,"r0_u64":%llu,"r0_i32":%d,"r0_f32":%.6f,"r1_u64":%llu,"r2_u64":%llu})",
        JsonEscape(name).c_str(),
        static_cast<unsigned long long>(hash),
        slots.size(),
        static_cast<unsigned long long>(r0),
        static_cast<int>(static_cast<int32_t>(r0)),
        f0,
        static_cast<unsigned long long>(r1),
        static_cast<unsigned long long>(r2));
    return buf;
}

std::string ExecuteOnScriptThread(const std::string& json) {
    using namespace shvi::nativedb::all;
    if (!g_api) return R"({"ok":false,"err":"shvi api not initialized"})";
    if (g_api->is_blocked && g_api->is_blocked()) {
        return R"({"ok":false,"err":"online guard latched"})";
    }

    std::string cmd;
    if (!JsonGetString(json, "cmd", cmd)) {
        return R"({"ok":false,"err":"missing cmd"})";
    }

    if (cmd == "game.ping") {
        char buf[256];
        std::snprintf(
            buf, sizeof(buf),
            R"({"ok":true,"game":%s,"build":%u,"ticks":%llu,"ready":%s})",
            JsonEscape(g_api->game_name ? g_api->game_name() : "unknown").c_str(),
            g_api->game_build ? g_api->game_build() : 0u,
            static_cast<unsigned long long>(g_ticks.load(std::memory_order_relaxed)),
            g_ready.load(std::memory_order_relaxed) ? "true" : "false");
        return buf;
    }

    if (cmd == "game.native") {
        return HandleGenericNative(json);
    }

    if (cmd == "game.player") {
        if (!HasNative("PLAYER_PED_ID") || !HasNative("GET_ENTITY_COORDS")) {
            return R"({"ok":false,"err":"player natives unavailable"})";
        }
        shvi_entity_ref ped = PLAYER_PED_ID();
        auto pos = GET_ENTITY_COORDS(ped, true);
        float heading = HasNative("GET_ENTITY_HEADING") ? GET_ENTITY_HEADING(ped) : 0.0f;
        int hp = HasNative("GET_ENTITY_HEALTH") ? GET_ENTITY_HEALTH(ped) : 0;
        int max_hp = HasNative("GET_ENTITY_MAX_HEALTH") ? GET_ENTITY_MAX_HEALTH(ped, false) : 0;
        void* ptr = (SHVI_API_HAS(g_api, entity_ptr)) ? g_api->entity_ptr(&ped) : nullptr;
        char buf[384];
        std::snprintf(
            buf, sizeof(buf),
            R"({"ok":true,"ped_lo":"0x%016llx","ped_hi":"0x%016llx","entity_ptr":"0x%llx","x":%.3f,"y":%.3f,"z":%.3f,"heading":%.2f,"health":%d,"max_health":%d})",
            static_cast<unsigned long long>(ped.lo),
            static_cast<unsigned long long>(ped.hi),
            static_cast<unsigned long long>(reinterpret_cast<uintptr_t>(ptr)),
            pos.x, pos.y, pos.z, heading, hp, max_hp);
        return buf;
    }

    if (cmd == "game.entities") {
        if (!SHVI_API_HAS(g_api, world_get_all)) {
            return R"({"ok":false,"err":"world_get_all unavailable"})";
        }
        int peds = g_api->world_get_all(SHVI_ENTITY_PED, nullptr, 0);
        int vehs = g_api->world_get_all(SHVI_ENTITY_VEHICLE, nullptr, 0);
        int objs = g_api->world_get_all(SHVI_ENTITY_OBJECT, nullptr, 0);
        char buf[192];
        std::snprintf(buf, sizeof(buf),
                      R"({"ok":true,"peds":%d,"vehicles":%d,"objects":%d})",
                      peds, vehs, objs);
        return buf;
    }

    if (cmd == "game.clock") {
        double h = 12, m = 0, s = 0, pause = 0;
        JsonGetNumber(json, "h", h);
        JsonGetNumber(json, "m", m);
        JsonGetNumber(json, "s", s);
        JsonGetNumber(json, "pause", pause);
        if (HasNative("SET_CLOCK_TIME")) {
            SET_CLOCK_TIME(static_cast<int>(h), static_cast<int>(m), static_cast<int>(s), false);
        }
        if (HasNative("PAUSE_CLOCK")) {
            PAUSE_CLOCK(pause != 0.0, false);
        }
        char buf[160];
        std::snprintf(buf, sizeof(buf),
                      R"({"ok":true,"hour":%d,"minute":%d,"second":%d,"paused":%s})",
                      static_cast<int>(h), static_cast<int>(m), static_cast<int>(s),
                      pause != 0.0 ? "true" : "false");
        return buf;
    }

    if (cmd == "game.weather") {
        std::string wname = "EXTRASUNNY";
        JsonGetString(json, "name", wname);
        uint32_t whash = Joaat(wname.c_str());
        if (HasNative("SET_WEATHER_TYPE")) {
            SET_WEATHER_TYPE(whash, true, true, false, 0.0f, false, false);
        }
        if (HasNative("_SET_WEATHER_TYPE_FROZEN")) {
            _SET_WEATHER_TYPE_FROZEN(true);
        }
        char buf[192];
        std::snprintf(buf, sizeof(buf),
                      R"({"ok":true,"weather":%s,"hash":"0x%08x"})",
                      JsonEscape(wname).c_str(), whash);
        return buf;
    }

    if (cmd == "game.teleport") {
        double x = 0, y = 0, z = 0;
        if (!JsonGetNumber(json, "x", x) || !JsonGetNumber(json, "y", y) || !JsonGetNumber(json, "z", z)) {
            return R"({"ok":false,"err":"missing x/y/z"})";
        }
        shvi_entity_ref ped = PLAYER_PED_ID();
        auto target = GET_ENTITY_COORDS(ped, true);
        target.x = static_cast<float>(x);
        target.y = static_cast<float>(y);
        target.z = static_cast<float>(z);
        SET_ENTITY_COORDS_NO_OFFSET(ped, target, false, false, false, 0.0f);
        char buf[160];
        std::snprintf(buf, sizeof(buf), R"({"ok":true,"x":%.3f,"y":%.3f,"z":%.3f})", x, y, z);
        return buf;
    }

    if (cmd == "game.god") {
        double on = 1;
        JsonGetNumber(json, "on", on);
        shvi_entity_ref ped = PLAYER_PED_ID();
        SET_ENTITY_INVINCIBLE(ped, on != 0.0);
        return on != 0.0 ? R"({"ok":true,"god":true})" : R"({"ok":true,"god":false})";
    }

    if (cmd == "game.freeze") {
        double on = 1;
        JsonGetNumber(json, "on", on);
        shvi_entity_ref ped = PLAYER_PED_ID();
        FREEZE_ENTITY_POSITION(ped, on != 0.0);
        return on != 0.0 ? R"({"ok":true,"frozen":true})" : R"({"ok":true,"frozen":false})";
    }

    if (cmd == "game.timescale") {
        double sc = 1.0;
        JsonGetNumber(json, "scale", sc);
        if (sc < 0.01) sc = 0.01;
        if (sc > 4.0) sc = 4.0;
        SET_TIME_SCALE(static_cast<float>(sc));
        char buf[96];
        std::snprintf(buf, sizeof(buf), R"({"ok":true,"scale":%.3f})", sc);
        return buf;
    }

    if (cmd == "game.bench_lock") {
        if (HasNative("SET_CLOCK_TIME")) SET_CLOCK_TIME(12, 0, 0, false);
        if (HasNative("PAUSE_CLOCK")) PAUSE_CLOCK(true, false);
        if (HasNative("SET_WEATHER_TYPE")) {
            SET_WEATHER_TYPE(Joaat("EXTRASUNNY"), true, true, false, 0.0f, false, false);
        }
        if (HasNative("_SET_WEATHER_TYPE_FROZEN")) _SET_WEATHER_TYPE_FROZEN(true);
        if (HasNative("PLAYER_PED_ID") && HasNative("FREEZE_ENTITY_POSITION")) {
            shvi_entity_ref ped = PLAYER_PED_ID();
            FREEZE_ENTITY_POSITION(ped, true);
            if (HasNative("SET_ENTITY_INVINCIBLE")) SET_ENTITY_INVINCIBLE(ped, true);
        }
        return R"({"ok":true,"bench_locked":true,"clock":"12:00:00","weather":"EXTRASUNNY","frozen":true})";
    }

    return std::string(R"({"ok":false,"err":"unknown game cmd","cmd":)") + JsonEscape(cmd) + "}";
}

void OnScriptTick(void*) {
    g_ready.store(true, std::memory_order_release);
    g_ticks.fetch_add(1, std::memory_order_relaxed);

    // Drain up to 8 commands per script tick so we never stall the frame.
    for (int i = 0; i < 8; ++i) {
        std::shared_ptr<PendingCmd> item;
        {
            std::lock_guard<std::mutex> lock(g_queue_mu);
            if (g_queue.empty()) break;
            item = std::move(g_queue.front());
            g_queue.pop();
        }
        std::string reply = ExecuteOnScriptThread(item->json);
        {
            std::lock_guard<std::mutex> lock(item->mu);
            item->reply = std::move(reply);
            item->done = true;
        }
        item->cv.notify_one();
    }
}

int BridgeCall(const uint8_t* cmd_json,
               size_t cmd_len,
               uint8_t* out_buf,
               size_t out_cap,
               size_t* out_len,
               uint32_t timeout_ms) {
    if (!cmd_json || cmd_len == 0 || !out_buf || out_cap == 0 || !out_len) {
        return -1;
    }
    *out_len = 0;
    if (!g_api) {
        const char* msg = R"({"ok":false,"err":"bridge not loaded"})";
        size_t n = std::strlen(msg);
        if (n > out_cap) n = out_cap;
        std::memcpy(out_buf, msg, n);
        *out_len = n;
        return -2;
    }

    auto item = std::make_shared<PendingCmd>();
    item->json.assign(reinterpret_cast<const char*>(cmd_json), cmd_len);
    {
        std::lock_guard<std::mutex> lock(g_queue_mu);
        if (g_queue.size() >= 64) {
            const char* msg = R"({"ok":false,"err":"script queue full"})";
            size_t n = std::strlen(msg);
            if (n > out_cap) n = out_cap;
            std::memcpy(out_buf, msg, n);
            *out_len = n;
            return -3;
        }
        g_queue.push(item);
    }

    std::unique_lock<std::mutex> lk(item->mu);
    bool completed = item->cv.wait_for(
        lk, std::chrono::milliseconds(timeout_ms ? timeout_ms : 2500),
        [&] { return item->done; });

    const std::string& resp =
        completed ? item->reply
                  : std::string(R"json({"ok":false,"err":"script thread timeout (game loading or paused)"})json");
    size_t n = resp.size();
    if (n > out_cap) n = out_cap;
    std::memcpy(out_buf, resp.data(), n);
    *out_len = n;
    return completed ? 0 : -4;
}

const HydraGameApi g_hydra_api = {
    sizeof(HydraGameApi),
    kHydraGameApiVersion,
    &BridgeCall,
};

} // namespace

extern "C" SHVI_PLUGIN_EXPORT int shvi_plugin_load(const shvi_api* api) {
    if (!shvi_api_compatible(api) || !SHVI_API_HAS(api, tick_register)) {
        return SHVI_E_ABI_MISMATCH;
    }
    g_api = api;
    ::shvi::api() = api;

    int rc = api->tick_register(SHVI_THIS_MODULE, &OnScriptTick, nullptr);
    if (rc != SHVI_OK) {
        if (api->log) api->log("hydra_bridge: tick_register failed");
        return rc;
    }

    HMODULE host = GetModuleHandleW(nullptr);
    auto reg = host
        ? reinterpret_cast<RegisterGameFn>(GetProcAddress(host, "victory_hydra_register_game"))
        : nullptr;
    if (reg) {
        int rrc = reg(&g_hydra_api);
        char msg[128];
        std::snprintf(msg, sizeof(msg), "hydra_bridge: registered with victory.exe (rc=%d)", rrc);
        if (api->log) api->log(msg);
    } else {
        if (api->log) api->log("hydra_bridge: victory_hydra_register_game export not found in host");
    }
    return SHVI_OK;
}

extern "C" SHVI_PLUGIN_EXPORT void shvi_plugin_unload(void) {
    if (g_api && g_api->module_unregister) {
        g_api->module_unregister(SHVI_THIS_MODULE);
    }
    g_api = nullptr;
}
