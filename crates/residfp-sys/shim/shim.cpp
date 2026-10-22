// C interface over reSIDfp::residfp for Rust (residfp-sys). GPL-2.0-or-later, like the library it wraps.
// No C++ exception may cross into Rust: every entry point catches everything and reports failure.

#include "residfp/residfp.h"

#include <new>

using reSIDfp::residfp;

extern "C" {

residfp* score64_residfp_new() {
    try {
        return new residfp();
    } catch (...) {
        return nullptr;
    }
}

void score64_residfp_free(residfp* sid) { delete sid; }

// model: 0 = MOS6581, 1 = CSG8580. Returns 1 on success.
int score64_residfp_set_chip_model(residfp* sid, int model) {
    try {
        return sid->setChipModel(model == 0 ? reSIDfp::MOS6581 : reSIDfp::CSG8580) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

// cws: 0 = AVERAGE, 1 = WEAK, 2 = STRONG. Returns 1 on success.
int score64_residfp_set_combined_waveforms(residfp* sid, int cws) {
    try {
        const reSIDfp::CombinedWaveforms v = cws == 1 ? reSIDfp::WEAK : cws == 2 ? reSIDfp::STRONG : reSIDfp::AVERAGE;
        return sid->setCombinedWaveforms(v) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

// method: 0 = DECIMATE, 1 = RESAMPLE. Returns 1 on success.
int score64_residfp_set_sampling(residfp* sid, double clock_hz, int method, double sample_rate) {
    try {
        return sid->setSamplingParameters(clock_hz, method == 0 ? reSIDfp::DECIMATE : reSIDfp::RESAMPLE, sample_rate)
            ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

void score64_residfp_set_filter_6581_curve(residfp* sid, double v) { sid->setFilter6581Curve(v); }
void score64_residfp_set_filter_6581_range(residfp* sid, double v) { sid->setFilter6581Range(v); }
void score64_residfp_set_filter_8580_curve(residfp* sid, double v) { sid->setFilter8580Curve(v); }
void score64_residfp_enable_filter(residfp* sid, int enable) { sid->enableFilter(enable != 0); }
void score64_residfp_reset(residfp* sid) { sid->reset(); }
void score64_residfp_write(residfp* sid, int reg, unsigned char value) { sid->write(reg, value); }
unsigned char score64_residfp_read(residfp* sid, int reg) { return sid->read(reg); }

// Runs `cycles` cycles and writes the samples produced to buf; returns how many. buf must hold at least `cycles`
// samples: SID::clock emits at most one per cycle, and slightly more than cycles * sample_rate / clock_hz.
int score64_residfp_clock(residfp* sid, unsigned int cycles, short* buf) { return sid->clock(cycles, buf); }

}  // extern "C"
