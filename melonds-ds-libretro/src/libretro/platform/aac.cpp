/*
    Copyright 2026 Jesse Talavera

    melonDS DS is free software: you can redistribute it and/or modify it under
    the terms of the GNU General Public License as published by the Free
    Software Foundation, either version 3 of the License, or (at your option)
    any later version.
*/

#include <Platform.h>
#include <formats/raac.h>

#include <array>
#include <cstdint>
#include <iterator>

using namespace melonDS;

struct melonDS::Platform::AACDecoder
{
    raac_t* handle = nullptr;
    unsigned channels = 0;
    std::array<int16_t, RAAC_MAX_FRAME * 2> pcm {};
};

Platform::AACDecoder* Platform::AAC_Init()
{
    return new AACDecoder();
}

void Platform::AAC_DeInit(Platform::AACDecoder* dec)
{
    if (!dec) return;
    if (dec->handle) raac_close(dec->handle);
    delete dec;
}

bool Platform::AAC_Configure(Platform::AACDecoder* dec, int frequency, int channels)
{
    if (!dec || (channels != 1 && channels != 2)) return false;

    if (dec->handle)
    {
        raac_close(dec->handle);
        dec->handle = nullptr;
    }
    dec->channels = 0;

    constexpr int rates[] = {
        96000, 88200, 64000, 48000, 44100, 32000, 24000,
        22050, 16000, 12000, 11025, 8000, 7350
    };
    unsigned index = 0;
    while (index < std::size(rates) && rates[index] != frequency) ++index;
    if (index == std::size(rates)) return false;

    // AAC-LC AudioSpecificConfig, without an SBR extension. The DSi DSP
    // expects at most 1024 decoded samples per channel from each command.
    const uint8_t asc[2] = {
        static_cast<uint8_t>((2u << 3) | (index >> 1)),
        static_cast<uint8_t>(((index & 1u) << 7) | (channels << 3))
    };
    dec->handle = raac_open(asc, sizeof(asc));
    if (!dec->handle || raac_channels(dec->handle) != static_cast<unsigned>(channels))
    {
        if (dec->handle) raac_close(dec->handle);
        dec->handle = nullptr;
        return false;
    }
    dec->channels = static_cast<unsigned>(channels);
    return true;
}

bool Platform::AAC_DecodeFrame(Platform::AACDecoder* dec, const void* input,
                               int inputlen, void* output, int outputlen)
{
    if (!dec || !dec->handle || !input || !output || inputlen <= 0 ||
        outputlen < 1024 * 2 * static_cast<int>(sizeof(int16_t))) return false;

    const int frames = raac_decode_s16(dec->handle,
        static_cast<const uint8_t*>(input), static_cast<size_t>(inputlen),
        dec->pcm.data());
    if (frames < 1024 || frames > RAAC_MAX_FRAME) return false;

    // melonDS's AAC ucode reads 1024 interleaved stereo samples, even for
    // mono streams. Duplicate mono and reduce any doubled-rate output to the
    // fixed buffer size expected by that ucode.
    auto* result = static_cast<int16_t*>(output);
    for (int i = 0; i < 1024; ++i)
    {
        const int source = i * frames / 1024;
        result[i * 2] = dec->pcm[source * dec->channels];
        result[i * 2 + 1] = dec->pcm[source * dec->channels + (dec->channels - 1)];
    }
    return true;
}
